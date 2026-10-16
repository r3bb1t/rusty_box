//! [`HttpClient`] over `ureq` 2 with rustls. Microsoft's symbol server
//! answers a found file with a 302 to signed blob storage, which the agent
//! follows; only the URL asked for is ever reported, never the signed one.
//! The connect and per-read timeouts bound a stalled transfer without
//! bounding a slow large one, and a cancel takes effect between reads.

use std::io::Read;
use std::time::Duration;

use crate::http::{Flow, HttpClient, HttpError, HttpResponse, Progress, Retry};

/// Microsoft's symbol server and symsrv clients identify themselves so.
pub const SYMBOL_SERVER_USER_AGENT: &str = "Microsoft-Symbol-Server/10.0.0.0";
const CHUNK_BYTES: usize = 64 * 1024;

/// One agent: clones share its connection pool, so each worker thread can
/// hold its own `UreqHttp` over one pool.
#[derive(Clone)]
pub struct UreqHttp {
    agent: ureq::Agent,
}

impl UreqHttp {
    #[must_use]
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .redirects(8)
            .user_agent(SYMBOL_SERVER_USER_AGENT)
            // HTTPS_PROXY / HTTP_PROXY / ALL_PROXY, as other symbol clients
            // honour them.
            .try_proxy_from_env(true)
            .build();
        UreqHttp { agent }
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a transport failure of `kind` may pass on a retry.
fn transport_retry(kind: ureq::ErrorKind) -> Retry {
    match kind {
        ureq::ErrorKind::InvalidUrl
        | ureq::ErrorKind::UnknownScheme
        | ureq::ErrorKind::InsecureRequestHttpsOnly
        | ureq::ErrorKind::InvalidProxyUrl
        | ureq::ErrorKind::ProxyUnauthorized => Retry::Final,
        ureq::ErrorKind::Dns
        | ureq::ErrorKind::ConnectionFailed
        | ureq::ErrorKind::TooManyRedirects
        | ureq::ErrorKind::BadStatus
        | ureq::ErrorKind::BadHeader
        | ureq::ErrorKind::Io
        | ureq::ErrorKind::ProxyConnect
        | ureq::ErrorKind::HTTP => Retry::Retryable,
    }
}

impl HttpClient for UreqHttp {
    fn get<P>(&mut self, url: &str, cap: u64, mut progress: P) -> Result<HttpResponse, HttpError>
    where
        P: FnMut(Progress) -> Flow,
    {
        let response = match self.agent.get(url).call() {
            Ok(response) => response,
            // A status of 400 or more still carries a readable response.
            Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(transport)) => {
                return Err(HttpError::Transport {
                    url: url.to_owned(),
                    message: transport.to_string(),
                    retry: transport_retry(transport.kind()),
                })
            }
        };
        let status = response.status();
        // Only a hint: a compressed transfer declares its compressed size.
        let declared: Option<u64> = response.header("Content-Length").and_then(|value| value.trim().parse().ok());
        if declared.is_some_and(|len| len > cap) {
            return Err(HttpError::TooLarge { url: url.to_owned(), cap, declared });
        }
        let mut reader = response.into_reader().take(cap.saturating_add(1));
        let mut body = Vec::new();
        let mut chunk = [0u8; CHUNK_BYTES];
        loop {
            let read = reader
                .read(&mut chunk)
                .map_err(|error| HttpError::Body { url: url.to_owned(), message: error.to_string() })?;
            if read == 0 {
                break;
            }
            let bytes = chunk.get(..read).unwrap_or(&[]);
            body.try_reserve(bytes.len())
                .map_err(|_| HttpError::Body { url: url.to_owned(), message: "no memory for the body".to_owned() })?;
            body.extend_from_slice(bytes);
            let done = u64::try_from(body.len()).unwrap_or(u64::MAX);
            if done > cap {
                return Err(HttpError::TooLarge { url: url.to_owned(), cap, declared });
            }
            match progress(Progress { done, total: declared }) {
                Flow::Continue => {}
                Flow::Cancel => return Err(HttpError::Cancelled { url: url.to_owned() }),
            }
        }
        Ok(HttpResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    /// A server on 127.0.0.1 that answers `requests` requests, each by its
    /// path, then stops; returns its base URL.
    fn serve(requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let redirect_to = format!("{base}/file");
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let mut stream = stream.unwrap();
                let mut request_line = String::new();
                BufReader::new(&stream).read_line(&mut request_line).unwrap();
                let path = request_line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let answer: Vec<u8> = match path.as_str() {
                    "/file" => b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec(),
                    "/moved" => format!(
                        "HTTP/1.1 302 Found\r\nLocation: {redirect_to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .into_bytes(),
                    "/large" => {
                        let mut answer = b"HTTP/1.1 200 OK\r\nContent-Length: 200000\r\nConnection: close\r\n\r\n".to_vec();
                        answer.resize(answer.len() + 200_000, b'x');
                        answer
                    }
                    _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot found".to_vec(),
                };
                // The client may close early (a cap, a cancel); the server
                // goes on to the next request either way.
                if stream.write_all(&answer).is_err() {
                    continue;
                }
            }
        });
        base
    }

    #[test]
    fn a_redirect_is_followed_to_the_file() {
        let base = serve(2);
        let response = UreqHttp::new().get(&format!("{base}/moved"), 1 << 20, |_| Flow::Continue).unwrap();
        assert_eq!(response, HttpResponse { status: 200, body: b"hello".to_vec() });
    }

    #[test]
    fn a_missing_file_is_a_404_response_not_an_error() {
        let base = serve(1);
        let response = UreqHttp::new().get(&format!("{base}/nope"), 1 << 20, |_| Flow::Continue).unwrap();
        assert_eq!(response.status, 404);
    }

    #[test]
    fn a_body_over_the_cap_is_refused_before_it_is_read() {
        let base = serve(1);
        let error = UreqHttp::new().get(&format!("{base}/large"), 1000, |_| Flow::Continue).unwrap_err();
        assert!(matches!(error, HttpError::TooLarge { cap: 1000, declared: Some(200_000), .. }), "{error:?}");
    }

    #[test]
    fn a_cancel_stops_the_download_between_reads() {
        let base = serve(1);
        let mut reports = 0;
        let error = UreqHttp::new()
            .get(&format!("{base}/large"), 1 << 20, |_| {
                reports += 1;
                Flow::Cancel
            })
            .unwrap_err();
        assert!(matches!(error, HttpError::Cancelled { .. }), "{error:?}");
        assert_eq!(reports, 1);
    }

    #[test]
    fn an_unreachable_host_is_a_retryable_transport_error() {
        // Nothing listens on a port that was just freed.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let error = UreqHttp::new().get(&format!("http://127.0.0.1:{port}/x"), 1 << 20, |_| Flow::Continue).unwrap_err();
        assert_eq!(error.retry(), Retry::Retryable, "{error:?}");
        let malformed = UreqHttp::new().get("not a url", 1 << 20, |_| Flow::Continue).unwrap_err();
        assert_eq!(malformed.retry(), Retry::Final, "{malformed:?}");
    }
}
