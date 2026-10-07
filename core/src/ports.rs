//! What the core needs from the outside world. Hosts implement the two foreign traits
//! (`SecretStore`, `ProgressObserver`); the rest are internal seams so tests never touch the network,
//! the clock, or a real model.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::{AutoPaperError, Result};
use crate::model::{ProgressDetail, ProgressStage};

/// The platform's secure store: Keychain (macOS), Credential Manager (Windows), Secret Service (Linux).
/// Accounts are the names from `model::secret_account_for` (one per hosted provider, one per OpenAI-compatible
/// server). Implementations may block briefly
/// (an unlock prompt); the core never calls them on a UI thread it owns.
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    fn get(&self, account: String) -> Option<String>;
    fn set(&self, account: String, value: String);
    fn delete(&self, account: String);
}

/// Progress of one generation. Called on the thread running the generation (whichever thread polls the
/// async call; not necessarily the UI thread), so hosts marshal to their UI thread.
#[uniffi::export(with_foreign)]
pub trait ProgressObserver: Send + Sync {
    fn on_progress(&self, stage: ProgressStage);
}

/// Progress in detail: the stage plus, while painting, how far along it is and about how long is left
/// (`ProgressDetail`). Registered once per engine (`Engine::set_progress_detail_observer`) and called for every
/// generation, whichever call started it (`generate`, `run_if_due`, `make_echo`), alongside the per-call
/// `ProgressObserver` — so hosts that don't register one keep working unchanged. Called on the thread running the
/// generation (not necessarily the UI thread): at each stage, and while painting on each reported step and about
/// once a second when the numbers change.
#[uniffi::export(with_foreign)]
pub trait ProgressDetailObserver: Send + Sync {
    fn on_progress_detail(&self, detail: ProgressDetail);
}

/// Wall-clock time in Unix seconds. Tests use a settable clock to simulate years.
pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

/// Turns concept text into a unit-length vector for similarity.
pub trait Embedder: Send + Sync {
    /// Identifies the model, stored with each embedding so a model change can re-embed.
    fn model_id(&self) -> &str;
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

/// One HTTP request. The client checks `url` (and any redirect) against `policy` before sending.
/// Its `Debug` output redacts credential headers and leaves out the body, so logging a request can't
/// leak a key.
#[derive(Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub policy: crate::net::HostPolicy,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout_secs: u64,
    /// Responses larger than this fail instead of being read.
    pub max_response_bytes: usize,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                let secret = crate::net::CREDENTIAL_HEADERS.iter().any(|c| c.eq_ignore_ascii_case(name));
                (name.as_str(), if secret { "[redacted]" } else { value.as_str() })
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("policy", &self.policy)
            .field("headers", &headers)
            .field("body_bytes", &self.body.as_ref().map(Vec::len))
            .field("timeout_secs", &self.timeout_secs)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[async_trait]
pub trait HttpClient: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse>;

    /// Opens a WebSocket for reading a server's text messages (ComfyUI's `/ws` progress events). `request.url` is
    /// the http(s) address of the socket (checked against `request.policy` like any request); `timeout_secs`
    /// covers connecting and the handshake; `max_response_bytes` caps one message. Best effort: callers carry on
    /// without it. Clients that can't open sockets say `Offline` (the default).
    async fn open_socket(&self, request: HttpRequest) -> Result<Box<dyn Socket>> {
        let _ = request;
        Err(AutoPaperError::Offline)
    }
}

/// An open WebSocket, read one text message at a time.
#[async_trait]
pub trait Socket: Send {
    /// The next text message; `None` once the server closed it. Binary messages (previews) are skipped. An error
    /// means the connection broke (the caller stops reading).
    async fn next_text(&mut self) -> Result<Option<String>>;
}

pub type SharedSecrets = Arc<dyn SecretStore>;
pub type SharedObserver = Arc<dyn ProgressObserver>;
pub type SharedDetailObserver = Arc<dyn ProgressDetailObserver>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_redacts_credential_headers_and_the_body() {
        let request = HttpRequest {
            method: HttpMethod::Post,
            url: "https://api.openai.com/v1/responses".into(),
            policy: crate::net::HostPolicy::Hosted(vec!["api.openai.com".into()]),
            headers: vec![
                ("Authorization".into(), "Bearer sk-secret".into()),
                ("x-goog-api-key".into(), "AIza-secret".into()),
                ("content-type".into(), "application/json".into()),
            ],
            body: Some(b"{\"prompt\":\"private\"}".to_vec()),
            timeout_secs: 60,
            max_response_bytes: 1024,
        };
        let printed = format!("{request:?}");
        assert!(!printed.contains("sk-secret") && !printed.contains("AIza-secret"), "{printed}");
        assert!(!printed.contains("private"), "{printed}");
        assert!(printed.contains("application/json") && printed.contains("[redacted]"), "{printed}");
    }
}
