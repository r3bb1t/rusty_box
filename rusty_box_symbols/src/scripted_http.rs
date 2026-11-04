//! An [`HttpClient`] for tests: each URL answers what the test scripted
//! (404 when it scripted nothing), and every request is recorded.

use std::collections::BTreeMap;

use crate::http::{Flow, HttpClient, HttpError, HttpResponse, Progress, Retry};

/// What a scripted URL answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scripted {
    Respond { status: u16, body: Vec<u8> },
    /// No answer: a DNS failure, a refused connection, a timeout.
    Unreachable,
}

#[derive(Default)]
pub struct ScriptedHttp {
    answers: BTreeMap<String, Scripted>,
    requests: Vec<String>,
}

impl ScriptedHttp {
    pub fn respond(&mut self, url: &str, status: u16, body: Vec<u8>) {
        self.answers.insert(url.to_owned(), Scripted::Respond { status, body });
    }

    pub fn unreachable(&mut self, url: &str) {
        self.answers.insert(url.to_owned(), Scripted::Unreachable);
    }

    /// Every URL requested, in order.
    #[must_use]
    pub fn requests(&self) -> &[String] {
        &self.requests
    }
}

impl HttpClient for ScriptedHttp {
    fn get<P>(&mut self, url: &str, cap: u64, mut progress: P) -> Result<HttpResponse, HttpError>
    where
        P: FnMut(Progress) -> Flow,
    {
        self.requests.push(url.to_owned());
        match self.answers.get(url) {
            None => Ok(HttpResponse { status: 404, body: Vec::new() }),
            Some(Scripted::Unreachable) => Err(HttpError::Transport {
                url: url.to_owned(),
                message: "the host did not answer".to_owned(),
                retry: Retry::Retryable,
            }),
            Some(Scripted::Respond { status, body }) => {
                let len = u64::try_from(body.len()).unwrap_or(u64::MAX);
                if len > cap {
                    return Err(HttpError::TooLarge { url: url.to_owned(), cap, declared: Some(len) });
                }
                match progress(Progress { done: len, total: Some(len) }) {
                    Flow::Continue => Ok(HttpResponse { status: *status, body: body.clone() }),
                    Flow::Cancel => Err(HttpError::Cancelled { url: url.to_owned() }),
                }
            }
        }
    }
}
