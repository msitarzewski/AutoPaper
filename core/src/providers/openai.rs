//! OpenAI: Responses API (strict json_schema, store: false) for concepts; Images API for images.
//!
//! Text + image. Host api.openai.com only. Key: SecretStore `openai.api_key` (Bearer). Defaults and request/response shapes: docs/research/providers.md §OpenAI. Images come back base64 only; no seed or negative prompt; Standard → quality "medium", High → "high"; refusals (`moderation_blocked`) → `Refused`. Models list: GET /v1/models, filtered to text models / gpt-image models. Timeouts: text 60 s, image 240 s ("up to 2 minutes" per image); a request that runs out of time is `ProviderUnavailable` "timed out", while a connection or DNS failure stays `Offline`.
//!
//! Concepts: `POST /v1/responses` with the composer's schema as `text.format` (`json_schema`, strict) and
//! `store: false`. `gpt-6-luna` runs at `reasoning.effort: "none"`, which keeps `temperature` (so Surprise
//! still varies sampling); other reasoning models run at "low" without `temperature` (the API rejects it
//! unless effort is "none"); GPT-4-family models aren't reasoning models and get `temperature` only. A
//! `refusal` content item → `Refused`; an `incomplete` response with the reason `content_filter` →
//! `Refused`. An answer cut off for another reason, or whose text isn't JSON, is returned as that text (a
//! JSON string) with its usage: it was billed, and the composer finds no candidates in it, so the engine
//! asks again. Without any text it is `InvalidResponse`.
//!
//! Images: `POST /v1/images/generations` with `size: "WxH"`, `output_format: "jpeg"` (faster and smaller
//! than png), `background: "opaque"`, `n: 1`. gpt-image-2 and later take any size within the documented
//! rules (multiples of 16, edges ≤ 3840, aspect 1:3–3:1, 655,360–8,294,400 pixels); the gpt-image-1
//! family and `chatgpt-image-latest` take 1024x1024, 1536x1024 and 1024x1536 only. A size outside the
//! model's rules fails with `InvalidInput` before anything is sent.
//!
//! Errors: `net::error_for_status`, then `moderation_blocked` → `Refused`, and a 429 for exhausted credit
//! (`insufficient_quota`) → `InvalidResponse`, because waiting doesn't fix it (no automatic retries).

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde_json::{Value, json};

use crate::error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason, Result};
use crate::model::{ImageQuality, ModelInfo, ProviderKind};
use crate::net::{self, HostPolicy};
use crate::ports::{HttpClient, HttpMethod, HttpRequest, HttpResponse, SecretStore};

use super::{
    ComposeRequest, ComposeResponse, ImageCapabilities, ImageProvider, ImageRequest, ImageResponse, TextProvider, Usage,
    used_whole_timeout,
};

pub const DEFAULT_TEXT_MODEL: &str = "gpt-6-luna";
pub const DEFAULT_IMAGE_MODEL: &str = "gpt-image-2.5-flare";

const PROVIDER: ProviderKind = ProviderKind::OpenAi;
const HOST: &str = "api.openai.com";
const API: &str = "https://api.openai.com/v1";
const TEXT_TIMEOUT_SECS: u64 = 60;
const IMAGE_TIMEOUT_SECS: u64 = 240;
/// Concepts and the model list are tens of kilobytes; anything near this is wrong.
const TEXT_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Base64 of the image pipeline's 50 MB decode cap (≈ 66.7 MB), plus the JSON around it.
const IMAGE_MAX_RESPONSE_BYTES: usize = 72 * 1024 * 1024;
/// `text.format.name` sent with the composer's schema.
const SCHEMA_NAME: &str = "wallpaper_concepts";
const OUTPUT_FORMAT: &str = "jpeg";
/// "The maximum length is 32000 characters."
const MAX_PROMPT_CHARS: usize = 32_000;

/// gpt-image-2 and later: the desktop sizes in research §1.4.3 (all within the documented rules; above
/// 2560x1440 is "experimental"), plus the portrait 9:16 and 10:16 maxima for rotated displays.
const SIZES: &[(u32, u32)] = &[
    (3840, 2160), // 16:9, the documented maximum
    (2560, 1440),
    (1920, 1088), // 1080p: heights must be multiples of 16
    (3632, 2272), // ~16:10 (3840x2400 is over the pixel cap)
    (2560, 1600),
    (3840, 1648), // ~21:9
    (3440, 1440),
    (2160, 3840), // portrait
    (2272, 3632),
];

/// The gpt-image-1 family and `chatgpt-image-latest`: fixed sizes only.
const LEGACY_SIZES: &[(u32, u32)] = &[(1536, 1024), (1024, 1024), (1024, 1536)];

/// Standard base64, with or without padding.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// OpenAI's hosted API (text and images).
pub struct OpenAi {
    http: Arc<dyn HttpClient>,
    secrets: Arc<dyn SecretStore>,
}

impl OpenAi {
    pub fn new(http: Arc<dyn HttpClient>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { http, secrets }
    }

    /// Sends one JSON request to `API/path` with the key read now, maps errors, and parses the JSON reply.
    async fn call(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<&Value>,
        timeout_secs: u64,
        max_response_bytes: usize,
    ) -> Result<Value> {
        let key = read_key(self.secrets.as_ref())?;
        let mut headers = vec![("Authorization".to_string(), format!("Bearer {key}"))];
        if body.is_some() {
            headers.push(("Content-Type".to_string(), "application/json".to_string()));
        }
        let request = HttpRequest {
            method,
            url: format!("{API}/{path}"),
            policy: HostPolicy::Hosted(vec![HOST.to_string()]),
            headers,
            body: body.map(|json| json.to_string().into_bytes()),
            timeout_secs,
            max_response_bytes,
        };
        let started = tokio::time::Instant::now();
        let response = self
            .http
            .send(request)
            .await
            .map_err(|error| attribute(error, used_whole_timeout(started, timeout_secs)))?;
        if let Some(error) = net::error_for_status(PROVIDER, &response) {
            return Err(refine(error, &response));
        }
        serde_json::from_slice(&response.body).map_err(|_| invalid("OpenAI's response wasn't JSON"))
    }

    /// Every model ID the key can use.
    async fn model_ids(&self) -> Result<Vec<String>> {
        let json = self.call(HttpMethod::Get, "models", None, TEXT_TIMEOUT_SECS, TEXT_MAX_RESPONSE_BYTES).await?;
        let data = json["data"].as_array().ok_or_else(|| invalid("OpenAI's model list had no data"))?;
        Ok(data
            .iter()
            .filter_map(|model| model["id"].as_str())
            .filter(|id| plausible_model_id(id))
            .map(str::to_string)
            .collect())
    }
}

#[async_trait]
impl TextProvider for OpenAi {
    fn kind(&self) -> ProviderKind {
        PROVIDER
    }

    fn check_ready(&self) -> Result<()> {
        read_key(self.secrets.as_ref()).map(|_| ())
    }

    fn default_model(&self) -> &str {
        DEFAULT_TEXT_MODEL
    }

    async fn compose(&self, request: ComposeRequest) -> Result<ComposeResponse> {
        let model = model_or(&request.model, DEFAULT_TEXT_MODEL).to_string();
        let body = responses_body(&model, &request);
        let json =
            self.call(HttpMethod::Post, "responses", Some(&body), TEXT_TIMEOUT_SECS, TEXT_MAX_RESPONSE_BYTES).await?;
        parse_response(&json, &model)
    }

    /// GPT and o-series chat models, minus audio, realtime, search, transcription, TTS, embedding,
    /// moderation, image and completions-only models, and models older than GPT-4o (no structured outputs).
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(model_infos(self.model_ids().await?, is_text_model))
    }
}

#[async_trait]
impl ImageProvider for OpenAi {
    fn kind(&self) -> ProviderKind {
        PROVIDER
    }

    fn check_ready(&self) -> Result<()> {
        read_key(self.secrets.as_ref()).map(|_| ())
    }

    fn default_model(&self) -> &str {
        DEFAULT_IMAGE_MODEL
    }

    fn capabilities(&self, model: &str) -> ImageCapabilities {
        let sizes = if is_legacy_image_model(model_or(model, DEFAULT_IMAGE_MODEL)) { LEGACY_SIZES } else { SIZES };
        ImageCapabilities { sizes: sizes.to_vec(), free_size: None }
    }

    /// `request.seed` is ignored: the Images API has no seed.
    async fn generate(&self, request: ImageRequest) -> Result<ImageResponse> {
        let model = model_or(&request.model, DEFAULT_IMAGE_MODEL).to_string();
        check_size(&model, request.width, request.height)?;
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            return Err(AutoPaperError::invalid_input(InvalidInputReason::Other, "the image prompt is empty"));
        }
        if prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(AutoPaperError::invalid_input(
                InvalidInputReason::Other,
                format!("image prompts for OpenAI can be up to {MAX_PROMPT_CHARS} characters"),
            ));
        }
        let body = json!({
            "model": model,
            "prompt": prompt,
            "size": format!("{}x{}", request.width, request.height),
            "quality": quality(request.quality),
            "output_format": OUTPUT_FORMAT,
            "background": "opaque",
            "n": 1,
        });
        let json = self
            .call(HttpMethod::Post, "images/generations", Some(&body), IMAGE_TIMEOUT_SECS, IMAGE_MAX_RESPONSE_BYTES)
            .await?;
        parse_image(&json, &model)
    }

    /// `gpt-image-*` and `chatgpt-image-*` models.
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(model_infos(self.model_ids().await?, is_image_model))
    }
}

// ── Requests ────────────────────────────────────────────────────────────────────────────────

/// How a text model takes its sampling settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sampling {
    /// Reasoning off (`effort: "none"`), which keeps `temperature`: gpt-6-luna.
    NoReasoning,
    /// A reasoning model that may not support "none" (gpt-6.1-sol, gpt-5, o-series): effort "low",
    /// no `temperature`.
    LowReasoning,
    /// Not a reasoning model (GPT-4 family): `temperature`, no `reasoning`.
    Classic,
}

fn sampling(model: &str) -> Sampling {
    let base = model.strip_prefix("ft:").unwrap_or(model);
    if base.starts_with("gpt-6-luna") {
        Sampling::NoReasoning
    } else if base.starts_with("gpt-4") || base.starts_with("gpt-3.5") || base.starts_with("chatgpt-4o") {
        Sampling::Classic
    } else {
        Sampling::LowReasoning
    }
}

/// The Responses API body: instructions as system + user input, the schema as strict `json_schema`.
fn responses_body(model: &str, request: &ComposeRequest) -> Value {
    let mut body = json!({
        "model": model,
        "store": false,
        "input": [
            { "role": "system", "content": request.system },
            { "role": "user", "content": request.user },
        ],
        "text": {
            "format": { "type": "json_schema", "name": SCHEMA_NAME, "strict": true, "schema": request.schema },
        },
    });
    let sampling = sampling(model);
    match sampling {
        Sampling::NoReasoning => body["reasoning"] = json!({ "effort": "none" }),
        Sampling::LowReasoning => body["reasoning"] = json!({ "effort": "low" }),
        Sampling::Classic => {}
    }
    if sampling != Sampling::LowReasoning
        && let Some(temperature) = temperature(request.temperature)
    {
        body["temperature"] = json!(temperature);
    }
    body
}

/// Clamped to the API's 0–2 and rounded to hundredths (f32 → JSON would otherwise read 0.6800000071…).
fn temperature(value: f32) -> Option<f64> {
    value.is_finite().then(|| (f64::from(value.clamp(0.0, 2.0)) * 100.0).round() / 100.0)
}

fn quality(quality: ImageQuality) -> &'static str {
    match quality {
        ImageQuality::Standard => "medium",
        ImageQuality::High => "high",
    }
}

fn is_legacy_image_model(model: &str) -> bool {
    model.starts_with("gpt-image-1") || model.starts_with("chatgpt-image")
}

/// The documented gpt-image-2 / 2.5 rules: "Width and height must be multiples of 16, the aspect ratio must
/// be between 1:3 and 3:1, and neither edge may exceed 3840 pixels. The total pixel count must be between
/// 655,360 and 8,294,400."
fn valid_flexible_size(width: u32, height: u32) -> bool {
    let (long, short) = (u64::from(width.max(height)), u64::from(width.min(height)));
    width.is_multiple_of(16)
        && height.is_multiple_of(16)
        && short > 0
        && long <= 3840
        && long <= 3 * short
        && (655_360..=8_294_400).contains(&(long * short))
}

fn check_size(model: &str, width: u32, height: u32) -> Result<()> {
    let ok = if is_legacy_image_model(model) {
        LEGACY_SIZES.contains(&(width, height))
    } else {
        valid_flexible_size(width, height)
    };
    if ok {
        Ok(())
    } else {
        Err(AutoPaperError::invalid_input(InvalidInputReason::Other, format!("{model} can't make a {width}×{height} image")))
    }
}

// ── Responses ───────────────────────────────────────────────────────────────────────────────

/// The structured output from a Responses API reply: the `output_text` of its `message` items (other
/// items, such as `reasoning`, are skipped), parsed as JSON; text that doesn't parse (or was cut off) is
/// passed on as a JSON string.
fn parse_response(json: &Value, requested_model: &str) -> Result<ComposeResponse> {
    let mut cut_off = None;
    match json["status"].as_str() {
        None | Some("completed") => {}
        Some("incomplete") => {
            let reason = json.pointer("/incomplete_details/reason").and_then(Value::as_str).unwrap_or("unknown");
            if reason == "content_filter" {
                return Err(AutoPaperError::Refused { provider: PROVIDER });
            }
            cut_off = Some(label(reason));
        }
        Some(other) => return Err(invalid(format!("OpenAI's response ended as {}", label(other)))),
    }
    let mut text = String::new();
    let messages = json["output"].as_array().into_iter().flatten().filter(|item| item["type"] == "message");
    for part in messages.flat_map(|item| item["content"].as_array().into_iter().flatten()) {
        match part["type"].as_str() {
            Some("output_text") => text.push_str(part["text"].as_str().unwrap_or_default()),
            Some("refusal") => return Err(AutoPaperError::Refused { provider: PROVIDER }),
            _ => {}
        }
    }
    if text.trim().is_empty() {
        return Err(match cut_off {
            Some(reason) => invalid(format!("OpenAI's answer was cut off ({reason})")),
            None => invalid("OpenAI's response had no text"),
        });
    }
    let output = serde_json::from_str(&text).unwrap_or_else(|_| {
        tracing::warn!(cut_off = cut_off.as_deref(), "OpenAI's concepts weren't valid JSON");
        Value::String(text.clone())
    });
    let tokens = |pointer: &str| json.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
    let usage = Usage { input_tokens: tokens("/usage/input_tokens"), output_tokens: tokens("/usage/output_tokens") };
    let model = json["model"].as_str().filter(|id| plausible_model_id(id)).unwrap_or(requested_model);
    Ok(ComposeResponse { output, usage, model: model.to_string() })
}

fn parse_image(json: &Value, model: &str) -> Result<ImageResponse> {
    let encoded = json
        .pointer("/data/0/b64_json")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("OpenAI's response had no image"))?;
    let bytes = BASE64.decode(encoded.trim()).map_err(|_| invalid("OpenAI's image wasn't valid base64"))?;
    if bytes.is_empty() {
        return Err(invalid("OpenAI's image was empty"));
    }
    let mime = match json["output_format"].as_str().unwrap_or(OUTPUT_FORMAT) {
        "png" => "image/png",
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    };
    Ok(ImageResponse { bytes, mime: mime.to_string(), model: model.to_string(), reported_cost_microusd: None })
}

// ── Errors ──────────────────────────────────────────────────────────────────────────────────

/// The key, trimmed. Missing or blank → `MissingKey`; anything that couldn't be a key (spaces, control
/// characters, non-ASCII, which also couldn't go in a header) → `InvalidKey` without sending it.
fn read_key(secrets: &dyn SecretStore) -> Result<String> {
    let account = PROVIDER.secret_account().ok_or(AutoPaperError::MissingKey { provider: PROVIDER })?;
    let stored = secrets.get(account.to_string()).unwrap_or_default();
    let key = stored.trim();
    if key.is_empty() {
        return Err(AutoPaperError::MissingKey { provider: PROVIDER });
    }
    if !key.chars().all(|c| c.is_ascii_graphic()) {
        return Err(AutoPaperError::InvalidKey { provider: PROVIDER });
    }
    Ok(key.to_string())
}

/// The HTTP client doesn't know which provider it was talking to, nor whether `Offline` was a timeout: one
/// that came after (nearly) the whole timeout is `ProviderUnavailable` "timed out" (OpenAI is slow or stuck);
/// a connection or DNS failure stays `Offline`.
fn attribute(error: AutoPaperError, timed_out: bool) -> AutoPaperError {
    match error {
        AutoPaperError::Offline if timed_out => {
            AutoPaperError::unavailable(PROVIDER, ProviderUnavailableReason::TimedOut, "timed out")
        }
        error @ AutoPaperError::ProviderUnavailable { .. } => error.for_provider(PROVIDER),
        AutoPaperError::RateLimited { retry_after_secs, .. } => {
            AutoPaperError::RateLimited { provider: PROVIDER, retry_after_secs }
        }
        other => other,
    }
}

/// OpenAI specifics on top of `net::error_for_status`.
fn refine(error: AutoPaperError, response: &HttpResponse) -> AutoPaperError {
    let Ok(body) = serde_json::from_slice::<Value>(&response.body) else {
        return error;
    };
    let field = |name: &str| body.pointer(&format!("/error/{name}")).and_then(Value::as_str).map(str::to_string);
    let code = field("code").unwrap_or_default();
    if code == "moderation_blocked" {
        return AutoPaperError::Refused { provider: PROVIDER };
    }
    let message = field("message").unwrap_or_default().to_ascii_lowercase();
    let out_of_credit = code == "insufficient_quota"
        || field("type").as_deref() == Some("insufficient_quota")
        || message.contains("exceeded your current quota")
        || message.contains("credit balance");
    if response.status == 429 && out_of_credit {
        let detail = net::error_message(&response.body).unwrap_or_else(|| "quota exhausted".to_string());
        return AutoPaperError::InvalidResponse { detail: format!("HTTP 429: {detail}") };
    }
    error
}

fn invalid(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::InvalidResponse { detail: detail.into() }
}

/// An untrusted enum-like value, safe to put in an error detail.
fn label(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect()
}

// ── Models ──────────────────────────────────────────────────────────────────────────────────

fn model_or<'a>(model: &'a str, default: &'a str) -> &'a str {
    match model.trim() {
        "" => default,
        model => model,
    }
}

/// Model IDs from the API are untrusted: short, and only the characters IDs use (fine-tunes have colons).
fn plausible_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
}

fn is_image_model(id: &str) -> bool {
    id.starts_with("gpt-image-") || id.starts_with("chatgpt-image")
}

fn is_text_model(id: &str) -> bool {
    const NOT_TEXT: [&str; 9] =
        ["image", "audio", "realtime", "transcribe", "tts", "search", "embedding", "moderation", "instruct"];
    let base = id.strip_prefix("ft:").unwrap_or(id);
    let mut chars = base.chars();
    let o_series = chars.next() == Some('o') && chars.next().is_some_and(|c| c.is_ascii_digit());
    let chat = base.starts_with("gpt-") || base.starts_with("chatgpt-") || o_series;
    // Structured outputs start with GPT-4o: no GPT-3.5, GPT-4 or GPT-4 Turbo.
    let too_old = base.starts_with("gpt-3") || base == "gpt-4" || base.starts_with("gpt-4-");
    chat && !too_old && !NOT_TEXT.iter().any(|word| base.contains(word))
}

/// Filtered, sorted by ID, de-duplicated. OpenAI has no display names, so the ID is shown.
fn model_infos(ids: Vec<String>, keep: fn(&str) -> bool) -> Vec<ModelInfo> {
    let mut ids: Vec<String> = ids.into_iter().filter(|id| keep(id)).collect();
    ids.sort();
    ids.dedup();
    ids.into_iter().map(|id| ModelInfo { display_name: id.clone(), id }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ComposeInputs;
    use crate::testing::{StubHttp, StubSecrets, response};

    const KEY: &str = "sk-test-0123456789abcdefghijklmnop";

    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!("../../tests/fixtures/openai/", $name))
        };
    }

    fn provider(http: &Arc<StubHttp>) -> OpenAi {
        OpenAi::new(http.clone(), Arc::new(StubSecrets::with(&[("openai.api_key", KEY)])))
    }

    fn provider_with_key(http: &Arc<StubHttp>, key: Option<&str>) -> OpenAi {
        let secrets = match key {
            Some(key) => StubSecrets::with(&[("openai.api_key", key)]),
            None => StubSecrets::default(),
        };
        OpenAi::new(http.clone(), Arc::new(secrets))
    }

    fn header<'a>(request: &'a HttpRequest, name: &str) -> Option<&'a str> {
        request.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "candidates": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": { "title": { "type": "string" }, "prompt": { "type": "string" } },
                        "required": ["title", "prompt"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["candidates"],
            "additionalProperties": false
        })
    }

    fn compose_request(model: &str) -> ComposeRequest {
        ComposeRequest {
            model: model.into(),
            system: "You compose desktop wallpaper concepts.".into(),
            user: "Musts: rain. Maybes: ruins.".into(),
            schema: schema(),
            temperature: 0.68,
            inputs: ComposeInputs::default(),
        }
    }

    fn image_request(model: &str, width: u32, height: u32, quality: ImageQuality) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "Ancient stone ruins at night in gentle rain".into(),
            width,
            height,
            quality,
            seed: Some(42),
            progress: None,
            expected_secs: None,
        }
    }

    // ── Text ────────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn compose_sends_the_schema_strict_with_store_false() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/responses", 200, fixture!("responses_compose.json"));
        provider(&http).compose(compose_request("")).await.expect("compose");

        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://api.openai.com/v1/responses");
        assert_eq!(request.policy, HostPolicy::Hosted(vec!["api.openai.com".into()]));
        assert_eq!(header(request, "authorization"), Some(format!("Bearer {KEY}").as_str()));
        assert_eq!(header(request, "content-type"), Some("application/json"));
        assert!(!request.url.contains(KEY));
        assert_eq!(request.timeout_secs, 60);
        assert_eq!(request.max_response_bytes, TEXT_MAX_RESPONSE_BYTES);
        assert_eq!(
            http.json_body(0),
            json!({
                "model": "gpt-6-luna",
                "store": false,
                "input": [
                    { "role": "system", "content": "You compose desktop wallpaper concepts." },
                    { "role": "user", "content": "Musts: rain. Maybes: ruins." }
                ],
                "text": {
                    "format": { "type": "json_schema", "name": "wallpaper_concepts", "strict": true, "schema": schema() }
                },
                "reasoning": { "effort": "none" },
                "temperature": 0.68
            })
        );
    }

    #[tokio::test]
    async fn compose_returns_the_structured_output_usage_and_answering_model() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/responses", 200, fixture!("responses_compose.json"));
        let response = provider(&http).compose(compose_request("gpt-6-luna")).await.expect("compose");

        assert_eq!(response.output["candidates"][0]["title"], "Rain over quiet ruins");
        assert_eq!(response.output["candidates"].as_array().map(Vec::len), Some(1));
        assert_eq!(response.usage, Usage { input_tokens: 1812, output_tokens: 1547 });
        assert_eq!(response.model, "gpt-6-luna-2026-09-22");
    }

    #[tokio::test]
    async fn sampling_settings_follow_the_model() {
        let http = Arc::new(StubHttp::new());
        http.always("/v1/responses", 200, fixture!("responses_compose.json"));
        let openai = provider(&http);

        openai.compose(compose_request("gpt-6.1-sol")).await.expect("sol");
        let sol = http.json_body(0);
        assert_eq!(sol["model"], "gpt-6.1-sol");
        assert_eq!(sol["reasoning"], json!({ "effort": "low" }));
        assert!(sol.get("temperature").is_none(), "{sol}");

        openai.compose(compose_request("gpt-4o-mini")).await.expect("4o-mini");
        let classic = http.json_body(1);
        assert!(classic.get("reasoning").is_none(), "{classic}");
        assert_eq!(classic["temperature"], 0.68);

        let mut wild = compose_request("gpt-6-luna-2026-09-22");
        wild.temperature = 3.5;
        openai.compose(wild).await.expect("clamped");
        assert_eq!(http.json_body(2)["temperature"], 2.0);

        let mut broken = compose_request("gpt-6-luna");
        broken.temperature = f32::NAN;
        openai.compose(broken).await.expect("nan");
        assert!(http.json_body(3).get("temperature").is_none());
        assert_eq!(http.json_body(3)["reasoning"], json!({ "effort": "none" }));
    }

    #[tokio::test]
    async fn compose_skips_reasoning_items() {
        let http = Arc::new(StubHttp::new());
        let body = json!({
            "object": "response",
            "status": "completed",
            "model": "gpt-6.1-sol",
            "output": [
                { "type": "reasoning", "id": "rs_1", "summary": [] },
                { "type": "message", "role": "assistant", "status": "completed",
                  "content": [ { "type": "output_text", "annotations": [], "text": "{\"candidates\":[]}" } ] }
            ]
        });
        http.once("/v1/responses", 200, body.to_string());
        let response = provider(&http).compose(compose_request("gpt-6.1-sol")).await.expect("compose");
        assert_eq!(response.output, json!({ "candidates": [] }));
        assert_eq!(response.usage, Usage::default());
    }

    #[tokio::test]
    async fn compose_maps_refusals_and_bad_answers() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/responses", 200, fixture!("responses_refusal.json"));
        let refused = provider(&http).compose(compose_request("")).await;
        assert!(matches!(refused, Err(AutoPaperError::Refused { provider: ProviderKind::OpenAi })), "{refused:?}");

        // Cut off, or not JSON: passed on as text (it was billed); the composer finds no candidates in it.
        http.once("/v1/responses", 200, fixture!("responses_incomplete.json"));
        let cut = provider(&http).compose(compose_request("")).await.expect("cut off, as text");
        assert_eq!(cut.output, json!("{\"candidates\":[{\"title\":\"Rain over"));
        let empty = json!({ "status": "incomplete", "incomplete_details": { "reason": "max_output_tokens" }, "output": [] });
        http.once("/v1/responses", 200, empty.to_string());
        match provider(&http).compose(compose_request("")).await {
            Err(AutoPaperError::InvalidResponse { detail }) => {
                assert!(detail.contains("max_output_tokens"), "{detail}")
            }
            other => panic!("{other:?}"),
        }

        let filtered =
            json!({ "status": "incomplete", "incomplete_details": { "reason": "content_filter" }, "output": [] });
        http.once("/v1/responses", 200, filtered.to_string());
        assert!(matches!(provider(&http).compose(compose_request("")).await, Err(AutoPaperError::Refused { .. })));

        let not_json = json!({ "status": "completed", "output": [ { "type": "message",
            "content": [ { "type": "output_text", "text": "Here are four concepts: rain over ruins" } ] } ],
            "usage": { "input_tokens": 12, "output_tokens": 8 } });
        http.once("/v1/responses", 200, not_json.to_string());
        let prose = provider(&http).compose(compose_request("")).await.expect("not JSON, as text");
        assert_eq!(prose.output, json!("Here are four concepts: rain over ruins"));
        assert_eq!(prose.usage, Usage { input_tokens: 12, output_tokens: 8 }, "still billed");

        http.once("/v1/responses", 200, r#"{"status":"completed","output":[]}"#);
        assert!(matches!(
            provider(&http).compose(compose_request("")).await,
            Err(AutoPaperError::InvalidResponse { .. })
        ));

        http.once("/v1/responses", 200, "<html>gateway</html>");
        assert!(matches!(
            provider(&http).compose(compose_request("")).await,
            Err(AutoPaperError::InvalidResponse { .. })
        ));
    }

    #[tokio::test]
    async fn missing_or_unusable_keys_fail_before_sending() {
        let http = Arc::new(StubHttp::new());
        http.always("api.openai.com", 200, fixture!("responses_compose.json"));
        for key in [None, Some(""), Some("   ")] {
            let result = provider_with_key(&http, key).compose(compose_request("")).await;
            assert!(matches!(result, Err(AutoPaperError::MissingKey { provider: ProviderKind::OpenAi })), "{result:?}");
            let result = ImageProvider::list_models(&provider_with_key(&http, key)).await;
            assert!(matches!(result, Err(AutoPaperError::MissingKey { .. })), "{result:?}");
        }
        let result = provider_with_key(&http, Some("sk-abc\r\nX-Evil: 1")).compose(compose_request("")).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidKey { provider: ProviderKind::OpenAi })), "{result:?}");
        assert!(http.requests().is_empty());

        provider_with_key(&http, Some(&format!("  {KEY}\n"))).compose(compose_request("")).await.expect("trimmed");
        assert_eq!(header(&http.requests()[0], "authorization"), Some(format!("Bearer {KEY}").as_str()));
    }

    #[tokio::test]
    async fn maps_http_errors() {
        let http = Arc::new(StubHttp::new());
        let openai = provider(&http);

        let unauthorized = r#"{"error":{"message":"Incorrect API key provided: sk-test-0123456789abcdefghijklmnop.","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        http.once("/v1/responses", 401, unauthorized);
        let result = openai.compose(compose_request("")).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidKey { provider: ProviderKind::OpenAi })), "{result:?}");

        let mut limited = response(
            429,
            r#"{"error":{"message":"Rate limit reached for requests","type":"requests","code":"rate_limit_exceeded"}}"#,
        );
        limited.headers.push(("Retry-After".into(), "17".into()));
        http.once_response("/v1/responses", limited);
        let result = openai.compose(compose_request("")).await;
        assert!(
            matches!(result, Err(AutoPaperError::RateLimited { provider: ProviderKind::OpenAi, retry_after_secs: 17 })),
            "{result:?}"
        );

        http.once("/v1/responses", 503, "upstream connect error");
        let result = openai.compose(compose_request("")).await;
        assert!(matches!(result, Err(AutoPaperError::ProviderUnavailable { provider: ProviderKind::OpenAi, .. })));

        let no_credit = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","param":null,"code":"insufficient_quota"}}"#;
        http.once("/v1/responses", 429, no_credit);
        match openai.compose(compose_request("")).await {
            Err(error @ AutoPaperError::InvalidResponse { .. }) => {
                assert!(!error.is_transient());
                assert!(error.to_string().contains("exceeded your current quota"), "{error}");
            }
            other => panic!("{other:?}"),
        }

        let bad = format!(
            r#"{{"error":{{"message":"Invalid value for 'size' with key {KEY}","type":"invalid_request_error"}}}}"#
        );
        http.once("/v1/images/generations", 400, bad);
        match openai.generate(image_request("", 3840, 2160, ImageQuality::High)).await {
            Err(AutoPaperError::InvalidResponse { detail }) => {
                assert!(detail.starts_with("HTTP 400: Invalid value for 'size'"), "{detail}");
                assert!(!detail.contains(KEY), "{detail}");
                assert!(!detail.contains("Ancient stone ruins"), "{detail}");
            }
            other => panic!("{other:?}"),
        }

        http.fail_once("/v1/responses", || {
            AutoPaperError::unavailable(ProviderKind::Demo, ProviderUnavailableReason::TimedOut, "timed out")
        });
        match openai.compose(compose_request("")).await {
            Err(AutoPaperError::ProviderUnavailable { provider, reason, detail }) => {
                assert_eq!(provider, ProviderKind::OpenAi);
                assert_eq!(reason, ProviderUnavailableReason::TimedOut, "the client's reason is kept");
                assert_eq!(detail, "timed out");
            }
            other => panic!("{other:?}"),
        }

        http.fail_once("/v1/responses", || AutoPaperError::Offline);
        assert!(matches!(openai.compose(compose_request("")).await, Err(AutoPaperError::Offline)));
    }

    // ── Images ──────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn generate_sends_size_quality_and_format() {
        let http = Arc::new(StubHttp::new());
        http.always("/v1/images/generations", 200, fixture!("images_generations.json"));
        let openai = provider(&http);
        openai.generate(image_request("", 3840, 2160, ImageQuality::High)).await.expect("high");
        openai
            .generate(image_request("gpt-image-2.5-sunburst", 3632, 2272, ImageQuality::Standard))
            .await
            .expect("standard");

        let requests = http.requests();
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, "https://api.openai.com/v1/images/generations");
        assert_eq!(request.policy, HostPolicy::Hosted(vec!["api.openai.com".into()]));
        assert_eq!(header(request, "authorization"), Some(format!("Bearer {KEY}").as_str()));
        assert!(!request.url.contains(KEY));
        assert_eq!(request.timeout_secs, 240);
        assert_eq!(request.max_response_bytes, IMAGE_MAX_RESPONSE_BYTES);
        assert_eq!(
            http.json_body(0),
            json!({
                "model": "gpt-image-2.5-flare",
                "prompt": "Ancient stone ruins at night in gentle rain",
                "size": "3840x2160",
                "quality": "high",
                "output_format": "jpeg",
                "background": "opaque",
                "n": 1
            })
        );
        let standard = http.json_body(1);
        assert_eq!(standard["model"], "gpt-image-2.5-sunburst");
        assert_eq!(standard["size"], "3632x2272");
        assert_eq!(standard["quality"], "medium");
    }

    #[tokio::test]
    async fn generate_decodes_the_base64_image() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/images/generations", 200, fixture!("images_generations.json"));
        let image = provider(&http).generate(image_request("", 3840, 2160, ImageQuality::High)).await.expect("image");

        assert_eq!(image.mime, "image/jpeg");
        assert_eq!(image.model, "gpt-image-2.5-flare");
        assert_eq!(image.reported_cost_microusd, None);
        assert_eq!(image.bytes.len(), 287);
        assert!(image.bytes.starts_with(&[0xFF, 0xD8, 0xFF]));
        let decoded = image::load_from_memory(&image.bytes).expect("valid JPEG");
        assert_eq!((decoded.width(), decoded.height()), (8, 8));
    }

    #[tokio::test]
    async fn generate_maps_moderation_blocks_and_bad_images() {
        let http = Arc::new(StubHttp::new());
        http.once("/v1/images/generations", 400, fixture!("images_moderation_blocked.json"));
        let result = provider(&http).generate(image_request("", 3840, 2160, ImageQuality::High)).await;
        assert!(matches!(result, Err(AutoPaperError::Refused { provider: ProviderKind::OpenAi })), "{result:?}");

        for body in [
            r#"{"created":1,"data":[]}"#,
            r#"{"created":1,"data":[{"b64_json":"%%% not base64 %%%"}]}"#,
            r#"{"data":[{"b64_json":""}]}"#,
        ] {
            http.once("/v1/images/generations", 200, body);
            let result = provider(&http).generate(image_request("", 3840, 2160, ImageQuality::High)).await;
            assert!(matches!(result, Err(AutoPaperError::InvalidResponse { .. })), "{body}: {result:?}");
        }
    }

    #[tokio::test]
    async fn generate_rejects_sizes_and_prompts_the_model_cannot_take() {
        let http = Arc::new(StubHttp::new());
        let openai = provider(&http);
        for (model, width, height) in [
            ("", 4000, 2250), // edge over 3840
            ("", 3840, 2400), // over 8,294,400 pixels
            ("", 1000, 1000), // not multiples of 16
            ("", 3840, 1072), // wider than 3:1
            ("", 512, 512),   // under 655,360 pixels
            ("", 0, 0),
            ("gpt-image-1", 3840, 2160), // fixed sizes only
            ("chatgpt-image-latest", 2560, 1440),
        ] {
            let result = openai.generate(image_request(model, width, height, ImageQuality::High)).await;
            assert!(matches!(result, Err(AutoPaperError::InvalidInput { .. })), "{model} {width}x{height}: {result:?}");
        }
        let mut empty = image_request("", 3840, 2160, ImageQuality::High);
        empty.prompt = "  ".into();
        assert!(matches!(openai.generate(empty).await, Err(AutoPaperError::InvalidInput { .. })));
        let mut long = image_request("", 3840, 2160, ImageQuality::High);
        long.prompt = "a".repeat(MAX_PROMPT_CHARS + 1);
        assert!(matches!(openai.generate(long).await, Err(AutoPaperError::InvalidInput { .. })));
        assert!(http.requests().is_empty());

        http.once("/v1/images/generations", 200, fixture!("images_generations.json"));
        openai.generate(image_request("gpt-image-1", 1536, 1024, ImageQuality::Standard)).await.expect("legacy size");
    }

    #[test]
    fn capabilities_list_the_documented_sizes() {
        let openai = provider(&Arc::new(StubHttp::new()));
        let caps = openai.capabilities("");
        assert!(caps.free_size.is_none());
        for size in [(3840, 2160), (3840, 1648), (3632, 2272), (2560, 1440), (3440, 1440), (1920, 1088)] {
            assert!(caps.sizes.contains(&size), "{size:?}");
        }
        assert!(caps.sizes.iter().all(|&(w, h)| valid_flexible_size(w, h)), "{:?}", caps.sizes);
        assert_eq!(caps.sizes[0], (3840, 2160));
        assert_eq!(openai.capabilities("gpt-image-2").sizes, caps.sizes);
        assert_eq!(openai.capabilities("gpt-image-1.5").sizes, vec![(1536, 1024), (1024, 1024), (1024, 1536)]);
        assert_eq!(openai.capabilities("chatgpt-image-latest").sizes, LEGACY_SIZES.to_vec());
        assert!(!valid_flexible_size(3840, 2400));
    }

    // ── Models ──────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_models_filters_text_and_image_models() {
        let http = Arc::new(StubHttp::new());
        http.always("/v1/models", 200, fixture!("models_list.json"));
        let openai = provider(&http);

        let text: Vec<String> =
            TextProvider::list_models(&openai).await.expect("text").into_iter().map(|m| m.id).collect();
        assert_eq!(
            text,
            [
                "ft:gpt-4o-mini:acme::AbC123",
                "gpt-4o-mini",
                "gpt-6-luna",
                "gpt-6-luna-2026-09-22",
                "gpt-6.1-sol",
                "o4-mini"
            ]
        );
        let images = ImageProvider::list_models(&openai).await.expect("images");
        let ids: Vec<&str> = images.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["chatgpt-image-latest", "gpt-image-1", "gpt-image-2.5-flare", "gpt-image-2.5-sunburst"]);
        assert!(images.iter().all(|m| m.display_name == m.id));

        let request = &http.requests()[0];
        assert_eq!(request.method, HttpMethod::Get);
        assert_eq!(request.url, "https://api.openai.com/v1/models");
        assert!(request.body.is_none());
        assert_eq!(header(request, "authorization"), Some(format!("Bearer {KEY}").as_str()));
        assert_eq!(header(request, "content-type"), None);

        let no_data = Arc::new(StubHttp::new());
        no_data.once("/v1/models", 200, r#"{"object":"list"}"#);
        let result = TextProvider::list_models(&provider(&no_data)).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidResponse { .. })), "{result:?}");
    }

    #[test]
    fn kinds_and_defaults() {
        let openai = provider(&Arc::new(StubHttp::new()));
        assert_eq!(TextProvider::kind(&openai), ProviderKind::OpenAi);
        assert_eq!(ImageProvider::kind(&openai), ProviderKind::OpenAi);
        assert_eq!(TextProvider::default_model(&openai), "gpt-6-luna");
        assert_eq!(ImageProvider::default_model(&openai), "gpt-image-2.5-flare");
    }

    /// `OPENAI_API_KEY=… cargo test -p autopaper-core --lib providers::openai -- --ignored`. Lists models and
    /// composes one tiny structured answer (well under a cent); makes no image.
    #[tokio::test]
    #[ignore = "live: calls api.openai.com with OPENAI_API_KEY"]
    async fn live_smoke() {
        let Some(key) = std::env::var("OPENAI_API_KEY").ok().filter(|key| !key.trim().is_empty()) else {
            eprintln!("OPENAI_API_KEY isn't set; skipping the live OpenAI smoke test");
            return;
        };
        let http = Arc::new(crate::net::ReqwestClient::new("live-test").expect("http client"));
        let openai = OpenAi::new(http, Arc::new(StubSecrets::with(&[("openai.api_key", key.trim())])));

        let models = TextProvider::list_models(&openai).await.expect("list models");
        assert!(models.iter().any(|m| m.id.starts_with("gpt-")), "{models:?}");

        let request = ComposeRequest {
            model: String::new(),
            system: "Answer with one English word for a calm colour.".into(),
            user: "One word, please.".into(),
            schema: json!({
                "type": "object",
                "properties": { "word": { "type": "string" } },
                "required": ["word"],
                "additionalProperties": false
            }),
            temperature: 0.4,
            inputs: ComposeInputs::default(),
        };
        let response = openai.compose(request).await.expect("compose");
        assert!(response.output["word"].is_string(), "{}", response.output);
        assert!(response.usage.output_tokens > 0);
        assert!(response.model.starts_with("gpt-6-luna"), "{}", response.model);
    }

    // ── Timeouts ────────────────────────────────────────────────────────────────────────────

    /// Like the real client: fails with `Offline` after `secs` (the request's whole timeout when `None`).
    struct FailsAfter(Option<u64>);

    #[async_trait]
    impl HttpClient for FailsAfter {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
            tokio::time::sleep(std::time::Duration::from_secs(self.0.unwrap_or(request.timeout_secs))).await;
            Err(AutoPaperError::Offline)
        }
    }

    fn assert_timed_out<T: std::fmt::Debug>(result: Result<T>) {
        match result {
            Err(AutoPaperError::ProviderUnavailable { provider, reason, detail }) => {
                assert_eq!((provider, reason, detail.as_str()), (PROVIDER, ProviderUnavailableReason::TimedOut, "timed out"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_that_runs_out_of_time_is_unavailable_not_offline() {
        let secrets: Arc<dyn SecretStore> = Arc::new(StubSecrets::with(&[("openai.api_key", KEY)]));
        let stalled = OpenAi::new(Arc::new(FailsAfter(None)), secrets.clone());
        assert_timed_out(stalled.compose(compose_request("")).await);
        assert_timed_out(stalled.generate(image_request("", 3840, 2160, ImageQuality::High)).await);
        assert_timed_out(TextProvider::list_models(&stalled).await);

        // A connection or DNS failure, or a connect timeout (10 s), well before the request's timeout: offline.
        for secs in [0, 10] {
            let unreachable = OpenAi::new(Arc::new(FailsAfter(Some(secs))), secrets.clone());
            let result = unreachable.compose(compose_request("")).await;
            assert!(matches!(result, Err(AutoPaperError::Offline)), "{secs} s: {result:?}");
            let result = unreachable.generate(image_request("", 3840, 2160, ImageQuality::High)).await;
            assert!(matches!(result, Err(AutoPaperError::Offline)), "{secs} s: {result:?}");
        }
    }
}
