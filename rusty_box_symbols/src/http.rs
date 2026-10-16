//! The engine's one way onto the network: a GET, its body read in chunks
//! with progress reported and a way to stop between chunks.

/// How far a download has come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    /// The size the server declared, when it declared one.
    pub total: Option<u64>,
}

/// Whether a download goes on after a progress report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Flow {
    Continue,
    Cancel,
}

/// A response whose body was read whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Whether trying a failed request again may succeed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    /// A network or server condition that may pass (DNS, a reset, a
    /// timeout, a 5xx).
    Retryable,
    /// Asking again gives the same answer (a malformed URL, a refusal).
    Final,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HttpError {
    #[error("{url}: {message}")]
    Transport { url: String, message: String, retry: Retry },
    #[error("{url}: the response broke off: {message}")]
    Body { url: String, message: String },
    #[error("{url}: the file is larger than the {cap} bytes allowed")]
    TooLarge { url: String, cap: u64, declared: Option<u64> },
    #[error("{url}: cancelled")]
    Cancelled { url: String },
}

impl HttpError {
    #[must_use]
    pub fn retry(&self) -> Retry {
        match self {
            HttpError::Transport { retry, .. } => *retry,
            HttpError::Body { .. } => Retry::Retryable,
            HttpError::TooLarge { .. } | HttpError::Cancelled { .. } => Retry::Final,
        }
    }
}

/// What a response status means for a lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusClass {
    Ok,
    /// 404 and 410: the server does not have the file.
    NotFound,
    /// 408, 429 and 5xx: the server may have it later.
    Retryable,
    /// Every other status: asking again changes nothing.
    Final,
}

#[must_use]
pub fn classify(status: u16) -> StatusClass {
    match status {
        200..=299 => StatusClass::Ok,
        404 | 410 => StatusClass::NotFound,
        408 | 429 | 500..=599 => StatusClass::Retryable,
        _ => StatusClass::Final,
    }
}

/// The network port.
pub trait HttpClient {
    /// GETs `url`, following redirects, and reads its body; a body (or a
    /// declared length) over `cap` bytes is refused, and `progress` hears of
    /// each chunk and may cancel before the next. A status of 400 or more
    /// is a response, not an error.
    fn get<P>(&mut self, url: &str, cap: u64, progress: P) -> Result<HttpResponse, HttpError>
    where
        P: FnMut(Progress) -> Flow;
}
