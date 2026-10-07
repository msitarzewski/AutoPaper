//! The one HTTP client and its policy (AudioPaper's untrusted-response hardening, adapted):
//! hosted providers talk https to their API host only; local and OpenAI-compatible endpoints may use
//! plain http only to loopback / private / link-local / `.local` addresses. No cookies, no cache, no
//! cross-host redirects (≤ 3 same-host), response-size caps, per-request timeouts, Retry-After honoured
//! by callers. Keys are never logged and never appear in error details.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, LOCATION};

use crate::error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason, Result};
use crate::model::ProviderKind;
use crate::ports::{HttpClient, HttpMethod, HttpRequest, HttpResponse, Socket};

/// Where a request may go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPolicy {
    /// https only, to exactly these hosts (lower-case DNS names), default port.
    Hosted(Vec<String>),
    /// The person's own endpoint: https to any named host; http only when the host is loopback
    /// (127.0.0.0/8, ::1, "localhost"), private (10/8, 172.16/12, 192.168/16, fc00::/7), link-local
    /// (169.254/16, fe80::/10) or ends in ".local". No credentials in the URL.
    UserEndpoint,
}

/// Parses and checks a URL against the policy. `InvalidInput` with reason `AddressInvalid` (not a complete
/// web address) or `AddressNotAllowed` (the policy refuses it), and a detail in plain words for logs (e.g.
/// "plain http is only allowed to this computer or your local network").
pub fn check_url(url: &str, policy: &HostPolicy) -> Result<url::Url> {
    let malformed = |detail: &str| AutoPaperError::invalid_input(InvalidInputReason::AddressInvalid, detail);
    let invalid = |detail: &str| AutoPaperError::invalid_input(InvalidInputReason::AddressNotAllowed, detail);
    let parsed = url::Url::parse(url.trim()).map_err(|_| malformed("that isn't a complete web address"))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(invalid("web addresses can't include a user name or password"));
    }
    let host = parsed.host().ok_or_else(|| malformed("the web address has no host"))?;
    match policy {
        HostPolicy::Hosted(hosts) => {
            let named = match host {
                url::Host::Domain(name) => name.to_ascii_lowercase(),
                _ => return Err(invalid("this provider is only reached by its own address")),
            };
            if parsed.scheme() != "https" || parsed.port().is_some() || !hosts.contains(&named) {
                return Err(invalid("this provider is only reached by its own address"));
            }
        }
        HostPolicy::UserEndpoint => match parsed.scheme() {
            "https" => {}
            "http" => {
                let local = match host {
                    url::Host::Domain(name) => {
                        let name = name.to_ascii_lowercase();
                        name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local")
                    }
                    url::Host::Ipv4(ip) => is_local_ip(IpAddr::V4(ip)),
                    url::Host::Ipv6(ip) => is_local_ip(IpAddr::V6(ip)),
                };
                if !local {
                    return Err(invalid(
                        "plain http is only allowed to this computer or your local network; use https",
                    ));
                }
            }
            _ => return Err(invalid("use an http or https address")),
        },
    }
    Ok(parsed)
}

/// Loopback, private (RFC 1918 / unique local) or link-local.
fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local_v4(v4),
            None => is_local_v6(v6),
        },
    }
}

fn is_local_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

fn is_local_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
}

/// reqwest with rustls (the OS trust store); ephemeral: no cookie store, no cache. The system proxy
/// settings are honoured except for local destinations (loopback, private and link-local addresses,
/// `localhost`, `.local`), which are always reached directly: macOS's proxy exception list isn't read,
/// and a local server must never be reached through a proxy. The request's URL is checked with
/// `check_url` against its `policy` before any connection.
///
/// Redirects (301/302/303/307/308) are followed by hand, at most 3, and only when the target passes
/// `check_url` with the same policy, has the same host as the original URL, and isn't a downgrade from
/// https to http. Credential headers are dropped when a redirect changes scheme or port; 303 (and
/// 301/302 after a POST) continue as a GET without the body. Any other redirect isn't followed: the 3xx
/// response is returned, so callers' `error_for_status` reports it.
///
/// Connect timeout 10 s; the request's `timeout_secs` (at least 1 s) covers everything, redirects and
/// the body included. The body is read in chunks and abandoned past `max_response_bytes`
/// (`InvalidResponse`). Response header names are lower-case; values are decoded lossily.
///
/// Errors: connection, DNS and transport failures, and timeouts, → `Offline` (the client doesn't know
/// which provider it's talking to, so it can't build `ProviderUnavailable`; providers that know better,
/// such as a local server that isn't running or is too slow, map `Offline` to `ProviderUnavailable`
/// themselves). TLS/certificate failures → `InvalidResponse`; a header that can't be sent →
/// `InvalidInput` naming the header, never its value. The User-Agent is
/// `AutoPaper/<version> (<client>; +https://github.com/msitarzewski/AutoPaper)`.
///
/// WebSockets (`open_socket`, for ComfyUI's progress events): plain `ws://` to the destinations plain http may
/// reach (local and private addresses; `check_url` with the request's policy), connected directly (never through
/// a proxy), same User-Agent, the request's timeout over connecting and the handshake, `max_response_bytes` per
/// message. An https address (`wss://`) isn't opened: callers carry on without the socket.
#[derive(Debug, Clone)]
pub struct ReqwestClient {
    /// For public hosts: system proxy settings apply.
    proxied: reqwest::Client,
    /// For local destinations: never through a proxy.
    direct: reqwest::Client,
    user_agent: HeaderValue,
}

const MAX_REDIRECTS: usize = 3;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROJECT_URL: &str = "https://github.com/msitarzewski/AutoPaper";
/// Request headers that carry credentials: never kept across a change of scheme or port, and marked
/// sensitive (kept out of reqwest's debug output and HTTP/2 header compression tables).
pub(crate) const CREDENTIAL_HEADERS: &[&str] =
    &["authorization", "proxy-authorization", "cookie", "x-goog-api-key", "x-api-key", "api-key"];

impl ReqwestClient {
    /// `client` names the host app in the User-Agent ("macOS", "Windows", "Linux", "cli"): 1–64
    /// printable ASCII characters without `(`, `)`, `;` or `\`. `InvalidInput` otherwise; `Internal` if
    /// the TLS stack can't start.
    pub fn new(client: &str) -> Result<Self> {
        Self::build(client, None)
    }

    /// `proxy` replaces the system proxy settings for public hosts (tests).
    fn build(client: &str, proxy: Option<reqwest::Proxy>) -> Result<Self> {
        let user_agent = HeaderValue::from_str(&user_agent(client)?)
            .map_err(|_| AutoPaperError::invalid_input(InvalidInputReason::Other, "the client name can't be sent"))?;
        let builder = || {
            reqwest::Client::builder()
                .user_agent(user_agent.clone())
                .redirect(reqwest::redirect::Policy::none())
                .referer(false)
                .connect_timeout(CONNECT_TIMEOUT)
        };
        let start = |builder: reqwest::ClientBuilder| {
            builder.build().map_err(|error| AutoPaperError::Internal {
                detail: format!("couldn't start the HTTP client: {error}"),
            })
        };
        let proxied = match proxy {
            Some(proxy) => builder().proxy(proxy),
            None => builder(),
        };
        Ok(Self { proxied: start(proxied)?, direct: start(builder().no_proxy())?, user_agent })
    }

    fn client_for(&self, url: &url::Url) -> &reqwest::Client {
        if is_local_destination(url) { &self.direct } else { &self.proxied }
    }

    /// Sends `request` to `url` (already checked), following allowed redirects, and reads the body.
    async fn exchange(&self, mut url: url::Url, request: HttpRequest) -> Result<HttpResponse> {
        let HttpRequest { method, policy, headers, mut body, max_response_bytes, .. } = request;
        let mut method = match method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
        };
        let mut headers = header_map(&headers)?;
        let original = url.clone();
        let mut redirects = 0;
        loop {
            let mut outgoing = self.client_for(&url).request(method.clone(), url.clone()).headers(headers.clone());
            if let Some(bytes) = &body {
                outgoing = outgoing.body(bytes.clone());
            }
            let response = outgoing.send().await.map_err(transport_error)?;
            let status = response.status().as_u16();
            let location = response.headers().get(LOCATION).and_then(|value| value.to_str().ok());
            let next = match next_hop(status, location, &url, &original, &policy) {
                Some(next) if redirects < MAX_REDIRECTS => next,
                _ => return read_body(response, max_response_bytes).await,
            };
            redirects += 1;
            if status == 303 || (matches!(status, 301 | 302) && method == reqwest::Method::POST) {
                method = reqwest::Method::GET;
                body = None;
                headers.remove(CONTENT_TYPE);
                headers.remove(CONTENT_LENGTH);
            }
            if next.scheme() != url.scheme() || next.port_or_known_default() != url.port_or_known_default() {
                for name in CREDENTIAL_HEADERS {
                    headers.remove(*name);
                }
            }
            tracing::debug!(status, host = next.host_str().unwrap_or_default(), "following a redirect");
            url = next;
        }
    }
}

#[async_trait]
impl HttpClient for ReqwestClient {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
        let url = check_url(&request.url, &request.policy)?;
        let limit = Duration::from_secs(request.timeout_secs.max(1));
        let host = url.host_str().unwrap_or_default().to_string();
        match tokio::time::timeout(limit, self.exchange(url, request)).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(host, seconds = limit.as_secs(), "request timed out");
                Err(AutoPaperError::Offline)
            }
        }
    }

    async fn open_socket(&self, request: HttpRequest) -> Result<Box<dyn Socket>> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::http::header::USER_AGENT;
        use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

        let url = check_url(&request.url, &request.policy)?;
        if url.scheme() != "http" || !is_local_destination(&url) {
            return Err(AutoPaperError::Offline);
        }
        let host = url.host_str().unwrap_or_default().to_string();
        let port = url.port_or_known_default().unwrap_or(80);
        let mut address = url.clone();
        address.set_scheme("ws").map_err(|()| AutoPaperError::Offline)?;
        let mut handshake = address.as_str().into_client_request().map_err(|_| AutoPaperError::Offline)?;
        handshake.headers_mut().insert(USER_AGENT, self.user_agent.clone());
        let config = WebSocketConfig::default()
            .max_message_size(Some(request.max_response_bytes))
            .max_frame_size(Some(request.max_response_bytes));
        let connect = async {
            let stream = tokio::net::TcpStream::connect(format!("{host}:{port}")).await.map_err(|error| {
                tracing::debug!(%error, "couldn't connect a WebSocket");
                AutoPaperError::Offline
            })?;
            let (socket, _) =
                tokio_tungstenite::client_async_with_config(handshake, stream, Some(config)).await.map_err(|error| {
                    tracing::debug!(error = %redact(&error.to_string()), "the WebSocket handshake failed");
                    AutoPaperError::Offline
                })?;
            Ok::<_, AutoPaperError>(socket)
        };
        let limit = Duration::from_secs(request.timeout_secs.max(1));
        let socket = tokio::time::timeout(limit, connect).await.map_err(|_| AutoPaperError::Offline)??;
        Ok(Box::new(TungsteniteSocket(socket)))
    }
}

/// An open WebSocket over the core's own TCP connection.
struct TungsteniteSocket(tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>);

#[async_trait]
impl Socket for TungsteniteSocket {
    async fn next_text(&mut self) -> Result<Option<String>> {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::Message;
        loop {
            match self.0.next().await {
                Some(Ok(Message::Text(text))) => return Ok(Some(text.as_str().to_string())),
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    tracing::debug!(error = %redact(&error.to_string()), "a WebSocket broke");
                    return Err(AutoPaperError::Offline);
                }
            }
        }
    }
}

/// "AutoPaper/<version> (<client>; +<project URL>)", with `client` validated as `ReqwestClient::new`
/// describes.
fn user_agent(client: &str) -> Result<String> {
    let client = client.trim();
    let acceptable = |c: char| (c.is_ascii_graphic() || c == ' ') && !matches!(c, '(' | ')' | ';' | '\\');
    if client.is_empty() || client.len() > 64 || !client.chars().all(acceptable) {
        return Err(AutoPaperError::invalid_input(InvalidInputReason::Other, "the client name can't be used in a User-Agent"));
    }
    Ok(format!("AutoPaper/{} ({client}; +{PROJECT_URL})", env!("CARGO_PKG_VERSION")))
}

/// Loopback, private or link-local addresses, and `localhost` / `.localhost` / `.local` names: the
/// destinations `check_url` lets plain http reach.
fn is_local_destination(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => {
            let name = name.to_ascii_lowercase();
            name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local")
        }
        Some(url::Host::Ipv4(ip)) => is_local_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_local_ip(IpAddr::V6(ip)),
        None => false,
    }
}

fn header_map(headers: &[(String, String)]) -> Result<HeaderMap> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| AutoPaperError::invalid_input(InvalidInputReason::Other, "a request header has an invalid name"))?;
        let mut value = HeaderValue::from_str(value).map_err(|_| {
            AutoPaperError::invalid_input(InvalidInputReason::Other, format!("the {name} header has characters that can't be sent"))
        })?;
        if CREDENTIAL_HEADERS.contains(&name.as_str()) {
            value.set_sensitive(true);
        }
        map.append(name, value);
    }
    Ok(map)
}

/// Where a redirect response may take the request, or `None` when it must not be followed: not a
/// following status, no usable `Location`, a target failing `check_url` with `policy`, another host
/// than `original`'s, or https → http.
fn next_hop(
    status: u16,
    location: Option<&str>,
    current: &url::Url,
    original: &url::Url,
    policy: &HostPolicy,
) -> Option<url::Url> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let target = current.join(location?.trim()).ok()?;
    let next = check_url(target.as_str(), policy).ok()?;
    let downgrade = current.scheme() == "https" && next.scheme() == "http";
    (next.host() == original.host() && !downgrade).then_some(next)
}

async fn read_body(mut response: reqwest::Response, max_bytes: usize) -> Result<HttpResponse> {
    let too_large =
        || AutoPaperError::InvalidResponse { detail: format!("the response is larger than {max_bytes} bytes") };
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (name.as_str().to_ascii_lowercase(), String::from_utf8_lossy(value.as_bytes()).into_owned())
        })
        .collect();
    let declared = response.content_length().map(|length| usize::try_from(length).unwrap_or(usize::MAX));
    if declared.is_some_and(|length| length > max_bytes) {
        return Err(too_large());
    }
    let mut body = Vec::with_capacity(declared.unwrap_or(0));
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if chunk.len() > max_bytes.saturating_sub(body.len()) {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(HttpResponse { status, headers, body })
}

/// Maps a reqwest failure to what a host can act on. The error (without its URL, redacted) goes to the
/// log only.
fn transport_error(error: reqwest::Error) -> AutoPaperError {
    let error = error.without_url();
    tracing::debug!(error = %redact(&error_chain(&error)), "HTTP request failed");
    if error.is_timeout() {
        AutoPaperError::Offline
    } else if is_tls_failure(&error) {
        AutoPaperError::InvalidResponse { detail: "the secure connection failed (certificate or TLS error)".into() }
    } else if error.is_connect() || error.is_request() || error.is_body() {
        AutoPaperError::Offline
    } else if error.is_decode() {
        AutoPaperError::InvalidResponse { detail: "the response couldn't be read".into() }
    } else if error.is_builder() {
        AutoPaperError::invalid_input(InvalidInputReason::Other, "the request couldn't be built")
    } else {
        AutoPaperError::Internal { detail: "the HTTP client failed".into() }
    }
}

/// rustls reports handshake and certificate failures as `io::ErrorKind::InvalidData` during connect,
/// wrapped in further `io::Error`s (whose `source()` skips the wrapped error, so `get_ref` is followed).
fn is_tls_failure(error: &reqwest::Error) -> bool {
    fn has_invalid_data(error: &(dyn std::error::Error + 'static)) -> bool {
        let mut current = Some(error);
        while let Some(cause) = current {
            if let Some(io) = cause.downcast_ref::<std::io::Error>() {
                if io.kind() == std::io::ErrorKind::InvalidData {
                    return true;
                }
                if io.get_ref().is_some_and(|inner| has_invalid_data(inner)) {
                    return true;
                }
            }
            current = cause.source();
        }
        false
    }
    error.is_connect() && std::error::Error::source(error).is_some_and(has_invalid_data)
}

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// `Retry-After` in seconds (delta-seconds or HTTP date), capped at 3600.
pub fn retry_after_secs(response: &HttpResponse) -> Option<u32> {
    const CAP: u64 = 3600;
    let value = response.header("retry-after")?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds.min(CAP) as u32);
    }
    let when = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let delta = when.timestamp() - chrono::Utc::now().timestamp();
    Some(delta.clamp(0, CAP as i64) as u32)
}

/// Maps a non-2xx response to an error (`None` for 2xx): 401/403 → `InvalidKey`; 429 → `RateLimited`
/// (Retry-After, default 60); 408/504 → `ProviderUnavailable` `TimedOut`, 500/502/503 → `ProviderUnavailable`
/// `ServerError`; others → `InvalidResponse`
/// with the status and, if the body is JSON with an error message, its first 200 characters (with
/// anything that looks like a key — "sk-…", "AIza…", long base64 runs — redacted).
pub fn error_for_status(provider: ProviderKind, response: &HttpResponse) -> Option<AutoPaperError> {
    let status = response.status;
    if (200..300).contains(&status) {
        return None;
    }
    Some(match status {
        401 | 403 => AutoPaperError::InvalidKey { provider },
        429 => AutoPaperError::RateLimited { provider, retry_after_secs: retry_after_secs(response).unwrap_or(60) },
        408 | 504 => AutoPaperError::unavailable(provider, ProviderUnavailableReason::TimedOut, format!("HTTP {status}")),
        500 | 502 | 503 => AutoPaperError::unavailable(provider, ProviderUnavailableReason::ServerError, format!("HTTP {status}")),
        _ => match error_message(&response.body) {
            Some(message) => AutoPaperError::InvalidResponse { detail: format!("HTTP {status}: {message}") },
            None => AutoPaperError::InvalidResponse { detail: format!("HTTP {status}") },
        },
    })
}

/// The human-readable message from a JSON error body (OpenAI/Google `error.message`, or a top-level
/// `error`/`message` string), first 200 characters, redacted.
pub fn error_message(body: &[u8]) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = json
        .pointer("/error/message")
        .or_else(|| json.get("error").filter(|e| e.is_string()))
        .or_else(|| json.get("message"))
        .and_then(|m| m.as_str())?;
    let short: String = message.chars().take(200).collect();
    Some(redact(&short))
}

/// Replaces anything that looks like a credential with `[redacted]`.
pub fn redact(text: &str) -> String {
    let is_token_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '/' | '=' | '.');
    // Words and numbers joined by `-`, `_` or `/` ("models/gemini-3-pro-image-preview") name things; a key's
    // random characters mix letters and digits within its parts.
    let is_name = |token: &str| {
        token.split(['-', '_', '/']).all(|part| {
            part.chars().all(|c| c.is_ascii_alphabetic()) || part.chars().all(|c| c.is_ascii_digit())
        })
    };
    let looks_secret = |token: &str| {
        let has_digit = token.chars().any(|c| c.is_ascii_digit());
        let has_alpha = token.chars().any(|c| c.is_ascii_alphabetic());
        (token.starts_with("sk-") && token.len() >= 20)
            || (token.starts_with("AIza") && token.len() >= 30)
            || (token.len() >= 32 && has_digit && has_alpha && !token.contains('.') && !is_name(token))
    };
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    // A sentence's closing full stop isn't part of the token.
    let flush = |token: &mut String, out: &mut String| {
        let body = token.trim_end_matches('.');
        if looks_secret(body) {
            out.push_str("[redacted]");
            out.push_str(&token[body.len()..]);
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if is_token_char(c) {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosted() -> HostPolicy {
        HostPolicy::Hosted(vec!["api.openai.com".into()])
    }

    #[test]
    fn hosted_policy_allows_only_https_to_named_hosts() {
        assert!(check_url("https://api.openai.com/v1/models", &hosted()).is_ok());
        assert!(check_url("https://API.OPENAI.COM/v1/models", &hosted()).is_ok());
        assert!(check_url("http://api.openai.com/v1/models", &hosted()).is_err());
        assert!(check_url("https://api.openai.com:8443/v1", &hosted()).is_err());
        assert!(check_url("https://evil.example/v1", &hosted()).is_err());
        assert!(check_url("https://api.openai.com.evil.example/v1", &hosted()).is_err());
        assert!(check_url("https://user:pw@api.openai.com/v1", &hosted()).is_err());
        assert!(check_url("https://104.18.0.1/v1", &hosted()).is_err());
    }

    #[test]
    fn user_endpoints_allow_http_only_locally() {
        let p = HostPolicy::UserEndpoint;
        for ok in [
            "http://127.0.0.1:8188", "http://localhost:11434/api/tags", "http://[::1]:1234/v1",
            "http://192.168.4.25:8188", "http://10.0.0.5", "http://172.20.1.1", "http://169.254.1.1",
            "http://studio.local:8188", "http://[fd00::1]:8080", "http://[fe80::1]", "https://api.together.example/v1",
            "https://8.8.8.8/v1", "http://[::ffff:127.0.0.1]:8188",
        ] {
            assert!(check_url(ok, &p).is_ok(), "{ok}");
        }
        for bad in [
            "http://8.8.8.8:8188", "http://example.com", "http://172.32.0.1", "http://0.0.0.0:8188",
            "ftp://127.0.0.1", "file:///etc/passwd", "http://user:pw@127.0.0.1", "not a url",
            "http://[2001:db8::1]", "http://myserver.lan",
        ] {
            assert!(check_url(bad, &p).is_err(), "{bad}");
        }
        let reason = |url: &str| match check_url(url, &p) {
            Err(AutoPaperError::InvalidInput { reason, .. }) => reason,
            other => panic!("{url}: {other:?}"),
        };
        for refused in ["http://8.8.8.8:8188", "http://example.com", "ftp://127.0.0.1", "http://user:pw@127.0.0.1"] {
            assert_eq!(reason(refused), InvalidInputReason::AddressNotAllowed, "{refused}");
        }
        // Forgetting the scheme reads as an incomplete address, not a refused one.
        for incomplete in ["not a url", "127.0.0.1:8188", "localhost:11434", "file:///etc/passwd", ""] {
            assert_eq!(reason(incomplete), InvalidInputReason::AddressInvalid, "{incomplete}");
        }
    }

    fn response(status: u16, headers: &[(&str, &str)], body: &str) -> HttpResponse {
        HttpResponse {
            status,
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn maps_statuses() {
        let p = ProviderKind::OpenAi;
        assert!(error_for_status(p, &response(200, &[], "")).is_none());
        assert!(matches!(error_for_status(p, &response(401, &[], "")), Some(AutoPaperError::InvalidKey { .. })));
        assert!(matches!(
            error_for_status(p, &response(429, &[("Retry-After", "17")], "")),
            Some(AutoPaperError::RateLimited { retry_after_secs: 17, .. })
        ));
        assert!(matches!(
            error_for_status(p, &response(429, &[], "")),
            Some(AutoPaperError::RateLimited { retry_after_secs: 60, .. })
        ));
        let unavailable = |status| match error_for_status(p, &response(status, &[], "")) {
            Some(AutoPaperError::ProviderUnavailable { provider, reason, .. }) => {
                assert_eq!(provider, p);
                reason
            }
            other => panic!("{status}: {other:?}"),
        };
        for status in [500, 502, 503] {
            assert_eq!(unavailable(status), ProviderUnavailableReason::ServerError, "{status}");
        }
        for status in [408, 504] {
            assert_eq!(unavailable(status), ProviderUnavailableReason::TimedOut, "{status}");
        }
        let bad = error_for_status(
            p,
            &response(400, &[], r#"{"error":{"message":"Invalid size; key sk-abcdefghijklmnopqrstuvwxyz0123 used"}}"#),
        );
        match bad {
            Some(AutoPaperError::InvalidResponse { detail }) => {
                assert!(detail.starts_with("HTTP 400: Invalid size"), "{detail}");
                assert!(!detail.contains("sk-abc"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn retry_after_caps_and_parses() {
        assert_eq!(retry_after_secs(&response(429, &[("retry-after", "99999")], "")), Some(3600));
        assert_eq!(retry_after_secs(&response(429, &[("retry-after", "Wed, 21 Oct 2015 07:28:00 GMT")], "")), Some(0));
        assert_eq!(retry_after_secs(&response(429, &[], "")), None);
    }

    #[test]
    fn redacts_credentials() {
        assert_eq!(redact("key sk-proj-abcdefghijklmnop1234 bad"), "key [redacted] bad");
        assert_eq!(redact("AIzaSyA1234567890abcdefghijklmnopqrstu."), "[redacted].");
        assert_eq!(redact("Bearer abcdefghijklmnopqrstuvwxyz0123456789ABCD"), "Bearer [redacted]");
        assert_eq!(redact("Invalid size 1024x1024 for gpt-image-2.5-flare"), "Invalid size 1024x1024 for gpt-image-2.5-flare");
        assert_eq!(redact("https://api.openai.com/v1/images/generations"), "https://api.openai.com/v1/images/generations");
        let missing = "HTTP 404: models/gemini-3-pro-image-preview is not found for API version v1";
        assert_eq!(redact(missing), missing);
        assert_eq!(redact("token a1b2c3d4-e5f6a7b8c9d0e1f2a3b4c5d6e7f8"), "token [redacted]");
    }
}

#[cfg(test)]
mod client_tests {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    /// One request as the test server saw it.
    #[derive(Debug, Clone)]
    struct Seen {
        head: String,
        body: Vec<u8>,
    }

    impl Seen {
        fn request_line(&self) -> &str {
            self.head.lines().next().unwrap_or_default()
        }

        fn path(&self) -> &str {
            self.request_line().split(' ').nth(1).unwrap_or_default()
        }

        fn header(&self, name: &str) -> Option<String> {
            self.head.lines().skip(1).find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
            })
        }
    }

    type Log = Arc<Mutex<Vec<Seen>>>;

    fn seen(log: &Log) -> Vec<Seen> {
        log.lock().unwrap().clone()
    }

    async fn read_request(socket: &mut TcpStream) -> Option<Seen> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break end + 4;
            }
            let read = socket.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
        };
        let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
        let mut request = Seen { head, body: buffer[head_end..].to_vec() };
        let length: usize = request.header("content-length").and_then(|value| value.parse().ok()).unwrap_or(0);
        while request.body.len() < length {
            let read = socket.read(&mut chunk).await.ok()?;
            if read == 0 {
                break;
            }
            request.body.extend_from_slice(&chunk[..read]);
        }
        Some(request)
    }

    /// An HTTP/1.1 server on 127.0.0.1 answering each request with `reply(request, port)` (raw bytes),
    /// one request per connection.
    async fn serve(reply: impl Fn(&Seen, u16) -> Vec<u8> + Send + Sync + 'static) -> (u16, Log) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log: Log = Arc::default();
        let task_log = Arc::clone(&log);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let Some(request) = read_request(&mut socket).await else { continue };
                let response = reply(&request, port);
                task_log.lock().unwrap().push(request);
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            }
        });
        (port, log)
    }

    fn reply(status: &str, headers: &[(&str, &str)], body: &str) -> Vec<u8> {
        let mut text = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n", body.len());
        for (name, value) in headers {
            text.push_str(&format!("{name}: {value}\r\n"));
        }
        text.push_str("\r\n");
        text.push_str(body);
        text.into_bytes()
    }

    /// Production configuration: every test URL is local, so the system proxy never applies.
    fn client() -> ReqwestClient {
        ReqwestClient::new("test").unwrap()
    }

    fn request(method: HttpMethod, url: String) -> HttpRequest {
        HttpRequest {
            method,
            url,
            policy: HostPolicy::UserEndpoint,
            headers: vec![],
            body: None,
            timeout_secs: 10,
            max_response_bytes: 1 << 20,
        }
    }

    fn get(url: String) -> HttpRequest {
        request(HttpMethod::Get, url)
    }

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
    }

    #[tokio::test]
    async fn get_and_post_round_trip_with_headers() {
        let (port, log) = serve(|request, _| match request.path() {
            "/v1/chat" => {
                reply("200 OK", &[("Content-Type", "application/json")], &String::from_utf8_lossy(&request.body))
            }
            _ => reply("200 OK", &[("X-Custom", "Yes"), ("X-Multi", "a"), ("X-Multi", "b")], "hello"),
        })
        .await;
        let client = client();

        let mut models = get(format!("http://127.0.0.1:{port}/v1/models"));
        models.headers = headers(&[("X-Test", "1"), ("Authorization", "Bearer local-key")]);
        let response = client.send(models).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hello");
        assert_eq!(response.header("x-custom"), Some("Yes"));
        assert!(response.headers.contains(&("x-custom".to_string(), "Yes".to_string())));
        assert!(response.headers.iter().all(|(name, _)| *name == name.to_ascii_lowercase()));
        assert_eq!(response.headers.iter().filter(|(name, _)| name == "x-multi").count(), 2);

        let mut chat = request(HttpMethod::Post, format!("http://127.0.0.1:{port}/v1/chat"));
        chat.headers = headers(&[("Content-Type", "application/json")]);
        chat.body = Some(br#"{"model":"m","stream":false}"#.to_vec());
        let response = client.send(chat).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, br#"{"model":"m","stream":false}"#);
        assert_eq!(response.header("content-type"), Some("application/json"));

        let requests = seen(&log);
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].request_line(), "GET /v1/models HTTP/1.1");
        assert_eq!(requests[0].header("x-test").as_deref(), Some("1"));
        assert_eq!(requests[0].header("authorization").as_deref(), Some("Bearer local-key"));
        assert_eq!(requests[1].request_line(), "POST /v1/chat HTTP/1.1");
        assert_eq!(requests[1].header("content-type").as_deref(), Some("application/json"));
        assert_eq!(requests[1].body, br#"{"model":"m","stream":false}"#);
    }

    #[tokio::test]
    async fn user_agent_names_the_project_and_client() {
        let expected =
            format!("AutoPaper/{} (macOS; +https://github.com/msitarzewski/AutoPaper)", env!("CARGO_PKG_VERSION"));
        assert_eq!(user_agent("macOS").unwrap(), expected);
        assert_eq!(user_agent(" cli ").unwrap(), expected.replace("macOS", "cli"));
        let long = "x".repeat(65);
        for bad in ["", "   ", "a(b", "x;y", "back\\slash", "tab\there", "line\nbreak", "é", long.as_str()] {
            assert!(matches!(user_agent(bad), Err(AutoPaperError::InvalidInput { .. })), "{bad:?}");
            assert!(ReqwestClient::new(bad).is_err(), "{bad:?}");
        }
        assert!(ReqwestClient::new("Windows").is_ok());

        let (port, log) = serve(|_, _| reply("204 No Content", &[], "")).await;
        let response = client().send(get(format!("http://127.0.0.1:{port}/"))).await.unwrap();
        assert_eq!(response.status, 204);
        assert_eq!(seen(&log)[0].header("user-agent"), Some(user_agent("test").unwrap()));
    }

    #[tokio::test]
    async fn aborts_responses_over_the_size_cap() {
        let (port, _) = serve(|request, _| match request.path() {
            "/declared" => reply("200 OK", &[], &"x".repeat(100)),
            _ => {
                let mut raw = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n".to_vec();
                for _ in 0..3 {
                    raw.extend(b"28\r\n");
                    raw.extend([b'y'; 40]);
                    raw.extend(b"\r\n");
                }
                raw.extend(b"0\r\n\r\n");
                raw
            }
        })
        .await;
        let client = client();
        let capped = |path: &str, cap: usize| {
            let mut request = get(format!("http://127.0.0.1:{port}{path}"));
            request.max_response_bytes = cap;
            request
        };
        let too_large = |result: Result<HttpResponse>| matches!(result, Err(AutoPaperError::InvalidResponse { ref detail }) if detail.contains("larger than"));
        // Declared length over the cap: refused before reading.
        assert!(too_large(client.send(capped("/declared", 99)).await));
        assert_eq!(client.send(capped("/declared", 100)).await.unwrap().body.len(), 100);
        // No declared length: counted chunk by chunk.
        assert!(too_large(client.send(capped("/chunked", 100)).await));
        assert_eq!(client.send(capped("/chunked", 120)).await.unwrap().body, vec![b'y'; 120]);
    }

    #[tokio::test(start_paused = true)]
    async fn times_out_as_offline() {
        // Accepts and reads the request, then never answers.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = read_request(&mut socket).await;
                let mut rest = [0u8; 1];
                let _ = socket.read(&mut rest).await;
            }
        });
        let mut request = get(format!("http://127.0.0.1:{port}/slow"));
        request.timeout_secs = 1;
        assert!(matches!(client().send(request).await, Err(AutoPaperError::Offline)));
    }

    #[tokio::test]
    async fn connection_refused_is_offline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(matches!(client().send(get(format!("http://127.0.0.1:{port}/"))).await, Err(AutoPaperError::Offline)));
    }

    #[tokio::test]
    async fn tls_failure_is_an_invalid_response() {
        // Speaks plain HTTP to a TLS client hello.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut hello = [0u8; 512];
                let _ = socket.read(&mut hello).await;
                let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n").await;
                let _ = socket.shutdown().await;
            }
        });
        let result = client().send(get(format!("https://127.0.0.1:{port}/"))).await;
        assert!(
            matches!(result, Err(AutoPaperError::InvalidResponse { ref detail }) if detail.contains("secure connection")),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn checks_the_url_before_connecting() {
        let (port, log) = serve(|_, _| reply("200 OK", &[], "")).await;
        let client = client();
        let mut hosted = get(format!("http://127.0.0.1:{port}/"));
        hosted.policy = HostPolicy::Hosted(vec!["api.openai.com".into()]);
        assert!(matches!(client.send(hosted).await, Err(AutoPaperError::InvalidInput { .. })));
        assert!(matches!(client.send(get("http://8.8.8.8/v1".into())).await, Err(AutoPaperError::InvalidInput { .. })));
        assert!(matches!(
            client.send(get(format!("http://user:pw@127.0.0.1:{port}/"))).await,
            Err(AutoPaperError::InvalidInput { .. })
        ));
        assert!(seen(&log).is_empty());
    }

    #[tokio::test]
    async fn rejects_unsendable_headers_without_echoing_them() {
        let (port, log) = serve(|_, _| reply("200 OK", &[], "")).await;
        let mut request = get(format!("http://127.0.0.1:{port}/"));
        request.headers = headers(&[("Authorization", "Bearer sk-secret-value\r\nX-Injected: 1")]);
        match client().send(request).await {
            Err(AutoPaperError::InvalidInput { detail, .. }) => {
                assert!(detail.contains("authorization"), "{detail}");
                assert!(!detail.contains("sk-secret"), "{detail}");
            }
            other => panic!("{other:?}"),
        }
        let mut request = get(format!("http://127.0.0.1:{port}/"));
        request.headers = headers(&[("Bad Name", "1")]);
        assert!(matches!(client().send(request).await, Err(AutoPaperError::InvalidInput { .. })));
        assert!(seen(&log).is_empty());
    }

    #[tokio::test]
    async fn follows_same_host_redirects_without_keeping_cookies() {
        let (port, log) = serve(|request, port| match request.path() {
            "/start" => reply("302 Found", &[("Location", "/middle"), ("Set-Cookie", "session=abc; Path=/")], ""),
            "/middle" => reply("301 Moved Permanently", &[("Location", &format!("http://127.0.0.1:{port}/end"))], ""),
            "/end" => reply("200 OK", &[("Set-Cookie", "late=1")], "done"),
            _ => reply("404 Not Found", &[], ""),
        })
        .await;
        let client = client();
        let response = client.send(get(format!("http://127.0.0.1:{port}/start"))).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"done");
        // A later request on the same client carries no cookie either.
        client.send(get(format!("http://127.0.0.1:{port}/end"))).await.unwrap();
        let requests = seen(&log);
        let paths: Vec<&str> = requests.iter().map(Seen::path).collect();
        assert_eq!(paths, ["/start", "/middle", "/end", "/end"]);
        assert!(requests.iter().all(|request| request.header("cookie").is_none()));
        assert!(requests.iter().all(|request| request.header("referer").is_none()));
    }

    #[tokio::test]
    async fn redirects_keep_or_drop_the_post_body_by_status() {
        let (port, log) = serve(|request, _| match request.path() {
            "/keep" => reply("307 Temporary Redirect", &[("Location", "/landing")], ""),
            "/see-other" => reply("303 See Other", &[("Location", "/landing")], ""),
            "/found" => reply("302 Found", &[("Location", "/landing")], ""),
            _ => {
                let method = request.request_line().split(' ').next().unwrap_or_default();
                reply("200 OK", &[], &format!("{method} {}", String::from_utf8_lossy(&request.body)))
            }
        })
        .await;
        let client = client();
        let post = |path: &str| {
            let mut request = request(HttpMethod::Post, format!("http://127.0.0.1:{port}{path}"));
            request.headers = headers(&[("Content-Type", "application/json")]);
            request.body = Some(b"{\"a\":1}".to_vec());
            request
        };
        assert_eq!(client.send(post("/keep")).await.unwrap().body, b"POST {\"a\":1}");
        assert_eq!(client.send(post("/see-other")).await.unwrap().body, b"GET ");
        assert_eq!(client.send(post("/found")).await.unwrap().body, b"GET ");
        let requests = seen(&log);
        assert_eq!(requests[1].header("content-type").as_deref(), Some("application/json"));
        assert!(requests[3].header("content-type").is_none());
        assert!(requests[5].header("content-type").is_none());
    }

    #[tokio::test]
    async fn follows_at_most_three_redirects() {
        let (port, log) = serve(|request, _| {
            let hop: u32 = request.path().trim_start_matches("/r").parse().unwrap_or(99);
            if hop < 4 {
                reply("302 Found", &[("Location", &format!("/r{}", hop + 1))], "")
            } else {
                reply("200 OK", &[], "arrived")
            }
        })
        .await;
        let client = client();
        // Three hops: /r1 → /r2 → /r3 → /r4.
        let response = client.send(get(format!("http://127.0.0.1:{port}/r1"))).await.unwrap();
        assert_eq!((response.status, response.body.as_slice()), (200, &b"arrived"[..]));
        assert_eq!(seen(&log).len(), 4);
        // Four hops: the fourth redirect comes back as-is.
        let response = client.send(get(format!("http://127.0.0.1:{port}/r0"))).await.unwrap();
        assert_eq!(response.status, 302);
        assert_eq!(response.header("location"), Some("/r4"));
        assert_eq!(seen(&log).len(), 8);
        assert!(matches!(
            error_for_status(ProviderKind::ComfyUi, &response),
            Some(AutoPaperError::InvalidResponse { .. })
        ));
    }

    #[tokio::test]
    async fn does_not_follow_redirects_to_other_hosts_or_disallowed_urls() {
        let (port, log) = serve(|request, port| match request.path() {
            "/cross-host" => reply("302 Found", &[("Location", &format!("http://localhost:{port}/next"))], ""),
            "/credentials" => reply("302 Found", &[("Location", &format!("http://user:pw@127.0.0.1:{port}/next"))], ""),
            "/scheme" => reply("302 Found", &[("Location", "ftp://127.0.0.1/next")], ""),
            "/public" => reply("307 Temporary Redirect", &[("Location", "http://8.8.8.8/next")], ""),
            "/no-location" => reply("302 Found", &[], ""),
            "/not-modified" => reply("304 Not Modified", &[("Location", "/next")], ""),
            _ => reply("200 OK", &[], "next"),
        })
        .await;
        let client = client();
        for (path, status) in [
            ("/cross-host", 302),
            ("/credentials", 302),
            ("/scheme", 302),
            ("/public", 307),
            ("/no-location", 302),
            ("/not-modified", 304),
        ] {
            let response = client.send(get(format!("http://127.0.0.1:{port}{path}"))).await.unwrap();
            assert_eq!(response.status, status, "{path}");
        }
        assert!(seen(&log).iter().all(|request| request.path() != "/next"));
        assert_eq!(seen(&log).len(), 6);
    }

    #[tokio::test]
    async fn drops_credentials_when_a_redirect_changes_port() {
        let (landing, landing_log) = serve(|_, _| reply("200 OK", &[], "landed")).await;
        let (start, start_log) = serve(move |_, _| {
            reply("307 Temporary Redirect", &[("Location", &format!("http://127.0.0.1:{landing}/x"))], "")
        })
        .await;
        let mut request = get(format!("http://127.0.0.1:{start}/"));
        request.headers =
            headers(&[("Authorization", "Bearer local-key"), ("x-goog-api-key", "AIza-local"), ("X-Other", "kept")]);
        let response = client().send(request).await.unwrap();
        assert_eq!(response.body, b"landed");
        assert_eq!(seen(&start_log)[0].header("authorization").as_deref(), Some("Bearer local-key"));
        let landed = &seen(&landing_log)[0];
        assert!(landed.header("authorization").is_none());
        assert!(landed.header("x-goog-api-key").is_none());
        assert_eq!(landed.header("x-other").as_deref(), Some("kept"));
    }

    #[tokio::test]
    async fn local_destinations_bypass_the_proxy_and_public_hosts_use_it() {
        // A "proxy" that refuses every CONNECT.
        let (proxy_port, proxy_log) = serve(|_, _| reply("403 Forbidden", &[], "")).await;
        let (port, log) = serve(|_, _| reply("200 OK", &[], "direct")).await;
        let proxy = reqwest::Proxy::all(format!("http://127.0.0.1:{proxy_port}")).unwrap();
        let client = ReqwestClient::build("test", Some(proxy)).unwrap();

        let response = client.send(get(format!("http://127.0.0.1:{port}/"))).await.unwrap();
        assert_eq!(response.body, b"direct");
        assert_eq!(seen(&log).len(), 1);
        assert!(seen(&proxy_log).is_empty());

        assert!(client.send(get("https://models.example.test/v1/models".into())).await.is_err());
        let proxied = seen(&proxy_log);
        assert_eq!(proxied.len(), 1);
        assert!(proxied[0].request_line().starts_with("CONNECT models.example.test:443 "), "{}", proxied[0].head);
    }

    #[test]
    fn local_destinations() {
        let local = |text: &str| is_local_destination(&url::Url::parse(text).unwrap());
        for yes in [
            "http://127.0.0.1:8188",
            "http://localhost:11434",
            "http://LOCALHOST",
            "http://studio.local:8188",
            "http://[::1]:1234",
            "http://192.168.1.5",
            "http://10.1.2.3",
            "http://169.254.0.9",
            "http://[fd00::1]",
        ] {
            assert!(local(yes), "{yes}");
        }
        for no in ["https://api.openai.com", "https://8.8.8.8", "https://local.example.com", "https://[2001:db8::1]"] {
            assert!(!local(no), "{no}");
        }
    }

    #[test]
    fn next_hop_rules() {
        let url = |text: &str| url::Url::parse(text).unwrap();
        let hosted = HostPolicy::Hosted(vec!["api.openai.com".into()]);
        let from = url("https://api.openai.com/v1/images");
        assert_eq!(
            next_hop(308, Some("/v2/images"), &from, &from, &hosted),
            Some(url("https://api.openai.com/v2/images"))
        );
        assert_eq!(next_hop(302, Some("https://cdn.openai.com/x"), &from, &from, &hosted), None);
        assert_eq!(next_hop(302, Some("http://api.openai.com/x"), &from, &from, &hosted), None);
        assert_eq!(next_hop(302, Some("https://api.openai.com:8443/x"), &from, &from, &hosted), None);
        assert_eq!(next_hop(200, Some("/x"), &from, &from, &hosted), None);
        assert_eq!(next_hop(300, Some("/x"), &from, &from, &hosted), None);
        assert_eq!(next_hop(302, None, &from, &from, &hosted), None);

        let user = HostPolicy::UserEndpoint;
        let local = url("https://192.168.1.5/v1");
        // Never https → http, even where the policy would allow plain http.
        assert_eq!(next_hop(302, Some("http://192.168.1.5/v1"), &local, &local, &user), None);
        let plain = url("http://192.168.1.5:8080/v1");
        assert_eq!(
            next_hop(302, Some("https://192.168.1.5/v1"), &plain, &plain, &user),
            Some(url("https://192.168.1.5/v1"))
        );
        // Same host as the previous hop isn't enough: it must be the original's.
        let hop = url("http://192.168.1.6/v1");
        assert_eq!(next_hop(302, Some("/v2"), &hop, &plain, &user), None);
    }
}
