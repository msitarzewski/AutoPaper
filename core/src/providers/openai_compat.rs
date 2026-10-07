//! OpenAI-compatible servers (LM Studio, LocalAI, stable-diffusion.cpp's sd-server, hosted services):
//! `/v1/chat/completions` with `response_format: {type: "json_schema", …}` for concepts and
//! `/v1/images/generations` (`response_format: "b64_json"`, falling back to downloading a returned URL
//! from the server's own origin only: same scheme, host and port) for images; `/v1/models` for the list.
//!
//! Base URL required (the person's; `HostPolicy::UserEndpoint`). An address with no path gets `/v1`
//! appended; one with a path is used as the API base as given (so `…/v1`, `…/api/v1` and `…/openai` all
//! work). Optional key as Bearer (no header when absent), from the SecretStore account
//! `model::secret_account_for` gives this server (`openai_compatible.api_key@<origin>`): a key belongs to the
//! server it was entered for and is never sent to another address. An
//! empty model means the first listed model that isn't an embedding model (concepts) or the server's
//! own choice (images). Sizes: free-form (step 64, max side 2048, max 4.2 MP) unless the server rejects
//! them. Timeouts: text 180 s, image 300 s; a request that used its whole timeout → `ProviderUnavailable`.
//! An answer that was cut short or isn't JSON is returned as that text (a JSON string): the composer finds
//! no candidates in it, so the engine asks again; an empty answer is `InvalidResponse`.
//! Nothing answering at a local address (loopback, private, link-local, `.local`) means the server isn't
//! running (`ProviderUnavailable`), not `Offline`.
//! Shapes: `core/tests/fixtures/openai_compat/`.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;

use crate::error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason, Result};
use crate::model::{ModelInfo, ProviderKind};
use crate::net::{HostPolicy, check_url, error_for_status};
use crate::ports::{HttpClient, HttpMethod, HttpRequest, HttpResponse, SecretStore};

use super::{
    ComposeRequest, ComposeResponse, FreeSize, ImageCapabilities, ImageProvider, ImageRequest, ImageResponse, TextProvider, Usage,
    used_whole_timeout,
};

const KIND: ProviderKind = ProviderKind::OpenAiCompatible;
const TEXT_TIMEOUT_SECS: u64 = 180;
const IMAGE_TIMEOUT_SECS: u64 = 300;
const LIST_TIMEOUT_SECS: u64 = 20;
const DOWNLOAD_TIMEOUT_SECS: u64 = 120;
const MAX_TEXT_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// Base64 of a 50 MB image plus the JSON around it.
const MAX_IMAGE_RESPONSE_BYTES: usize = 72 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: usize = 50 * 1024 * 1024;
const MAX_LIST_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// The structured-output schema's name (OpenAI requires one; servers that ignore it don't mind).
const SCHEMA_NAME: &str = "wallpaper_concepts";

pub struct OpenAiCompatible {
    http: Arc<dyn HttpClient>,
    secrets: Arc<dyn SecretStore>,
    base_url: String,
}

impl OpenAiCompatible {
    pub fn new(http: Arc<dyn HttpClient>, secrets: Arc<dyn SecretStore>, base_url: String) -> Self {
        Self { http, secrets, base_url }
    }

    /// The API base every path hangs off, e.g. "http://127.0.0.1:1234/v1".
    fn api_base(&self) -> String {
        api_base(&self.base_url)
    }

    fn headers(&self, json_body: bool) -> Vec<(String, String)> {
        let mut headers = vec![("accept".to_string(), "application/json".to_string())];
        if json_body {
            headers.push(("content-type".to_string(), "application/json".to_string()));
        }
        let selection = crate::model::ProviderSelection { kind: KIND, model: String::new(), base_url: Some(self.base_url.clone()) };
        let key = crate::model::secret_account_for(selection)
            .and_then(|account| self.secrets.get(account))
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());
        if let Some(key) = key {
            headers.push(("authorization".to_string(), format!("Bearer {key}")));
        }
        headers
    }

    /// Sends one request and maps transport failures and non-2xx statuses to errors.
    async fn send(
        &self,
        method: HttpMethod,
        url: String,
        body: Option<&serde_json::Value>,
        timeout_secs: u64,
        max_response_bytes: usize,
    ) -> Result<HttpResponse> {
        let request = HttpRequest {
            method,
            url,
            policy: HostPolicy::UserEndpoint,
            headers: self.headers(body.is_some()),
            body: body.map(serde_json::Value::to_string).map(String::into_bytes),
            timeout_secs,
            max_response_bytes,
        };
        let started = tokio::time::Instant::now();
        let response = self.http.send(request).await.map_err(|error| {
            let timed_out = used_whole_timeout(started, timeout_secs);
            self.transport_error(error, timed_out.then_some(timeout_secs))
        })?;
        match error_for_status(KIND, &response) {
            Some(error) => Err(error),
            None => Ok(response),
        }
    }

    async fn send_json<T: serde::de::DeserializeOwned>(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<&serde_json::Value>,
        timeout_secs: u64,
        max_response_bytes: usize,
    ) -> Result<T> {
        let url = format!("{}{path}", self.api_base());
        let response = self.send(method, url, body, timeout_secs, max_response_bytes).await?;
        serde_json::from_slice(&response.body)
            .map_err(|_| AutoPaperError::InvalidResponse { detail: format!("the server's answer to {path} wasn't the expected JSON") })
    }

    /// `Offline` after the whole timeout means the server was too slow; from a local address otherwise, it
    /// isn't running; a hosted one may really be offline.
    fn transport_error(&self, error: AutoPaperError, timed_out_after: Option<u64>) -> AutoPaperError {
        match (error, timed_out_after) {
            (AutoPaperError::Offline, Some(secs)) => AutoPaperError::unavailable(
                KIND,
                ProviderUnavailableReason::TimedOut,
                format!("the server didn't answer within {secs} s"),
            ),
            (AutoPaperError::Offline, None) if is_local_address(&self.base_url) => AutoPaperError::unavailable(
                KIND,
                ProviderUnavailableReason::NotRunning,
                format!("nothing is answering at {}", self.base_url.trim().trim_end_matches('/')),
            ),
            (error @ AutoPaperError::ProviderUnavailable { .. }, _) => error.for_provider(KIND),
            (other, _) => other,
        }
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        let list: ModelList = self.send_json(HttpMethod::Get, "/models", None, LIST_TIMEOUT_SECS, MAX_LIST_RESPONSE_BYTES).await?;
        Ok(list
            .data
            .into_iter()
            .filter(|model| !model.id.trim().is_empty())
            .map(|model| ModelInfo { display_name: model.id.clone(), id: model.id })
            .collect())
    }

    /// The model to write concepts with: the requested one, else the first listed that isn't for embeddings.
    async fn text_model(&self, requested: &str) -> Result<String> {
        let requested = requested.trim();
        if !requested.is_empty() {
            return Ok(requested.to_string());
        }
        self.models()
            .await?
            .into_iter()
            .map(|model| model.id)
            .find(|id| !id.to_ascii_lowercase().contains("embed"))
            .ok_or_else(|| AutoPaperError::invalid_input(InvalidInputReason::NoModels, "the server lists no models; choose one in Settings"))
    }

    /// Fetches an image the server returned by URL: only from the server's own origin (scheme, host and
    /// port), so the key is never sent anywhere else, nor over http when the server is https.
    async fn download(&self, url: &str) -> Result<(Vec<u8>, String)> {
        let base = check_url(&self.api_base(), &HostPolicy::UserEndpoint)?;
        let target = if url.starts_with("http://") || url.starts_with("https://") {
            url::Url::parse(url).map_err(|_| invalid("the image address isn't valid"))?
        } else {
            base.join(url).map_err(|_| invalid("the image address isn't valid"))?
        };
        let target = check_url(target.as_str(), &HostPolicy::UserEndpoint).map_err(|_| invalid("the image address isn't allowed"))?;
        if target.origin() != base.origin() {
            return Err(invalid("the server returned an image on another host"));
        }
        let response = self.send(HttpMethod::Get, target.to_string(), None, DOWNLOAD_TIMEOUT_SECS, MAX_DOWNLOAD_BYTES).await?;
        let mime = response.header("content-type").unwrap_or("image/png").to_string();
        Ok((response.body, mime))
    }
}

#[async_trait]
impl TextProvider for OpenAiCompatible {
    fn kind(&self) -> ProviderKind {
        KIND
    }

    fn default_model(&self) -> &str {
        ""
    }

    /// POST /chat/completions with a strict `json_schema` response format; the JSON arrives as a string
    /// in `choices[0].message.content` (fenced or wrapped JSON is tolerated). A `refusal` → `Refused`.
    async fn compose(&self, request: ComposeRequest) -> Result<ComposeResponse> {
        let model = self.text_model(&request.model).await?;
        let mut messages = Vec::with_capacity(2);
        if !request.system.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": request.system }));
        }
        messages.push(json!({ "role": "user", "content": request.user }));
        let body = json!({
            "model": model,
            "messages": messages,
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": SCHEMA_NAME, "strict": true, "schema": request.schema },
            },
            "temperature": request.temperature,
            "stream": false,
        });
        let completion: ChatCompletion =
            self.send_json(HttpMethod::Post, "/chat/completions", Some(&body), TEXT_TIMEOUT_SECS, MAX_TEXT_RESPONSE_BYTES).await?;
        let choice = completion.choices.into_iter().next().ok_or_else(|| invalid("the server returned no answer"))?;
        if choice.message.refusal.as_deref().is_some_and(|refusal| !refusal.trim().is_empty()) {
            return Err(AutoPaperError::Refused { provider: KIND });
        }
        let content = choice.message.content.unwrap_or_default();
        let why = if choice.finish_reason.as_deref() == Some("length") {
            "the model's answer was cut short"
        } else {
            "the model's answer wasn't the JSON that was asked for"
        };
        if content.trim().is_empty() {
            return Err(invalid(why));
        }
        let output = json_in_text(&content).unwrap_or_else(|| {
            tracing::warn!("{why}");
            serde_json::Value::String(content.clone())
        });
        let usage = completion.usage.unwrap_or_default();
        Ok(ComposeResponse {
            output,
            usage: Usage { input_tokens: usage.prompt_tokens, output_tokens: usage.completion_tokens },
            model: completion.model.filter(|m| !m.trim().is_empty()).unwrap_or(model),
        })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.models().await
    }
}

#[async_trait]
impl ImageProvider for OpenAiCompatible {
    fn kind(&self) -> ProviderKind {
        KIND
    }

    fn default_model(&self) -> &str {
        ""
    }

    fn capabilities(&self, _model: &str) -> ImageCapabilities {
        ImageCapabilities { sizes: Vec::new(), free_size: Some(FreeSize { step: 64, max_side: 2048, max_pixels: 2048 * 2048 }) }
    }

    /// POST /images/generations asking for `b64_json`. If the server answers with a URL instead, it is
    /// downloaded only from the server's own host. No seed or quality is sent (servers differ).
    async fn generate(&self, request: ImageRequest) -> Result<ImageResponse> {
        if request.prompt.trim().is_empty() {
            return Err(AutoPaperError::invalid_input(InvalidInputReason::Other, "the image prompt is empty"));
        }
        let model = request.model.trim();
        let mut body = json!({
            "prompt": request.prompt,
            "size": format!("{}x{}", request.width, request.height),
            "n": 1,
            "response_format": "b64_json",
        });
        if !model.is_empty() {
            body["model"] = json!(model);
        }
        let images: ImagesResponse =
            self.send_json(HttpMethod::Post, "/images/generations", Some(&body), IMAGE_TIMEOUT_SECS, MAX_IMAGE_RESPONSE_BYTES).await?;
        let claimed_mime = images.output_format.as_deref().map(mime_for_format).unwrap_or("image/png").to_string();
        let image = images.data.into_iter().next().ok_or_else(|| invalid("the server returned no image"))?;
        let (bytes, mime) = match (image.b64_json, image.url) {
            (Some(data), _) if !data.trim().is_empty() => (decode_base64(&data)?, claimed_mime),
            (_, Some(url)) if url.starts_with("data:") => (decode_base64(&url)?, claimed_mime),
            (_, Some(url)) if !url.trim().is_empty() => self.download(url.trim()).await?,
            _ => return Err(invalid("the server returned no image")),
        };
        if bytes.is_empty() {
            return Err(invalid("the server returned an empty image"));
        }
        Ok(ImageResponse { bytes, mime, model: model.to_string(), reported_cost_microusd: None })
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.models().await
    }
}

/// "http://host:1234" → "http://host:1234/v1"; a base with a path is kept as given (trailing slash dropped).
fn api_base(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    match url::Url::parse(trimmed) {
        Ok(parsed) if matches!(parsed.path(), "" | "/") && parsed.query().is_none() => format!("{trimmed}/v1"),
        _ => trimmed.to_string(),
    }
}

/// Loopback, private, link-local or `.local`, by the network policy's own definition of a local address
/// (plain http is allowed exactly there).
fn is_local_address(base_url: &str) -> bool {
    url::Url::parse(base_url.trim())
        .is_ok_and(|mut parsed| parsed.set_scheme("http").is_ok() && check_url(parsed.as_str(), &HostPolicy::UserEndpoint).is_ok())
}

/// A JSON object or array in a model's text answer: the whole text, else inside a Markdown code fence,
/// else the outermost `{…}`. Local models without grammar-constrained output often wrap their JSON.
pub(super) fn json_in_text(text: &str) -> Option<serde_json::Value> {
    let structured =
        |candidate: &str| serde_json::from_str::<serde_json::Value>(candidate.trim()).ok().filter(|v| v.is_object() || v.is_array());
    let text = text.trim();
    if let Some(value) = structured(text) {
        return Some(value);
    }
    if let Some(fenced) = text.strip_prefix("```") {
        let body = fenced.split_once('\n').map_or("", |(_, rest)| rest);
        let body = body.trim_end().strip_suffix("```").unwrap_or(body);
        if let Some(value) = structured(body) {
            return Some(value);
        }
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start < end).then(|| structured(&text[start..=end])).flatten()
}

/// Standard base64, optionally as a `data:` URL, whitespace ignored.
fn decode_base64(data: &str) -> Result<Vec<u8>> {
    let payload = match data.strip_prefix("data:") {
        Some(rest) => rest.split_once(',').map_or("", |(_, payload)| payload),
        None => data,
    };
    let compact: String = payload.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(compact.as_bytes()).map_err(|_| invalid("the image data couldn't be decoded"))
}

fn mime_for_format(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn invalid(detail: &str) -> AutoPaperError {
    AutoPaperError::InvalidResponse { detail: detail.to_string() }
}

#[derive(Deserialize)]
struct ModelList {
    #[serde(default)]
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
}

#[derive(Deserialize)]
struct ChatCompletion {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<TokenUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Deserialize, Default)]
struct TokenUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[derive(Deserialize)]
struct ImagesResponse {
    #[serde(default)]
    data: Vec<ImageDatum>,
    #[serde(default)]
    output_format: Option<String>,
}

#[derive(Deserialize)]
struct ImageDatum {
    #[serde(default)]
    b64_json: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::providers::ComposeInputs;
    use crate::testing::{StubHttp, StubSecrets};

    const CHAT: &str = include_str!("../../tests/fixtures/openai_compat/chat-completions-response.json");
    const MODELS: &str = include_str!("../../tests/fixtures/openai_compat/models.json");
    const NOT_FOUND: &str = include_str!("../../tests/fixtures/openai_compat/error-model-not-found.json");
    const TOKEN_REQUIRED: &str = include_str!("../../tests/fixtures/openai_compat/error-lmstudio-token-required.json");
    const IMAGES_B64: &str = include_str!("../../tests/fixtures/openai_compat/images-b64-response.json");
    const IMAGES_URL: &str = include_str!("../../tests/fixtures/openai_compat/images-url-response.json");
    const TINY_PNG: &[u8] = include_bytes!("../../tests/fixtures/openai_compat/tiny.png");

    fn provider(http: &Arc<StubHttp>, secrets: StubSecrets, base: &str) -> OpenAiCompatible {
        OpenAiCompatible::new(http.clone(), Arc::new(secrets), base.to_string())
    }

    fn compose_request(model: &str) -> ComposeRequest {
        ComposeRequest {
            model: model.into(),
            system: "You write wallpaper concepts.".into(),
            user: "Must: rain, ruins.".into(),
            schema: json!({ "type": "object", "properties": { "candidates": { "type": "array" } }, "required": ["candidates"] }),
            temperature: 0.56,
            inputs: ComposeInputs::default(),
        }
    }

    fn image_request(model: &str) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "Ancient ruins in the rain at blue hour".into(),
            width: 1792,
            height: 1024,
            quality: crate::model::ImageQuality::Standard,
            seed: None,
            progress: None,
            expected_secs: None,
        }
    }

    fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
        request.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    #[test]
    fn normalises_the_base_url() {
        assert_eq!(api_base("http://127.0.0.1:1234"), "http://127.0.0.1:1234/v1");
        assert_eq!(api_base(" http://127.0.0.1:1234/ "), "http://127.0.0.1:1234/v1");
        assert_eq!(api_base("http://127.0.0.1:1234/v1"), "http://127.0.0.1:1234/v1");
        assert_eq!(api_base("http://127.0.0.1:1234/v1/"), "http://127.0.0.1:1234/v1");
        assert_eq!(api_base("https://openrouter.example/api/v1"), "https://openrouter.example/api/v1");
        assert_eq!(api_base("https://host.example/openai/"), "https://host.example/openai");
    }

    #[test]
    fn local_addresses_follow_the_network_policy() {
        assert!(is_local_address("http://127.0.0.1:1234"));
        assert!(is_local_address("https://192.168.1.20:8443/v1"));
        assert!(is_local_address("http://studio.local:8080"));
        assert!(!is_local_address("https://api.together.example/v1"));
        assert!(!is_local_address("not a url"));
    }

    #[test]
    fn finds_json_in_model_text() {
        assert_eq!(json_in_text(r#" {"a":1} "#), Some(json!({"a":1})));
        assert_eq!(json_in_text("```json\n{\"a\":1}\n```"), Some(json!({"a":1})));
        assert_eq!(json_in_text("Here you go:\n{\"a\":{\"b\":2}}\nEnjoy!"), Some(json!({"a":{"b":2}})));
        assert_eq!(json_in_text(r#"[{"title":"x"}]"#), Some(json!([{"title":"x"}])));
        assert_eq!(json_in_text("\"just a string\""), None);
        assert_eq!(json_in_text("no json } here {"), None);
        assert_eq!(json_in_text(""), None);
    }

    #[tokio::test]
    async fn composes_with_a_strict_json_schema() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/chat/completions", 200, CHAT);
        let request = compose_request("qwen2.5vl:7b");
        let schema = request.schema.clone();
        let response = provider(&http, StubSecrets::default(), "http://127.0.0.1:11434").compose(request).await.unwrap();

        let sent = &http.requests()[0];
        assert_eq!(sent.method, HttpMethod::Post);
        assert_eq!(sent.url, "http://127.0.0.1:11434/v1/chat/completions");
        assert_eq!(sent.policy, HostPolicy::UserEndpoint);
        assert_eq!(sent.timeout_secs, TEXT_TIMEOUT_SECS);
        assert_eq!(header(sent, "authorization"), None, "no key, no header");
        let body = http.json_body(0);
        assert_eq!(body["model"], "qwen2.5vl:7b");
        assert_eq!(body["stream"], false);
        assert!((body["temperature"].as_f64().unwrap() - 0.56).abs() < 1e-6);
        assert_eq!(body["messages"][0], json!({"role": "system", "content": "You write wallpaper concepts."}));
        assert_eq!(body["messages"][1], json!({"role": "user", "content": "Must: rain, ruins."}));
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(body["response_format"]["json_schema"]["name"], SCHEMA_NAME);
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema);

        let candidates = response.output["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0]["title"], "Rainy Ruins");
        assert_eq!(response.usage, Usage { input_tokens: 70, output_tokens: 720 });
        assert_eq!(response.model, "qwen2.5vl:7b");
    }

    #[tokio::test]
    async fn a_key_is_only_sent_to_the_server_it_was_saved_for() {
        // Saved for a hosted service, then the address changed to a machine on the LAN: no key goes there.
        let http = Arc::new(StubHttp::new());
        http.once("/chat/completions", 200, CHAT);
        let secrets = StubSecrets::with(&[("openai_compatible.api_key@https://api.example.com", "hosted-key")]);
        provider(&http, secrets, "http://192.168.1.50:1234/v1").compose(compose_request("m")).await.unwrap();
        assert_eq!(header(&http.requests()[0], "authorization"), None);
    }

    #[test]
    fn key_accounts_name_the_servers_origin() {
        use crate::model::{secret_account_for, ProviderSelection};
        let selection = |kind, base: Option<&str>| ProviderSelection { kind, model: String::new(), base_url: base.map(str::to_string) };
        assert_eq!(
            secret_account_for(selection(KIND, Some("http://127.0.0.1:1234/v1"))).as_deref(),
            Some("openai_compatible.api_key@http://127.0.0.1:1234")
        );
        assert_eq!(
            secret_account_for(selection(KIND, Some("https://api.example.com:443/openai/"))).as_deref(),
            Some("openai_compatible.api_key@https://api.example.com")
        );
        assert_eq!(secret_account_for(selection(KIND, Some("not a url"))), None);
        assert_eq!(secret_account_for(selection(KIND, None)), None);
        assert_eq!(secret_account_for(selection(ProviderKind::OpenAi, None)).as_deref(), Some("openai.api_key"));
        assert_eq!(secret_account_for(selection(ProviderKind::ComfyUi, Some("http://127.0.0.1:8188"))), None);
    }

    #[tokio::test]
    async fn sends_the_key_as_bearer_when_there_is_one() {
        let http = Arc::new(StubHttp::new());
        http.once("/chat/completions", 200, CHAT);
        let secrets = StubSecrets::with(&[("openai_compatible.api_key@http://127.0.0.1:1234", " lm-token-123 ")]);
        provider(&http, secrets, "http://127.0.0.1:1234/v1").compose(compose_request("m")).await.unwrap();
        assert_eq!(header(&http.requests()[0], "authorization"), Some("Bearer lm-token-123"));
    }

    #[tokio::test]
    async fn an_empty_model_uses_the_first_listed_non_embedding_model() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/models", 200, r#"{"object":"list","data":[{"id":"text-embedding-nomic-embed-text-v1.5"},{"id":"qwen3-8b"}]}"#);
        http.once("/v1/chat/completions", 200, CHAT);
        provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request(" ")).await.unwrap();
        assert_eq!(http.requests()[0].method, HttpMethod::Get);
        assert_eq!(http.json_body(1)["model"], "qwen3-8b");

        let empty = Arc::new(StubHttp::new());
        empty.once("/v1/models", 200, r#"{"object":"list","data":[]}"#);
        let error = provider(&empty, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("")).await;
        assert!(matches!(error, Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::NoModels, .. })), "{error:?}");
    }

    #[tokio::test]
    async fn tolerates_fenced_json_and_reports_bad_answers() {
        let fenced = json!({
            "model": "local",
            "choices": [{ "message": { "role": "assistant", "content": "```json\n{\"candidates\":[]}\n```" }, "finish_reason": "stop" }]
        });
        let http = Arc::new(StubHttp::new());
        http.once("/chat/completions", 200, fenced.to_string());
        let ok = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("m")).await.unwrap();
        assert_eq!(ok.output, json!({"candidates": []}));
        assert_eq!(ok.usage, Usage::default());

        // Cut short: passed on as text; the composer finds no candidates in it.
        let cut = json!({ "choices": [{ "message": { "content": "{\"candidates\":[{\"title\":\"Rai" }, "finish_reason": "length" }] });
        http.once("/chat/completions", 200, cut.to_string());
        let cut = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("m")).await.unwrap();
        assert_eq!(cut.output, json!("{\"candidates\":[{\"title\":\"Rai"));
        let empty = json!({ "choices": [{ "message": { "content": "" }, "finish_reason": "length" }] });
        http.once("/chat/completions", 200, empty.to_string());
        match provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("m")).await {
            Err(AutoPaperError::InvalidResponse { detail }) => assert!(detail.contains("cut short"), "{detail}"),
            other => panic!("{other:?}"),
        }

        http.once("/chat/completions", 200, r#"{"choices":[]}"#);
        let none = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("m")).await;
        assert!(matches!(none, Err(AutoPaperError::InvalidResponse { .. })), "{none:?}");

        http.once("/chat/completions", 200, "<html>proxy error</html>");
        let html = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("m")).await;
        assert!(matches!(html, Err(AutoPaperError::InvalidResponse { .. })), "{html:?}");
    }

    #[tokio::test]
    async fn a_refusal_is_refused() {
        let refusal =
            json!({ "choices": [{ "message": { "content": null, "refusal": "I can't help with that." }, "finish_reason": "stop" }] });
        let http = Arc::new(StubHttp::new());
        http.once("/chat/completions", 200, refusal.to_string());
        let result = provider(&http, StubSecrets::default(), "https://api.example.com/v1").compose(compose_request("m")).await;
        assert!(matches!(result, Err(AutoPaperError::Refused { provider: ProviderKind::OpenAiCompatible })), "{result:?}");
    }

    #[tokio::test]
    async fn maps_http_errors() {
        let http = Arc::new(StubHttp::new());
        http.once("/chat/completions", 401, TOKEN_REQUIRED);
        let unauthorised = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("m")).await;
        assert!(matches!(unauthorised, Err(AutoPaperError::InvalidKey { provider: KIND })), "{unauthorised:?}");

        http.once("/chat/completions", 404, NOT_FOUND);
        match provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("no-such-model")).await {
            Err(AutoPaperError::InvalidResponse { detail }) => assert_eq!(detail, "HTTP 404: model 'no-such-model' not found"),
            other => panic!("{other:?}"),
        }

        http.once_response(
            "/chat/completions",
            HttpResponse { status: 429, headers: vec![("retry-after".into(), "12".into())], body: Vec::new() },
        );
        let limited = provider(&http, StubSecrets::default(), "https://api.example.com").compose(compose_request("m")).await;
        assert!(matches!(limited, Err(AutoPaperError::RateLimited { retry_after_secs: 12, .. })), "{limited:?}");
    }

    #[tokio::test]
    async fn nothing_answering_locally_means_unavailable_but_hosted_means_offline() {
        let http = Arc::new(StubHttp::new());
        match provider(&http, StubSecrets::default(), "http://127.0.0.1:1234/").compose(compose_request("m")).await {
            Err(AutoPaperError::ProviderUnavailable { provider, reason, detail }) => {
                assert_eq!(provider, KIND);
                assert_eq!(reason, ProviderUnavailableReason::NotRunning);
                assert_eq!(detail, "nothing is answering at http://127.0.0.1:1234");
            }
            other => panic!("{other:?}"),
        }
        let hosted = provider(&http, StubSecrets::default(), "https://api.example.com/v1").compose(compose_request("m")).await;
        assert!(matches!(hosted, Err(AutoPaperError::Offline)), "{hosted:?}");

        http.fail_once("/chat/completions", || {
            AutoPaperError::unavailable(ProviderKind::Demo, ProviderUnavailableReason::TimedOut, "timed out")
        });
        let timed_out = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").compose(compose_request("m")).await;
        assert!(
            matches!(&timed_out, Err(AutoPaperError::ProviderUnavailable { provider: KIND, detail, .. }) if detail == "timed out"),
            "{timed_out:?}"
        );
    }

    #[tokio::test]
    async fn plain_http_to_a_public_host_is_refused_before_sending() {
        let http = Arc::new(StubHttp::new());
        let result = provider(&http, StubSecrets::default(), "http://example.com").compose(compose_request("m")).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidInput { .. })), "{result:?}");
        assert!(http.requests().is_empty());
    }

    #[tokio::test]
    async fn lists_models() {
        let http = Arc::new(StubHttp::new());
        http.always("/v1/models", 200, MODELS);
        let p = provider(&http, StubSecrets::default(), "http://127.0.0.1:11434");
        let models = TextProvider::list_models(&p).await.unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["ornith:latest", "qwen2.5vl:7b", "nomic-embed-text:latest"]);
        assert_eq!(models[0].display_name, "ornith:latest");
        assert_eq!(ImageProvider::list_models(&p).await.unwrap(), models);
        assert_eq!(http.requests()[0].url, "http://127.0.0.1:11434/v1/models");
        assert_eq!(http.requests()[0].method, HttpMethod::Get);
    }

    #[tokio::test]
    async fn generates_images_as_base64() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/images/generations", 200, IMAGES_B64);
        let secrets = StubSecrets::with(&[("openai_compatible.api_key@http://127.0.0.1:1234", "sd-key")]);
        let image = provider(&http, secrets, "http://127.0.0.1:1234").generate(image_request("flux-schnell")).await.unwrap();
        assert_eq!(image.bytes, TINY_PNG);
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.model, "flux-schnell");
        assert_eq!(image.reported_cost_microusd, None);

        let sent = &http.requests()[0];
        assert_eq!(sent.url, "http://127.0.0.1:1234/v1/images/generations");
        assert_eq!(sent.timeout_secs, IMAGE_TIMEOUT_SECS);
        assert_eq!(header(sent, "authorization"), Some("Bearer sd-key"));
        let body = http.json_body(0);
        assert_eq!(
            body,
            json!({
                "model": "flux-schnell", "prompt": "Ancient ruins in the rain at blue hour",
                "size": "1792x1024", "n": 1, "response_format": "b64_json",
            })
        );
    }

    #[tokio::test]
    async fn an_empty_image_model_is_left_to_the_server() {
        let http = Arc::new(StubHttp::new());
        let data_url = format!(
            r#"{{"data":[{{"url":"data:image/png;base64,{}"}}],"output_format":"webp"}}"#,
            base64::engine::general_purpose::STANDARD.encode(TINY_PNG)
        );
        http.once("/images/generations", 200, data_url);
        let image = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").generate(image_request("")).await.unwrap();
        assert!(http.json_body(0).get("model").is_none());
        assert_eq!(image.bytes, TINY_PNG);
        assert_eq!(image.mime, "image/webp", "the server's claim; the pipeline sniffs the real format");
    }

    #[tokio::test]
    async fn downloads_a_returned_url_only_from_the_same_host() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/images/generations", 200, IMAGES_URL);
        http.once_response(
            "/generated-images/",
            HttpResponse { status: 200, headers: vec![("Content-Type".into(), "image/png".into())], body: TINY_PNG.to_vec() },
        );
        let secrets = StubSecrets::with(&[("openai_compatible.api_key@http://127.0.0.1:8080", "local-key")]);
        let image = provider(&http, secrets, "http://127.0.0.1:8080").generate(image_request("sd")).await.unwrap();
        assert_eq!(image.bytes, TINY_PNG);
        assert_eq!(image.mime, "image/png");
        let download = &http.requests()[1];
        assert_eq!(download.method, HttpMethod::Get);
        assert_eq!(download.url, "http://127.0.0.1:8080/generated-images/b641791239150.png");
        assert_eq!(header(download, "authorization"), Some("Bearer local-key"));

        // A relative URL resolves against the server.
        http.once("/v1/images/generations", 200, r#"{"data":[{"url":"/generated-images/rel.png"}]}"#);
        http.once("/generated-images/rel.png", 200, TINY_PNG.to_vec());
        provider(&http, StubSecrets::default(), "http://127.0.0.1:8080/v1").generate(image_request("sd")).await.unwrap();
        assert_eq!(http.requests()[3].url, "http://127.0.0.1:8080/generated-images/rel.png");

        // Another host, or another port on the same host, is never fetched.
        for elsewhere in ["http://192.168.1.9:8080/generated-images/x.png", "http://127.0.0.1:9999/x.png", "https://cdn.example.com/x.png"]
        {
            let http = Arc::new(StubHttp::new());
            http.once("/v1/images/generations", 200, json!({ "data": [{ "url": elsewhere }] }).to_string());
            let result = provider(&http, StubSecrets::default(), "http://127.0.0.1:8080").generate(image_request("sd")).await;
            assert!(matches!(result, Err(AutoPaperError::InvalidResponse { .. })), "{elsewhere}: {result:?}");
            assert_eq!(http.requests().len(), 1, "{elsewhere} must not be fetched");
        }

        // Nor the same host and port over http when the server is https: the key would travel in clear text.
        let http = Arc::new(StubHttp::new());
        http.once("/v1/images/generations", 200, r#"{"data":[{"url":"http://192.168.1.20:8443/files/out.png"}]}"#);
        let secrets = StubSecrets::with(&[("openai_compatible.api_key@https://192.168.1.20:8443", "secret-key-123")]);
        let result = provider(&http, secrets, "https://192.168.1.20:8443").generate(image_request("sd")).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidResponse { .. })), "{result:?}");
        assert_eq!(http.requests().len(), 1, "an https → http download must not be fetched");
    }

    #[tokio::test]
    async fn rejects_missing_or_broken_image_data() {
        for body in [r#"{"data":[]}"#, r#"{"data":[{}]}"#, r#"{"data":[{"b64_json":"%%%not base64"}]}"#, r#"{"data":[{"b64_json":""}]}"#] {
            let http = Arc::new(StubHttp::new());
            http.once("/images/generations", 200, body);
            let result = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").generate(image_request("m")).await;
            assert!(matches!(result, Err(AutoPaperError::InvalidResponse { .. })), "{body}: {result:?}");
        }
        let http = Arc::new(StubHttp::new());
        http.once("/images/generations", 400, r#"{"error":{"message":"size 1792x1024 is not supported"}}"#);
        match provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").generate(image_request("m")).await {
            Err(AutoPaperError::InvalidResponse { detail }) => assert!(detail.contains("not supported"), "{detail}"),
            other => panic!("{other:?}"),
        }
        let empty_prompt = ImageRequest { prompt: "  ".into(), ..image_request("m") };
        let result = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").generate(empty_prompt).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidInput { .. })), "{result:?}");
    }

    #[test]
    fn offers_free_sizes() {
        let http = Arc::new(StubHttp::new());
        let caps = provider(&http, StubSecrets::default(), "http://127.0.0.1:1234").capabilities("any");
        assert!(caps.sizes.is_empty());
        let free = caps.free_size.unwrap();
        assert_eq!((free.step, free.max_side, free.max_pixels), (64, 2048, 4_194_304));
    }

    /// Like the real client on a timeout: waits out the request's timeout, then reports `Offline`.
    struct Stalled;

    #[async_trait]
    impl HttpClient for Stalled {
        async fn send(&self, request: HttpRequest) -> Result<crate::ports::HttpResponse> {
            tokio::time::sleep(Duration::from_secs(request.timeout_secs)).await;
            Err(AutoPaperError::Offline)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn too_slow_is_unavailable_even_when_hosted() {
        let slow = OpenAiCompatible::new(Arc::new(Stalled), Arc::new(StubSecrets::default()), "https://api.example.com/v1".into());
        match slow.compose(compose_request("m")).await {
            Err(AutoPaperError::ProviderUnavailable { provider: KIND, reason, detail }) => {
                assert_eq!(reason, ProviderUnavailableReason::TimedOut);
                assert_eq!(detail, "the server didn't answer within 180 s");
            }
            other => panic!("{other:?}"),
        }
        match slow.generate(image_request("m")).await {
            Err(AutoPaperError::ProviderUnavailable { detail, .. }) => assert_eq!(detail, "the server didn't answer within 300 s"),
            other => panic!("{other:?}"),
        }
    }
}
