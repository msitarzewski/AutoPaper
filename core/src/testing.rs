//! Test doubles shared by unit tests, integration tests and `autopaper simulate`: a scripted HTTP client
//! that records requests (and plays scripted WebSockets), an in-memory secret store, and a settable clock. Small
//! and dependency-free, so it's always compiled.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use crate::error::{AutoPaperError, Result};
use crate::net::check_url;
use crate::ports::{Clock, HttpClient, HttpRequest, HttpResponse, SecretStore, Socket};

/// Answers requests from a script of (URL substring, response) rules, first match wins; a rule is
/// used once unless added with `always`. Requests are checked against their `HostPolicy` like the real
/// client and recorded. An unmatched request fails with `Offline`. WebSockets play a script added with
/// `socket` (once per script); without one, opening a socket fails with `Offline`.
#[derive(Default)]
pub struct StubHttp {
    rules: Mutex<Vec<Rule>>,
    requests: Mutex<Vec<HttpRequest>>,
    sockets: Mutex<Vec<(String, Vec<SocketStep>)>>,
    opened: Mutex<Vec<HttpRequest>>,
}

/// One step of a scripted WebSocket.
#[derive(Debug, Clone, PartialEq)]
pub enum SocketStep {
    /// The server sends this text.
    Text(String),
    /// Nothing for this long (tokio's clock, so paused-time tests stay instant).
    Wait(Duration),
    /// The server closes the socket.
    Close,
    /// The connection breaks.
    Break,
}

/// A socket playing its script; after the last step it stays open and silent. Like a real socket, a read dropped
/// half way loses nothing: a wait under way keeps its end time for the next read.
struct ScriptedSocket {
    steps: VecDeque<SocketStep>,
    until: Option<tokio::time::Instant>,
}

#[async_trait]
impl Socket for ScriptedSocket {
    async fn next_text(&mut self) -> Result<Option<String>> {
        loop {
            if let Some(until) = self.until {
                tokio::time::sleep_until(until).await;
                self.until = None;
            }
            match self.steps.pop_front() {
                Some(SocketStep::Text(text)) => return Ok(Some(text)),
                Some(SocketStep::Wait(duration)) => self.until = Some(tokio::time::Instant::now() + duration),
                Some(SocketStep::Close) => return Ok(None),
                Some(SocketStep::Break) => return Err(AutoPaperError::Offline),
                None => std::future::pending::<()>().await,
            }
        }
    }
}

struct Rule {
    url_contains: String,
    response: std::result::Result<HttpResponse, fn() -> AutoPaperError>,
    repeat: bool,
}

impl StubHttp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Answers the next request whose URL contains `url_contains`, once.
    pub fn once(&self, url_contains: &str, status: u16, body: impl Into<Vec<u8>>) -> &Self {
        self.push(url_contains, Ok(response(status, body)), false)
    }

    /// Answers every request whose URL contains `url_contains`.
    pub fn always(&self, url_contains: &str, status: u16, body: impl Into<Vec<u8>>) -> &Self {
        self.push(url_contains, Ok(response(status, body)), true)
    }

    /// Answers once with a full response (custom headers).
    pub fn once_response(&self, url_contains: &str, response: HttpResponse) -> &Self {
        self.push(url_contains, Ok(response), false)
    }

    /// Fails the next matching request with the error `make` returns (e.g. `|| AutoPaperError::Offline`).
    pub fn fail_once(&self, url_contains: &str, make: fn() -> AutoPaperError) -> &Self {
        self.push(url_contains, Err(make), false)
    }

    fn push(&self, url_contains: &str, response: std::result::Result<HttpResponse, fn() -> AutoPaperError>, repeat: bool) -> &Self {
        self.rules.lock().unwrap().push(Rule { url_contains: url_contains.to_string(), response, repeat });
        self
    }

    /// Every request sent, in order.
    pub fn requests(&self) -> Vec<HttpRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Opens the next WebSocket whose URL contains `url_contains` with this script, once.
    pub fn socket(&self, url_contains: &str, script: Vec<SocketStep>) -> &Self {
        self.sockets.lock().unwrap().push((url_contains.to_string(), script));
        self
    }

    /// Every WebSocket asked for (opened or not), in order.
    pub fn sockets_opened(&self) -> Vec<HttpRequest> {
        self.opened.lock().unwrap().clone()
    }

    /// The JSON body of the n-th request.
    pub fn json_body(&self, n: usize) -> serde_json::Value {
        let requests = self.requests();
        let body = requests[n].body.as_deref().unwrap_or_default();
        serde_json::from_slice(body).expect("request body is JSON")
    }
}

#[async_trait]
impl HttpClient for StubHttp {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
        check_url(&request.url, &request.policy)?;
        self.requests.lock().unwrap().push(request.clone());
        let mut rules = self.rules.lock().unwrap();
        let Some(index) = rules.iter().position(|rule| request.url.contains(&rule.url_contains)) else {
            return Err(AutoPaperError::Offline);
        };
        let outcome = match &rules[index].response {
            Ok(response) => Ok(response.clone()),
            Err(make) => Err(make()),
        };
        if !rules[index].repeat {
            rules.remove(index);
        }
        outcome
    }

    async fn open_socket(&self, request: HttpRequest) -> Result<Box<dyn Socket>> {
        check_url(&request.url, &request.policy)?;
        self.opened.lock().unwrap().push(request.clone());
        let mut sockets = self.sockets.lock().unwrap();
        let index = sockets.iter().position(|(part, _)| request.url.contains(part.as_str())).ok_or(AutoPaperError::Offline)?;
        let (_, script) = sockets.remove(index);
        Ok(Box::new(ScriptedSocket { steps: script.into(), until: None }))
    }
}

pub fn response(status: u16, body: impl Into<Vec<u8>>) -> HttpResponse {
    HttpResponse { status, headers: vec![("content-type".into(), "application/json".into())], body: body.into() }
}

/// Secrets in memory.
#[derive(Default)]
pub struct StubSecrets {
    values: Mutex<HashMap<String, String>>,
}

impl StubSecrets {
    pub fn with(pairs: &[(&str, &str)]) -> Self {
        let secrets = Self::default();
        for (account, value) in pairs {
            secrets.set(account.to_string(), value.to_string());
        }
        secrets
    }
}

impl SecretStore for StubSecrets {
    fn get(&self, account: String) -> Option<String> {
        self.values.lock().unwrap().get(&account).cloned()
    }

    fn set(&self, account: String, value: String) {
        self.values.lock().unwrap().insert(account, value);
    }

    fn delete(&self, account: String) {
        self.values.lock().unwrap().remove(&account);
    }
}

/// A clock tests move by hand (Unix seconds).
pub struct FixedClock(AtomicI64);

impl FixedClock {
    pub fn at(unix: i64) -> Self {
        Self(AtomicI64::new(unix))
    }

    pub fn set(&self, unix: i64) {
        self.0.store(unix, Ordering::SeqCst);
    }

    pub fn advance_days(&self, days: f64) {
        self.0.fetch_add((days * 86_400.0) as i64, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
