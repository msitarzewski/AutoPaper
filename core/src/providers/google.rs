//! Google Gemini API (AI Studio key): generateContent for concepts (responseSchema) and images (Nano Banana).
//!
//! Text + image. Host generativelanguage.googleapis.com only. Key: SecretStore `google.api_key` in header `x-goog-api-key` (never in the URL). Shapes: docs/research/providers.md §Google. Image size via imageConfig aspectRatio + imageSize "2K"/"4K" (uppercase K; Standard → 2K, High → 4K); no 16:10 (use 3:2 and crop). Blocks (promptFeedback.blockReason, finishReason IMAGE_SAFETY / NO_IMAGE) → `Refused`. No `store` field (see below). Models list: GET /v1beta/models filtered by supportedGenerationMethods.
//!
//! Concepts: `POST /v1beta/models/{model}:generateContent` with `systemInstruction`, the user text, and
//! `generationConfig.responseMimeType: "application/json"` + `responseSchema` (the composer's JSON Schema
//! converted to the OpenAPI subset `responseSchema` takes; see [`response_schema`]). Gemini 3 and later
//! keep their default temperature (Google "strongly recommends" it) and think as little as they can
//! (`thinkingLevel` MINIMAL on Flash models, LOW on Pro); older models get the composer's temperature.
//! Thinking tokens count as output tokens, as they're billed.
//!
//! Images: `POST /v1beta/models/{model}:generateContent`, the version the model list comes from (preview models and
//! Nano Banana 2.1 exist only there; `/v1` answers 404 for them), with
//! `responseModalities: ["IMAGE"]` and `imageConfig`. The requested width × height picks the nearest
//! supported aspect ratio; `quality` picks the resolution (Nano Banana 2 Lite makes 1K only), so the image
//! that comes back can be smaller than requested — [`output_size`] says exactly what it will be. Thought
//! images (`thought: true`) are skipped; the first final `inlineData` image is returned. No seed is sent:
//! the docs promise no reproducible images. Other finish reasons without an image → `InvalidResponse`.
//!
//! `store`: generateContent has no such field and keeps no application state (only the Interactions API
//! stores interactions), so it is not sent. Errors: `net::error_for_status`, then a 400 saying the API key
//! isn't valid → `InvalidKey`. Timeouts: text 60 s, image 240 s; a request that runs out of time is
//! `ProviderUnavailable` "timed out", while a connection or DNS failure stays `Offline`.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde_json::{Map, Value, json};

use crate::error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason, Result};
use crate::model::{ImageQuality, ModelInfo, ProviderKind};
use crate::net::{self, HostPolicy};
use crate::ports::{HttpClient, HttpMethod, HttpRequest, HttpResponse, SecretStore};

use super::{
    ComposeRequest, ComposeResponse, ImageCapabilities, ImageProvider, ImageRequest, ImageResponse, TextProvider, Usage,
    used_whole_timeout,
};

pub const DEFAULT_TEXT_MODEL: &str = "gemini-3.5-flash-lite";
pub const DEFAULT_IMAGE_MODEL: &str = "gemini-3.1-flash-image";

const PROVIDER: ProviderKind = ProviderKind::Google;
const HOST: &str = "generativelanguage.googleapis.com";
const ORIGIN: &str = "https://generativelanguage.googleapis.com";
const TEXT_TIMEOUT_SECS: u64 = 60;
const IMAGE_TIMEOUT_SECS: u64 = 240;
/// Concepts and model-list pages are tens of kilobytes; anything near this is wrong.
const TEXT_MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Base64 of the image pipeline's 50 MB decode cap (≈ 66.7 MB), plus the JSON around it.
const IMAGE_MAX_RESPONSE_BYTES: usize = 72 * 1024 * 1024;
/// The documented maximum page size; one page normally holds every model.
const MODEL_PAGE_SIZE: u32 = 1000;
const MAX_MODEL_PAGES: usize = 10;
const MAX_SCHEMA_DEPTH: usize = 32;

/// Standard base64, with or without padding.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// One `aspectRatio` and the pixels each `imageSize` makes.
struct Ratio {
    name: &'static str,
    k1: (u32, u32),
    k2: (u32, u32),
    k4: (u32, u32),
}

/// Research §2.3.2 (the official table, identical for Nano Banana 2 and Pro). Portrait ratios are the
/// landscape ones transposed; the table lists landscape and square only. No 16:10: use 3:2 and crop.
const RATIOS: &[Ratio] = &[
    Ratio { name: "16:9", k1: (1376, 768), k2: (2752, 1536), k4: (5504, 3072) },
    Ratio { name: "21:9", k1: (1584, 672), k2: (3168, 1344), k4: (6336, 2688) },
    Ratio { name: "3:2", k1: (1264, 848), k2: (2528, 1696), k4: (5056, 3392) },
    Ratio { name: "4:3", k1: (1200, 896), k2: (2400, 1792), k4: (4800, 3584) },
    Ratio { name: "1:1", k1: (1024, 1024), k2: (2048, 2048), k4: (4096, 4096) },
    Ratio { name: "3:4", k1: (896, 1200), k2: (1792, 2400), k4: (3584, 4800) },
    Ratio { name: "2:3", k1: (848, 1264), k2: (1696, 2528), k4: (3392, 5056) },
    Ratio { name: "9:16", k1: (768, 1376), k2: (1536, 2752), k4: (3072, 5504) },
];

/// `imageSize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    K1,
    K2,
    K4,
}

impl Tier {
    fn param(self) -> &'static str {
        match self {
            Tier::K1 => "1K",
            Tier::K2 => "2K",
            Tier::K4 => "4K",
        }
    }
}

impl Ratio {
    fn size(&self, tier: Tier) -> (u32, u32) {
        match tier {
            Tier::K1 => self.k1,
            Tier::K2 => self.k2,
            Tier::K4 => self.k4,
        }
    }
}

/// The Gemini API with an AI Studio key (text and images).
/// Models that refused a `thinkingLevel` ("Thinking level MINIMAL is not supported for this model"), seen live
/// 2026-10-06: asked again without one, and not sent one again while AutoPaper runs (process-wide, because providers
/// are built per wallpaper).
static REFUSES_THINKING_LEVEL: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

pub struct Google {
    http: Arc<dyn HttpClient>,
    secrets: Arc<dyn SecretStore>,
}

impl Google {
    pub fn new(http: Arc<dyn HttpClient>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { http, secrets }
    }

    /// Sends one JSON request with the key (read now) in `x-goog-api-key`, maps errors, parses the reply.
    async fn call(
        &self,
        method: HttpMethod,
        url: String,
        body: Option<&Value>,
        timeout_secs: u64,
        max_response_bytes: usize,
    ) -> Result<Value> {
        let key = read_key(self.secrets.as_ref())?;
        let mut headers = vec![("x-goog-api-key".to_string(), key)];
        if body.is_some() {
            headers.push(("Content-Type".to_string(), "application/json".to_string()));
        }
        let request = HttpRequest {
            method,
            url,
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
        serde_json::from_slice(&response.body).map_err(|_| invalid("Gemini's response wasn't JSON"))
    }

    /// Every model the key can see, following `nextPageToken`.
    async fn models(&self) -> Result<Vec<Listed>> {
        let mut models = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_MODEL_PAGES {
            let mut url = format!("{ORIGIN}/v1beta/models?pageSize={MODEL_PAGE_SIZE}");
            if let Some(token) = &page_token {
                url.push_str("&pageToken=");
                url.extend(url::form_urlencoded::byte_serialize(token.as_bytes()));
            }
            let json = self.call(HttpMethod::Get, url, None, TEXT_TIMEOUT_SECS, TEXT_MAX_RESPONSE_BYTES).await?;
            if !json.is_object() {
                return Err(invalid("Gemini's model list wasn't an object"));
            }
            models.extend(json["models"].as_array().into_iter().flatten().filter_map(Listed::from_json));
            page_token = json["nextPageToken"].as_str().filter(|token| !token.is_empty()).map(str::to_string);
            if page_token.is_none() {
                break;
            }
        }
        Ok(models)
    }
}

#[async_trait]
impl TextProvider for Google {
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
        let model = model_id(&request.model, DEFAULT_TEXT_MODEL)?;
        let mut body = text_body(&model, &request)?;
        let refuses = |model: &str| REFUSES_THINKING_LEVEL.lock().is_ok_and(|set| set.contains(model));
        if refuses(&model) {
            without_thinking_level(&mut body);
        }
        let url = format!("{ORIGIN}/v1beta/models/{model}:generateContent");
        let json = match self.call(HttpMethod::Post, url.clone(), Some(&body), TEXT_TIMEOUT_SECS, TEXT_MAX_RESPONSE_BYTES).await {
            // Some models don't take the level asked for (newer Flash models refuse MINIMAL): ask once more with the
            // model's own default, and remember it.
            Err(AutoPaperError::InvalidResponse { detail })
                if has_thinking_level(&body) && detail.to_ascii_lowercase().contains("thinking level") =>
            {
                if let Ok(mut set) = REFUSES_THINKING_LEVEL.lock() {
                    set.insert(model.clone());
                }
                without_thinking_level(&mut body);
                self.call(HttpMethod::Post, url, Some(&body), TEXT_TIMEOUT_SECS, TEXT_MAX_RESPONSE_BYTES).await?
            }
            other => other?,
        };
        parse_text(&json, &model)
    }

    /// Gemini models that support `generateContent`, minus image, embedding, TTS, audio, Live,
    /// computer-use and robotics models (Gemma models don't take a response schema).
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(model_infos(self.models().await?, is_text_model))
    }
}

#[async_trait]
impl ImageProvider for Google {
    fn kind(&self) -> ProviderKind {
        PROVIDER
    }

    fn check_ready(&self) -> Result<()> {
        read_key(self.secrets.as_ref()).map(|_| ())
    }

    fn default_model(&self) -> &str {
        DEFAULT_IMAGE_MODEL
    }

    /// Every supported aspect ratio at each resolution the model makes: 4K and 2K (Nano Banana 2 and Pro),
    /// or 1K (Nano Banana 2 Lite). The size picked from these sets only the aspect ratio of a request;
    /// `quality` sets the resolution (see [`output_size`]).
    fn capabilities(&self, model: &str) -> ImageCapabilities {
        let tiers: &[Tier] = if is_lite_image_model(model) { &[Tier::K1] } else { &[Tier::K4, Tier::K2] };
        let sizes = RATIOS.iter().flat_map(|ratio| tiers.iter().map(|&tier| ratio.size(tier))).collect();
        ImageCapabilities { sizes, free_size: None }
    }

    /// `request.seed` is ignored (see the module docs).
    async fn generate(&self, request: ImageRequest) -> Result<ImageResponse> {
        let model = model_id(&request.model, DEFAULT_IMAGE_MODEL)?;
        let ratio = nearest_ratio(request.width, request.height)
            .ok_or_else(|| AutoPaperError::invalid_input(InvalidInputReason::Other, "an image needs a width and a height"))?;
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            return Err(AutoPaperError::invalid_input(InvalidInputReason::Other, "the image prompt is empty"));
        }
        let body = json!({
            "contents": [ { "role": "user", "parts": [ { "text": prompt } ] } ],
            "generationConfig": {
                "responseModalities": ["IMAGE"],
                "imageConfig": { "aspectRatio": ratio.name, "imageSize": tier(&model, request.quality).param() },
            },
        });
        let url = format!("{ORIGIN}/v1beta/models/{model}:generateContent");
        let json = self.call(HttpMethod::Post, url, Some(&body), IMAGE_TIMEOUT_SECS, IMAGE_MAX_RESPONSE_BYTES).await?;
        parse_image(&json, &model)
    }

    /// Gemini models with "-image" in their ID that support `generateContent`.
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(model_infos(self.models().await?, is_image_model))
    }
}

/// The pixel size Gemini will return for a request: the nearest supported aspect ratio to
/// `width`:`height`, at the resolution `quality` selects for `model` (empty = the default image model).
/// `None` when either side is 0. Lets cost estimates and the pipeline know the real size up front.
pub fn output_size(model: &str, width: u32, height: u32, quality: ImageQuality) -> Option<(u32, u32)> {
    let model = canonical_model(model, DEFAULT_IMAGE_MODEL);
    nearest_ratio(width, height).map(|ratio| ratio.size(tier(model, quality)))
}

// ── Requests ────────────────────────────────────────────────────────────────────────────────

/// The generateContent body for concepts.
fn text_body(model: &str, request: &ComposeRequest) -> Result<Value> {
    let mut config = json!({
        "responseMimeType": "application/json",
        "responseSchema": response_schema(&request.schema)?,
    });
    match generation(model) {
        Some(major) if major >= 3 => {
            let level = if model.contains("-pro") { "LOW" } else { "MINIMAL" };
            config["thinkingConfig"] = json!({ "thinkingLevel": level });
        }
        Some(_) => {
            if let Some(temperature) = temperature(request.temperature) {
                config["temperature"] = json!(temperature);
            }
        }
        // An alias such as "gemini-flash-latest": its own defaults.
        None => {}
    }
    Ok(json!({
        "systemInstruction": { "parts": [ { "text": request.system } ] },
        "contents": [ { "role": "user", "parts": [ { "text": request.user } ] } ],
        "generationConfig": config,
    }))
}

fn has_thinking_level(body: &Value) -> bool {
    body.pointer("/generationConfig/thinkingConfig").is_some()
}

fn without_thinking_level(body: &mut Value) {
    if let Some(config) = body.pointer_mut("/generationConfig").and_then(Value::as_object_mut) {
        config.remove("thinkingConfig");
    }
}

/// The major version in "gemini-<major>[.<minor>]-…"; `None` for aliases and non-Gemini models.
fn generation(model: &str) -> Option<u32> {
    let rest = model.strip_prefix("gemini-")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Clamped to 0–2 and rounded to hundredths (f32 → JSON would otherwise read 0.6800000071…).
fn temperature(value: f32) -> Option<f64> {
    value.is_finite().then(|| (f64::from(value.clamp(0.0, 2.0)) * 100.0).round() / 100.0)
}

/// Converts the composer's JSON Schema to the OpenAPI-style subset `responseSchema` accepts: types upper-cased
/// (`"string"` → `"STRING"`), `["T", "null"]` → `T` + `nullable: true`, `propertyOrdering` added (the
/// `required` order first, since JSON maps here don't keep key order), and everything else —
/// `additionalProperties`, `$schema`, `$defs`, … — dropped, because the API rejects fields it doesn't know.
/// `InvalidInput` for what it can't express (type unions, `$ref`, nodes that aren't objects).
pub fn response_schema(schema: &Value) -> Result<Value> {
    convert(schema, 0)
}

fn convert(node: &Value, depth: usize) -> Result<Value> {
    let unsupported = |what: &str| AutoPaperError::invalid_input(InvalidInputReason::Other, format!("the concept schema {what}"));
    if depth > MAX_SCHEMA_DEPTH {
        return Err(unsupported("is nested too deeply for Gemini"));
    }
    let Some(map) = node.as_object() else {
        return Err(unsupported("has a node that isn't an object"));
    };
    if map.contains_key("$ref") {
        return Err(unsupported("uses $ref, which Gemini's responseSchema can't follow"));
    }
    let mut out = Map::new();
    for (key, value) in map {
        match key.as_str() {
            "type" => convert_type(value, &mut out).ok_or_else(|| unsupported("has a type Gemini can't express"))?,
            "properties" => {
                let properties =
                    value.as_object().ok_or_else(|| unsupported("has properties that aren't an object"))?;
                let mut converted = Map::new();
                for (name, property) in properties {
                    converted.insert(name.clone(), convert(property, depth + 1)?);
                }
                out.insert(key.clone(), Value::Object(converted));
            }
            "items" => {
                out.insert(key.clone(), convert(value, depth + 1)?);
            }
            "anyOf" => {
                let options = value.as_array().ok_or_else(|| unsupported("has an anyOf that isn't a list"))?;
                let converted = options.iter().map(|option| convert(option, depth + 1)).collect::<Result<Vec<_>>>()?;
                out.insert(key.clone(), Value::Array(converted));
            }
            "nullable" | "format" | "title" | "description" | "enum" | "required" | "minItems" | "maxItems"
            | "minLength" | "maxLength" | "pattern" | "minimum" | "maximum" | "minProperties" | "maxProperties"
            | "propertyOrdering" | "default" | "example" => {
                out.insert(key.clone(), value.clone());
            }
            _ => {}
        }
    }
    if let Some(properties) = out.get("properties").and_then(Value::as_object)
        && !out.contains_key("propertyOrdering")
    {
        let required = out.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
        let mut ordering: Vec<&str> = required.filter(|name| properties.contains_key(*name)).collect();
        let rest: Vec<&str> = properties.keys().map(String::as_str).filter(|name| !ordering.contains(name)).collect();
        ordering.extend(rest);
        let ordering = json!(ordering);
        out.insert("propertyOrdering".to_string(), ordering);
    }
    Ok(Value::Object(out))
}

/// `type` as `responseSchema` spells it. `None` for unknown types and unions of two or more non-null types.
fn convert_type(value: &Value, out: &mut Map<String, Value>) -> Option<()> {
    let upper = |name: &str| {
        matches!(name, "string" | "number" | "integer" | "boolean" | "array" | "object" | "null")
            .then(|| Value::String(name.to_ascii_uppercase()))
    };
    match value {
        Value::String(name) => {
            out.insert("type".to_string(), upper(name)?);
        }
        Value::Array(names) => {
            let names: Vec<&str> = names.iter().map(Value::as_str).collect::<Option<_>>()?;
            let non_null: Vec<&str> = names.iter().copied().filter(|name| *name != "null").collect();
            let [single] = non_null.as_slice() else {
                return None;
            };
            out.insert("type".to_string(), upper(single)?);
            if names.contains(&"null") {
                out.insert("nullable".to_string(), Value::Bool(true));
            }
        }
        _ => return None,
    }
    Some(())
}

/// Nano Banana 2 Lite makes 1K only.
fn is_lite_image_model(model: &str) -> bool {
    model.contains("flash-lite-image")
}

fn tier(model: &str, quality: ImageQuality) -> Tier {
    match quality {
        _ if is_lite_image_model(model) => Tier::K1,
        ImageQuality::Standard => Tier::K2,
        ImageQuality::High => Tier::K4,
    }
}

/// The ratio closest to `width`:`height` (log distance), e.g. 16:10 → 3:2, 5120×2160 → 21:9.
fn nearest_ratio(width: u32, height: u32) -> Option<&'static Ratio> {
    if width == 0 || height == 0 {
        return None;
    }
    let aspect = |(w, h): (u32, u32)| (f64::from(w) / f64::from(h)).ln();
    let target = aspect((width, height));
    let distance = |ratio: &&Ratio| (aspect(ratio.k4) - target).abs();
    RATIOS.iter().min_by(|a, b| distance(a).total_cmp(&distance(b)))
}

// ── Responses ───────────────────────────────────────────────────────────────────────────────

/// `finishReason`s that mean Gemini declined (safety, policy, recitation, or no image made).
fn is_refusal(reason: &str) -> bool {
    matches!(
        reason,
        "SAFETY"
            | "PROHIBITED_CONTENT"
            | "BLOCKLIST"
            | "SPII"
            | "RECITATION"
            | "IMAGE_SAFETY"
            | "IMAGE_PROHIBITED_CONTENT"
            | "IMAGE_RECITATION"
            | "NO_IMAGE"
    )
}

/// A blocked prompt comes back as a 200 with `promptFeedback.blockReason` and no candidates.
fn check_prompt_feedback(json: &Value) -> Result<()> {
    match json.pointer("/promptFeedback/blockReason").and_then(Value::as_str) {
        Some(reason) if !reason.is_empty() => Err(AutoPaperError::Refused { provider: PROVIDER }),
        _ => Ok(()),
    }
}

/// The first candidate's parts, without thoughts.
fn final_parts(candidate: &Value) -> impl Iterator<Item = &Value> {
    candidate
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|part| part["thought"] != true)
}

/// The structured output of a text answer. One that stopped early (other than for safety) or isn't JSON is
/// passed on as a JSON string with its usage: it was billed, and the composer finds no candidates in it, so
/// the engine asks again. Without any text it is `InvalidResponse`.
fn parse_text(json: &Value, requested_model: &str) -> Result<ComposeResponse> {
    check_prompt_feedback(json)?;
    let candidate = json.pointer("/candidates/0").ok_or_else(|| invalid("Gemini's response had no candidates"))?;
    let stopped_early = match candidate["finishReason"].as_str() {
        None | Some("STOP") => None,
        Some(reason) if is_refusal(reason) => return Err(AutoPaperError::Refused { provider: PROVIDER }),
        Some(reason) => Some(label(reason)),
    };
    let text: String = final_parts(candidate).filter_map(|part| part["text"].as_str()).collect();
    if text.trim().is_empty() {
        return Err(match stopped_early {
            Some(reason) => invalid(format!("Gemini stopped early ({reason})")),
            None => invalid("Gemini's response had no text"),
        });
    }
    let output = serde_json::from_str(&text).unwrap_or_else(|_| {
        tracing::warn!(stopped_early = stopped_early.as_deref(), "Gemini's concepts weren't valid JSON");
        Value::String(text.clone())
    });
    let count = |name: &str| json.pointer(&format!("/usageMetadata/{name}")).and_then(Value::as_u64).unwrap_or(0);
    let usage = Usage {
        input_tokens: count("promptTokenCount"),
        output_tokens: count("candidatesTokenCount").saturating_add(count("thoughtsTokenCount")),
    };
    Ok(ComposeResponse { output, usage, model: answering_model(json, requested_model) })
}

fn parse_image(json: &Value, requested_model: &str) -> Result<ImageResponse> {
    check_prompt_feedback(json)?;
    let candidate = json.pointer("/candidates/0").ok_or_else(|| invalid("Gemini's response had no candidates"))?;
    let finish = candidate["finishReason"].as_str().unwrap_or("STOP");
    if is_refusal(finish) {
        return Err(AutoPaperError::Refused { provider: PROVIDER });
    }
    let inline = final_parts(candidate)
        .filter_map(|part| part.get("inlineData").or_else(|| part.get("inline_data")))
        .find(|data| mime_of(data).is_none_or(|mime| mime.starts_with("image/")));
    let Some(inline) = inline else {
        return Err(match finish {
            "STOP" => invalid("Gemini's response had no image"),
            other => invalid(format!("Gemini made no image ({})", label(other))),
        });
    };
    let encoded = inline["data"].as_str().ok_or_else(|| invalid("Gemini's image had no data"))?;
    let bytes = BASE64.decode(encoded.trim()).map_err(|_| invalid("Gemini's image wasn't valid base64"))?;
    if bytes.is_empty() {
        return Err(invalid("Gemini's image was empty"));
    }
    Ok(ImageResponse {
        bytes,
        mime: image_mime(mime_of(inline)),
        model: answering_model(json, requested_model),
        reported_cost_microusd: None,
    })
}

fn mime_of(inline: &Value) -> Option<&str> {
    inline.get("mimeType").or_else(|| inline.get("mime_type")).and_then(Value::as_str)
}

/// The claimed type when it looks like one (the pipeline sniffs the real format anyway).
fn image_mime(claimed: Option<&str>) -> String {
    match claimed {
        None => "image/png".to_string(),
        Some(mime)
            if mime.len() <= 40
                && mime.strip_prefix("image/").is_some_and(|sub| {
                    !sub.is_empty() && sub.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
                }) =>
        {
            mime.to_ascii_lowercase()
        }
        Some(_) => "application/octet-stream".to_string(),
    }
}

fn answering_model(json: &Value, requested_model: &str) -> String {
    json["modelVersion"].as_str().filter(|id| plausible_model_id(id)).unwrap_or(requested_model).to_string()
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
/// that came after (nearly) the whole timeout is `ProviderUnavailable` "timed out" (Gemini is slow or stuck);
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

/// Google answers a bad key with 400 INVALID_ARGUMENT ("API key not valid…", reason `API_KEY_INVALID`),
/// not 401.
fn refine(error: AutoPaperError, response: &HttpResponse) -> AutoPaperError {
    if response.status == 400 && says_key_invalid(&response.body) {
        AutoPaperError::InvalidKey { provider: PROVIDER }
    } else {
        error
    }
}

fn says_key_invalid(body: &[u8]) -> bool {
    let Ok(json) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let message = json.pointer("/error/message").and_then(Value::as_str).unwrap_or_default();
    let details = json.pointer("/error/details").and_then(Value::as_array).into_iter().flatten();
    let mut reasons = details.filter_map(|detail| detail["reason"].as_str());
    message.contains("API key not valid")
        || message.contains("API key expired")
        || reasons.any(|reason| matches!(reason, "API_KEY_INVALID" | "API_KEY_EXPIRED"))
}

fn invalid(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::InvalidResponse { detail: detail.into() }
}

/// An untrusted enum-like value, safe to put in an error detail.
fn label(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect()
}

// ── Models ──────────────────────────────────────────────────────────────────────────────────

/// Trimmed, without a "models/" prefix; empty → `default`.
fn canonical_model<'a>(model: &'a str, default: &'a str) -> &'a str {
    let model = model.trim();
    match model.strip_prefix("models/").unwrap_or(model) {
        "" => default,
        model => model,
    }
}

/// The model ID goes in the URL path, so only the characters model IDs use are accepted.
fn model_id(model: &str, default: &str) -> Result<String> {
    let model = canonical_model(model, default);
    if plausible_model_id(model) {
        Ok(model.to_string())
    } else {
        Err(AutoPaperError::invalid_input(
            InvalidInputReason::Other,
            "Gemini model names use only letters, digits, '-', '_' and '.'",
        ))
    }
}

fn plausible_model_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// One entry of `models.list`.
struct Listed {
    id: String,
    display_name: String,
    generate_content: bool,
}

impl Listed {
    fn from_json(model: &Value) -> Option<Self> {
        let name = model["name"].as_str()?;
        let id = name.strip_prefix("models/").unwrap_or(name);
        if !plausible_model_id(id) {
            return None;
        }
        let display_name: String = model["displayName"]
            .as_str()
            .map(|name| name.chars().filter(|c| !c.is_control()).take(80).collect::<String>().trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| id.to_string());
        let generate_content = model["supportedGenerationMethods"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|method| method == "generateContent");
        Some(Self { id: id.to_string(), display_name, generate_content })
    }
}

/// Gemini's model list has no output-modality field (checked live 2026-10-06: name, displayName, description,
/// methods, token limits, thinking), and image, speech, music and agent models all list `generateContent`, so models
/// are sorted by family name. Image models: the `-image` family (Nano Banana, Nano Banana 2, Pro Image) and the
/// `nano-banana-*` IDs (e.g. `gemini-nano-banana-2.1`, `nano-banana-pro-preview`). Gemini Omni models are left out of
/// both lists until their image output is verified with this provider's request.
fn is_image_model(id: &str) -> bool {
    (id.starts_with("gemini-") && id.contains("-image")) || id.contains("nano-banana")
}

/// Models that write text well enough for the composer: the Gemini text families. Not image, speech, transcription,
/// live/streaming, music, video, embedding, agent or specialised variants (all of which also list `generateContent`).
fn is_text_model(id: &str) -> bool {
    const NOT_TEXT: [&str; 13] = [
        "-image", "nano-banana", "omni", "embedding", "-tts", "-audio", "-live", "transcribe", "computer-use",
        "robotics", "customtools", "native-audio", "translate",
    ];
    id.starts_with("gemini-") && !NOT_TEXT.iter().any(|word| id.contains(word))
}

/// `generateContent` models passing `keep`, sorted by ID, de-duplicated.
fn model_infos(models: Vec<Listed>, keep: fn(&str) -> bool) -> Vec<ModelInfo> {
    let mut models: Vec<Listed> = models.into_iter().filter(|m| m.generate_content && keep(&m.id)).collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    models.into_iter().map(|m| ModelInfo { id: m.id, display_name: m.display_name }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ComposeInputs;
    use crate::testing::{StubHttp, StubSecrets, response};

    const KEY: &str = "AIzaSyTest0123456789abcdefghijklmnopq";
    const TEXT_URL: &str =
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-flash-lite:generateContent";
    const IMAGE_URL: &str =
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-image:generateContent";

    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!("../../tests/fixtures/google/", $name))
        };
    }

    fn provider(http: &Arc<StubHttp>) -> Google {
        Google::new(http.clone(), Arc::new(StubSecrets::with(&[("google.api_key", KEY)])))
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

    fn converted_schema() -> Value {
        json!({
            "type": "OBJECT",
            "properties": {
                "candidates": {
                    "type": "ARRAY",
                    "items": {
                        "type": "OBJECT",
                        "properties": { "title": { "type": "STRING" }, "prompt": { "type": "STRING" } },
                        "required": ["title", "prompt"],
                        "propertyOrdering": ["title", "prompt"]
                    }
                }
            },
            "required": ["candidates"],
            "propertyOrdering": ["candidates"]
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
            expected_secs: None,
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

    fn assert_key_only_in_header(request: &HttpRequest) {
        assert_eq!(header(request, "x-goog-api-key"), Some(KEY));
        assert_eq!(header(request, "authorization"), None);
        assert!(!request.url.contains(KEY), "{}", request.url);
        assert!(!request.url.contains("key="), "{}", request.url);
        assert_eq!(request.policy, HostPolicy::Hosted(vec!["generativelanguage.googleapis.com".into()]));
    }

    // ── Text ────────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn compose_sends_generate_content_with_a_response_schema() {
        let http = Arc::new(StubHttp::new());
        http.once(":generateContent", 200, fixture!("generate_content_text.json"));
        provider(&http).compose(compose_request("")).await.expect("compose");

        let requests = http.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, TEXT_URL);
        assert_key_only_in_header(request);
        assert_eq!(header(request, "content-type"), Some("application/json"));
        assert_eq!(request.timeout_secs, 60);
        assert_eq!(request.max_response_bytes, TEXT_MAX_RESPONSE_BYTES);
        let body = http.json_body(0);
        assert!(body.get("store").is_none(), "generateContent has no store field");
        assert_eq!(
            body,
            json!({
                "systemInstruction": { "parts": [ { "text": "You compose desktop wallpaper concepts." } ] },
                "contents": [ { "role": "user", "parts": [ { "text": "Musts: rain. Maybes: ruins." } ] } ],
                "generationConfig": {
                    "responseMimeType": "application/json",
                    "responseSchema": converted_schema(),
                    "thinkingConfig": { "thinkingLevel": "MINIMAL" }
                }
            })
        );
    }

    #[tokio::test]
    async fn compose_returns_the_structured_output_usage_and_answering_model() {
        let http = Arc::new(StubHttp::new());
        http.once(":generateContent", 200, fixture!("generate_content_text.json"));
        let response = provider(&http).compose(compose_request("gemini-3.5-flash-lite")).await.expect("compose");

        assert_eq!(response.output["candidates"][0]["title"], "Rain over quiet ruins");
        assert_eq!(response.usage, Usage { input_tokens: 1804, output_tokens: 1490 + 64 });
        assert_eq!(response.model, "gemini-3.5-flash-lite");
    }

    #[tokio::test]
    async fn sampling_settings_follow_the_model_generation() {
        let http = Arc::new(StubHttp::new());
        http.always(":generateContent", 200, fixture!("generate_content_text.json"));
        let google = provider(&http);

        google.compose(compose_request("gemini-2.5-flash")).await.expect("2.5");
        let older = &http.json_body(0)["generationConfig"];
        assert_eq!(older["temperature"], 0.68);
        assert!(older.get("thinkingConfig").is_none(), "{older}");

        google.compose(compose_request("gemini-3-pro")).await.expect("pro");
        let pro = &http.json_body(1)["generationConfig"];
        assert_eq!(pro["thinkingConfig"], json!({ "thinkingLevel": "LOW" }));
        assert!(pro.get("temperature").is_none(), "{pro}");

        google.compose(compose_request("gemini-flash-latest")).await.expect("alias");
        let alias = &http.json_body(2)["generationConfig"];
        assert!(alias.get("temperature").is_none() && alias.get("thinkingConfig").is_none(), "{alias}");

        google.compose(compose_request(" models/gemini-3.8-flash ")).await.expect("prefixed");
        let requests = http.requests();
        assert_eq!(
            requests[3].url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent"
        );
        assert_eq!(http.json_body(3)["generationConfig"]["thinkingConfig"]["thinkingLevel"], "MINIMAL");
    }

    #[tokio::test]
    async fn a_model_that_refuses_the_thinking_level_is_asked_again_without_it() {
        let http = Arc::new(StubHttp::new());
        // gemini-3.7-flash: a made-up-for-the-test model ID, so the process-wide memory starts clean for it.
        http.once(
            ":generateContent",
            400,
            r#"{"error":{"code":400,"message":"Thinking level MINIMAL is not supported for this model. Please retry with other thinking level.","status":"INVALID_ARGUMENT"}}"#,
        );
        http.always(":generateContent", 200, fixture!("generate_content_text.json"));
        let google = provider(&http);
        google.compose(compose_request("gemini-3.7-flash-test")).await.expect("retried without the level");
        assert_eq!(http.json_body(0)["generationConfig"]["thinkingConfig"]["thinkingLevel"], "MINIMAL");
        assert!(http.json_body(1)["generationConfig"].get("thinkingConfig").is_none(), "{}", http.json_body(1));
        // Remembered: the next request for that model doesn't send a level at all.
        google.compose(compose_request("gemini-3.7-flash-test")).await.expect("second");
        assert!(http.json_body(2)["generationConfig"].get("thinkingConfig").is_none());
        assert_eq!(http.requests().len(), 3);
    }

    #[tokio::test]
    async fn compose_skips_thoughts() {
        let http = Arc::new(StubHttp::new());
        let body = json!({
            "candidates": [ { "content": { "role": "model", "parts": [
                { "text": "Thinking about rain…", "thought": true },
                { "text": "{\"candidates\":" },
                { "text": "[]}" }
            ] }, "finishReason": "STOP" } ]
        });
        http.once(":generateContent", 200, body.to_string());
        let response = provider(&http).compose(compose_request("")).await.expect("compose");
        assert_eq!(response.output, json!({ "candidates": [] }));
        assert_eq!(response.usage, Usage::default());
        assert_eq!(response.model, "gemini-3.5-flash-lite");
    }

    #[tokio::test]
    async fn compose_maps_blocks_and_bad_answers() {
        let http = Arc::new(StubHttp::new());
        let google = provider(&http);

        http.once(":generateContent", 200, fixture!("generate_content_prompt_blocked.json"));
        let result = google.compose(compose_request("")).await;
        assert!(matches!(result, Err(AutoPaperError::Refused { provider: ProviderKind::Google })), "{result:?}");

        http.once(":generateContent", 200, r#"{"candidates":[{"finishReason":"SAFETY","index":0}]}"#);
        assert!(matches!(google.compose(compose_request("")).await, Err(AutoPaperError::Refused { .. })));

        // Cut short, or not JSON: passed on as text (it was billed); the composer finds no candidates in it.
        let cut = r#"{"candidates":[{"content":{"parts":[{"text":"{\"candidates\":["}]},"finishReason":"MAX_TOKENS"}],
                      "usageMetadata":{"promptTokenCount":40,"candidatesTokenCount":8000}}"#;
        http.once(":generateContent", 200, cut);
        let cut = google.compose(compose_request("")).await.expect("cut short, as text");
        assert_eq!(cut.output, json!("{\"candidates\":["));
        assert_eq!(cut.usage, Usage { input_tokens: 40, output_tokens: 8000 }, "still billed");
        http.once(":generateContent", 200, r#"{"candidates":[{"finishReason":"MAX_TOKENS"}]}"#);
        match google.compose(compose_request("")).await {
            Err(AutoPaperError::InvalidResponse { detail }) => assert!(detail.contains("MAX_TOKENS"), "{detail}"),
            other => panic!("{other:?}"),
        }

        let prose =
            r#"{"candidates":[{"content":{"parts":[{"text":"Rain over ruins, four ways"}]},"finishReason":"STOP"}]}"#;
        http.once(":generateContent", 200, prose);
        let prose = google.compose(compose_request("")).await.expect("not JSON, as text");
        assert_eq!(prose.output, json!("Rain over ruins, four ways"));

        for body in [r#"{"candidates":[]}"#, r#"{"usageMetadata":{}}"#, "not json"] {
            http.once(":generateContent", 200, body);
            let result = google.compose(compose_request("")).await;
            assert!(matches!(result, Err(AutoPaperError::InvalidResponse { .. })), "{body}: {result:?}");
        }
    }

    #[test]
    fn converts_json_schema_to_response_schema() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "description": "Wallpaper concepts",
            "properties": {
                "mood": { "type": "array", "items": { "type": "string", "enum": ["calm", "dramatic"] }, "maxItems": 12 },
                "note": { "type": ["string", "null"], "description": "Echoes only" },
                "count": { "type": "integer", "minimum": 1 }
            },
            "required": ["note", "mood"],
            "additionalProperties": false,
            "$defs": {}
        });
        assert_eq!(
            response_schema(&schema).expect("convert"),
            json!({
                "type": "OBJECT",
                "description": "Wallpaper concepts",
                "properties": {
                    "mood": { "type": "ARRAY", "items": { "type": "STRING", "enum": ["calm", "dramatic"] }, "maxItems": 12 },
                    "note": { "type": "STRING", "nullable": true, "description": "Echoes only" },
                    "count": { "type": "INTEGER", "minimum": 1 }
                },
                "required": ["note", "mood"],
                "propertyOrdering": ["note", "mood", "count"]
            })
        );
        let explicit =
            json!({ "type": "object", "properties": { "a": { "type": "string" } }, "propertyOrdering": ["a"] });
        assert_eq!(response_schema(&explicit).expect("explicit")["propertyOrdering"], json!(["a"]));

        for bad in [
            json!({ "type": ["string", "integer"] }),
            json!({ "type": "tuple" }),
            json!({ "type": "object", "properties": { "a": "string" } }),
            json!({ "$ref": "#/$defs/concept" }),
            json!("object"),
        ] {
            let result = response_schema(&bad);
            assert!(matches!(result, Err(AutoPaperError::InvalidInput { .. })), "{bad}: {result:?}");
        }
        let mut deep = json!({ "type": "string" });
        for _ in 0..40 {
            deep = json!({ "type": "array", "items": deep });
        }
        assert!(matches!(response_schema(&deep), Err(AutoPaperError::InvalidInput { .. })));
    }

    #[tokio::test]
    async fn missing_keys_and_bad_model_names_fail_before_sending() {
        let http = Arc::new(StubHttp::new());
        http.always("googleapis.com", 200, fixture!("generate_content_text.json"));

        let keyless = Google::new(http.clone(), Arc::new(StubSecrets::default()));
        let result = keyless.compose(compose_request("")).await;
        assert!(matches!(result, Err(AutoPaperError::MissingKey { provider: ProviderKind::Google })), "{result:?}");
        let result = keyless.generate(image_request("", 3840, 2160, ImageQuality::High)).await;
        assert!(matches!(result, Err(AutoPaperError::MissingKey { .. })), "{result:?}");
        assert!(matches!(TextProvider::list_models(&keyless).await, Err(AutoPaperError::MissingKey { .. })));

        let spaced = Google::new(http.clone(), Arc::new(StubSecrets::with(&[("google.api_key", "AIza key\n")])));
        assert!(matches!(spaced.compose(compose_request("")).await, Err(AutoPaperError::InvalidKey { .. })));

        let google = provider(&http);
        for model in ["../../v1/files", "gemini-3:streamGenerateContent", "gemini?alt=sse", "models/", "gemini 3"] {
            let result = google.compose(compose_request(model)).await;
            let expected_default = model == "models/";
            if expected_default {
                assert!(result.is_ok(), "{model}: {result:?}");
            } else {
                assert!(matches!(result, Err(AutoPaperError::InvalidInput { .. })), "{model}: {result:?}");
            }
        }
        assert_eq!(http.requests().len(), 1);
        assert_eq!(http.requests()[0].url, TEXT_URL);
    }

    #[tokio::test]
    async fn maps_http_errors() {
        let http = Arc::new(StubHttp::new());
        let google = provider(&http);

        http.once(":generateContent", 400, fixture!("error_api_key_invalid.json"));
        let result = google.compose(compose_request("")).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidKey { provider: ProviderKind::Google })), "{result:?}");

        http.once(":generateContent", 403, r#"{"error":{"code":403,"message":"Method doesn't allow unregistered callers.","status":"PERMISSION_DENIED"}}"#);
        assert!(matches!(google.compose(compose_request("")).await, Err(AutoPaperError::InvalidKey { .. })));

        let mut exhausted = response(
            429,
            r#"{"error":{"code":429,"message":"Resource has been exhausted.","status":"RESOURCE_EXHAUSTED"}}"#,
        );
        exhausted.headers.push(("Retry-After".into(), "30".into()));
        http.once_response(":generateContent", exhausted);
        let result = google.generate(image_request("", 3840, 2160, ImageQuality::High)).await;
        assert!(
            matches!(result, Err(AutoPaperError::RateLimited { provider: ProviderKind::Google, retry_after_secs: 30 })),
            "{result:?}"
        );

        http.once(
            ":generateContent",
            503,
            r#"{"error":{"code":503,"message":"The model is overloaded.","status":"UNAVAILABLE"}}"#,
        );
        let result = google.compose(compose_request("")).await;
        assert!(matches!(result, Err(AutoPaperError::ProviderUnavailable { provider: ProviderKind::Google, .. })));

        let location = format!(
            r#"{{"error":{{"code":400,"message":"User location is not supported for the API use. ({KEY})","status":"FAILED_PRECONDITION"}}}}"#
        );
        http.once(":generateContent", 400, location);
        match google.compose(compose_request("")).await {
            Err(AutoPaperError::InvalidResponse { detail }) => {
                assert!(detail.starts_with("HTTP 400: User location is not supported"), "{detail}");
                assert!(!detail.contains(KEY), "{detail}");
            }
            other => panic!("{other:?}"),
        }

        http.fail_once(":generateContent", || {
            AutoPaperError::unavailable(ProviderKind::Demo, ProviderUnavailableReason::TimedOut, "timed out")
        });
        let result = google.generate(image_request("", 3840, 2160, ImageQuality::High)).await;
        assert!(matches!(result, Err(AutoPaperError::ProviderUnavailable { provider: ProviderKind::Google, .. })));
    }

    // ── Images ──────────────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn generate_sends_aspect_ratio_and_image_size() {
        let http = Arc::new(StubHttp::new());
        http.always(":generateContent", 200, fixture!("generate_content_image.json"));
        let google = provider(&http);
        google.generate(image_request("", 3840, 2160, ImageQuality::High)).await.expect("high");

        let requests = http.requests();
        let request = &requests[0];
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(request.url, IMAGE_URL);
        assert_key_only_in_header(request);
        assert_eq!(header(request, "content-type"), Some("application/json"));
        assert_eq!(request.timeout_secs, 240);
        assert_eq!(request.max_response_bytes, IMAGE_MAX_RESPONSE_BYTES);
        assert_eq!(
            http.json_body(0),
            json!({
                "contents": [ { "role": "user", "parts": [ { "text": "Ancient stone ruins at night in gentle rain" } ] } ],
                "generationConfig": {
                    "responseModalities": ["IMAGE"],
                    "imageConfig": { "aspectRatio": "16:9", "imageSize": "4K" }
                }
            })
        );

        let cases = [
            ("", 2560, 1600, ImageQuality::Standard, "3:2", "2K"), // 16:10 → 3:2, then crop
            ("gemini-3-pro-image", 5120, 2160, ImageQuality::High, "21:9", "4K"),
            ("", 1440, 2560, ImageQuality::High, "9:16", "4K"),
            ("", 2048, 1536, ImageQuality::Standard, "4:3", "2K"),
            ("gemini-3.1-flash-lite-image", 3840, 2160, ImageQuality::High, "16:9", "1K"),
        ];
        for (index, (model, width, height, quality, ratio, size)) in cases.into_iter().enumerate() {
            google.generate(image_request(model, width, height, quality)).await.expect("image");
            let config = &http.json_body(index + 1)["generationConfig"]["imageConfig"];
            assert_eq!(config, &json!({ "aspectRatio": ratio, "imageSize": size }), "{model} {width}x{height}");
        }
        assert!(http.requests()[2].url.contains("/v1beta/models/gemini-3-pro-image:generateContent"));
    }

    #[tokio::test]
    async fn generate_decodes_the_first_final_image() {
        let http = Arc::new(StubHttp::new());
        http.once(":generateContent", 200, fixture!("generate_content_image.json"));
        let image = provider(&http).generate(image_request("", 3840, 2160, ImageQuality::High)).await.expect("image");
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.model, "gemini-3.1-flash-image");
        assert_eq!(image.reported_cost_microusd, None);
        assert_eq!(image.bytes.len(), 72);
        let decoded = image::load_from_memory(&image.bytes).expect("valid PNG");
        assert_eq!((decoded.width(), decoded.height()), (2, 1));

        // A Pro "thought image" comes first; snake_case field names are accepted too.
        let thought = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR42mNoaGgAAAMEAYF1LgG8AAAAAElFTkSuQmCC";
        let fixture: Value = serde_json::from_str(fixture!("generate_content_image.json")).expect("fixture");
        let final_data = fixture["candidates"][0]["content"]["parts"][1]["inlineData"]["data"].clone();
        let body = json!({
            "candidates": [ { "content": { "parts": [
                { "inlineData": { "mimeType": "image/png", "data": thought }, "thought": true },
                { "inline_data": { "mime_type": "image/png", "data": final_data } }
            ] }, "finishReason": "STOP" } ]
        });
        http.once(":generateContent", 200, body.to_string());
        let image = provider(&http)
            .generate(image_request("gemini-3-pro-image", 3840, 2160, ImageQuality::High))
            .await
            .expect("pro");
        let decoded = image::load_from_memory(&image.bytes).expect("valid PNG");
        assert_eq!((decoded.width(), decoded.height()), (2, 1));
        assert_eq!(image.model, "gemini-3-pro-image");
    }

    #[tokio::test]
    async fn generate_maps_blocks_and_missing_images() {
        let http = Arc::new(StubHttp::new());
        let google = provider(&http);
        let request = || image_request("", 3840, 2160, ImageQuality::High);

        for refused in [
            fixture!("generate_content_prompt_blocked.json"),
            fixture!("generate_content_image_safety.json"),
            fixture!("generate_content_no_image.json"),
        ] {
            http.once(":generateContent", 200, refused);
            let result = google.generate(request()).await;
            assert!(matches!(result, Err(AutoPaperError::Refused { provider: ProviderKind::Google })), "{result:?}");
        }

        http.once(":generateContent", 200, fixture!("generate_content_image_other.json"));
        match google.generate(request()).await {
            Err(AutoPaperError::InvalidResponse { detail }) => assert!(detail.contains("IMAGE_OTHER"), "{detail}"),
            other => panic!("{other:?}"),
        }

        for body in [
            r#"{"candidates":[{"content":{"parts":[{"text":"Here is a description instead."}]},"finishReason":"STOP"}]}"#,
            r#"{"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":"@@@"}}]},"finishReason":"STOP"}]}"#,
            r#"{"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":""}}]}}]}"#,
            r#"{"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"audio/wav","data":"AAAA"}}]}}]}"#,
            r#"{"candidates":[]}"#,
        ] {
            http.once(":generateContent", 200, body);
            let result = google.generate(request()).await;
            assert!(matches!(result, Err(AutoPaperError::InvalidResponse { .. })), "{body}: {result:?}");
        }

        let mut empty = request();
        empty.prompt = " ".into();
        assert!(matches!(google.generate(empty).await, Err(AutoPaperError::InvalidInput { .. })));
        let mut no_size = request();
        no_size.width = 0;
        assert!(matches!(google.generate(no_size).await, Err(AutoPaperError::InvalidInput { .. })));
    }

    #[test]
    fn capabilities_and_output_sizes() {
        let google = provider(&Arc::new(StubHttp::new()));
        let caps = google.capabilities("");
        assert!(caps.free_size.is_none());
        for size in [(5504, 3072), (6336, 2688), (5056, 3392), (4800, 3584), (4096, 4096), (2752, 1536), (3168, 1344)] {
            assert!(caps.sizes.contains(&size), "{size:?}");
        }
        assert!(!caps.sizes.iter().any(|&(w, h)| w * 10 == h * 16), "no native 16:10");
        assert_eq!(caps.sizes.len(), RATIOS.len() * 2);
        assert_eq!(google.capabilities("gemini-3-pro-image").sizes, caps.sizes);
        let lite = google.capabilities("gemini-3.1-flash-lite-image");
        assert_eq!(lite.sizes.len(), RATIOS.len());
        assert!(lite.sizes.contains(&(1376, 768)) && lite.sizes.iter().all(|&(w, h)| w.max(h) <= 1584));

        assert_eq!(output_size("", 3840, 2160, ImageQuality::High), Some((5504, 3072)));
        assert_eq!(output_size("", 5120, 2880, ImageQuality::Standard), Some((2752, 1536)));
        assert_eq!(output_size("models/gemini-3-pro-image", 2560, 1600, ImageQuality::High), Some((5056, 3392)));
        assert_eq!(output_size("", 3440, 1440, ImageQuality::High), Some((6336, 2688)));
        assert_eq!(output_size("gemini-3.1-flash-lite-image", 3840, 2160, ImageQuality::High), Some((1376, 768)));
        assert_eq!(output_size("", 0, 1080, ImageQuality::High), None);
    }

    // ── Models ──────────────────────────────────────────────────────────────────────────────

    #[test]
    fn sorts_the_live_model_list_by_family() {
        // IDs from the live list (2026-10-06): image and other non-text models must not be offered for writing.
        for id in ["gemini-3.5-flash-lite", "gemini-3.8-flash", "gemini-flash-latest", "gemini-3.1-pro-preview", "gemini-2.5-pro"] {
            assert!(is_text_model(id), "{id} writes");
            assert!(!is_image_model(id), "{id} doesn't paint");
        }
        for id in ["gemini-nano-banana-2.1", "nano-banana-pro-preview", "gemini-3.1-flash-image", "gemini-2.5-flash-image", "gemini-3-pro-image"] {
            assert!(is_image_model(id), "{id} paints");
            assert!(!is_text_model(id), "{id} isn't a writer");
        }
        for id in [
            "gemini-omni-1.1-flash", "gemini-omni-flash-preview", "gemini-3.5-transcribe", "gemini-3.8-flash-tts",
            "gemini-3.8-live", "gemini-2.5-flash-native-audio-latest", "gemini-embedding-2", "gemini-robotics-er-2-preview",
            "gemini-2.5-computer-use-preview-10-2025", "gemini-3.1-pro-preview-customtools", "gemini-3.5-live-translate-preview",
            "lyria-3.5", "veo-3.1-generate-preview", "gemma-4-31b-it", "antigravity-preview-latest", "deep-research-preview-04-2026",
        ] {
            assert!(!is_text_model(id), "{id} isn't offered for writing");
            assert!(!is_image_model(id), "{id} isn't offered for painting");
        }
    }

    #[tokio::test]
    async fn list_models_pages_and_filters() {
        let http = Arc::new(StubHttp::new());
        let page_two = "pageToken=Cg5nZW1pbmktMy44LWZsYXNo%2B%2F%3D";
        for _ in 0..2 {
            http.once(page_two, 200, fixture!("models_list_page2.json"));
            http.once("/v1beta/models?pageSize=1000", 200, fixture!("models_list_page1.json"));
        }
        let google = provider(&http);

        let text = TextProvider::list_models(&google).await.expect("text");
        assert_eq!(
            text,
            vec![
                ModelInfo { id: "gemini-3.1-flash-lite".into(), display_name: "Gemini 3.1 Flash-Lite".into() },
                ModelInfo { id: "gemini-3.5-flash-lite".into(), display_name: "Gemini 3.5 Flash-Lite".into() },
                ModelInfo { id: "gemini-3.8-flash".into(), display_name: "Gemini 3.8 Flash".into() },
            ]
        );
        let images = ImageProvider::list_models(&google).await.expect("images");
        let ids: Vec<&str> = images.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gemini-3-pro-image", "gemini-3.1-flash-image", "gemini-3.1-flash-lite-image"]);
        assert_eq!(images[1].display_name, "Nano Banana 2");

        let requests = http.requests();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, HttpMethod::Get);
        assert_eq!(requests[0].url, "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000");
        assert_eq!(
            requests[1].url,
            "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000&pageToken=Cg5nZW1pbmktMy44LWZsYXNo%2B%2F%3D"
        );
        for request in &requests {
            assert_key_only_in_header(request);
            assert!(request.body.is_none());
            assert_eq!(header(request, "content-type"), None);
        }

        let odd = Arc::new(StubHttp::new());
        odd.once("/v1beta/models", 200, "[]");
        assert!(matches!(
            TextProvider::list_models(&provider(&odd)).await,
            Err(AutoPaperError::InvalidResponse { .. })
        ));
        odd.once("/v1beta/models", 200, "{}");
        assert_eq!(TextProvider::list_models(&provider(&odd)).await.expect("empty"), vec![]);
    }

    #[test]
    fn kinds_and_defaults() {
        let google = provider(&Arc::new(StubHttp::new()));
        assert_eq!(TextProvider::kind(&google), ProviderKind::Google);
        assert_eq!(ImageProvider::kind(&google), ProviderKind::Google);
        assert_eq!(TextProvider::default_model(&google), "gemini-3.5-flash-lite");
        assert_eq!(ImageProvider::default_model(&google), "gemini-3.1-flash-image");
    }

    /// `GEMINI_API_KEY=… cargo test -p autopaper-core --lib providers::google -- --ignored`. Lists models and
    /// composes one tiny structured answer (well under a cent); makes no image (image models are paid only).
    #[tokio::test]
    #[ignore = "live: calls generativelanguage.googleapis.com with GEMINI_API_KEY"]
    async fn live_smoke() {
        let Some(key) = std::env::var("GEMINI_API_KEY").ok().filter(|key| !key.trim().is_empty()) else {
            eprintln!("GEMINI_API_KEY isn't set; skipping the live Gemini smoke test");
            return;
        };
        let http = Arc::new(crate::net::ReqwestClient::new("live-test").expect("http client"));
        let google = Google::new(http, Arc::new(StubSecrets::with(&[("google.api_key", key.trim())])));

        let models = TextProvider::list_models(&google).await.expect("list models");
        assert!(models.iter().any(|m| m.id == DEFAULT_TEXT_MODEL), "{models:?}");
        let images = ImageProvider::list_models(&google).await.expect("list image models");
        assert!(images.iter().any(|m| m.id == DEFAULT_IMAGE_MODEL), "{images:?}");

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
            expected_secs: None,
        };
        let response = google.compose(request).await.expect("compose");
        assert!(response.output["word"].is_string(), "{}", response.output);
        assert!(response.usage.input_tokens > 0);
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
        let secrets: Arc<dyn SecretStore> = Arc::new(StubSecrets::with(&[("google.api_key", KEY)]));
        let stalled = Google::new(Arc::new(FailsAfter(None)), secrets.clone());
        assert_timed_out(stalled.compose(compose_request("")).await);
        assert_timed_out(stalled.generate(image_request("", 3840, 2160, ImageQuality::High)).await);
        assert_timed_out(TextProvider::list_models(&stalled).await);

        // A connection or DNS failure, or a connect timeout (10 s), well before the request's timeout: offline.
        for secs in [0, 10] {
            let unreachable = Google::new(Arc::new(FailsAfter(Some(secs))), secrets.clone());
            let result = unreachable.compose(compose_request("")).await;
            assert!(matches!(result, Err(AutoPaperError::Offline)), "{secs} s: {result:?}");
            let result = unreachable.generate(image_request("", 3840, 2160, ImageQuality::High)).await;
            assert!(matches!(result, Err(AutoPaperError::Offline)), "{secs} s: {result:?}");
        }
    }
}
