//! Ollama (local): concepts only — Ollama removed image generation in 2026-07 (docs/research/providers.md).
//!
//! Native `/api/chat` with `format: <JSON schema>`, `stream: false`, `think: false` (concepts don't need a
//! reasoning pass, and thinking models otherwise spend most of the timeout on it; models without thinking
//! accept the flag) and `options.temperature`; models from `/api/tags` (embedding-only models left out).
//! Base URL from the selection (default http://127.0.0.1:11434), checked with `HostPolicy::UserEndpoint`.
//! No key. An empty model means the first listed. A connection failure means "Ollama isn't running"
//! (`ProviderUnavailable`), not `Offline`; a request that used its whole timeout means it was too slow
//! (`ProviderUnavailable` too). Text timeout: max(10 minutes, 3 × the learned estimate) — a model loaded from scratch can take
//! minutes (`providers::local_timeout_secs`). An answer that was cut
//! short or isn't JSON is returned as that text (a JSON string): the composer finds no candidates in it, so
//! the engine asks again; an empty answer is `InvalidResponse`.
//! Shapes: `core/tests/fixtures/ollama/` (recorded from Ollama 0.34.2).

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason, Result};
use crate::model::{ModelInfo, ProviderKind};
use crate::net::{HostPolicy, error_for_status};
use crate::ports::{HttpClient, HttpMethod, HttpRequest};

use super::openai_compat::json_in_text;
use super::{ComposeRequest, ComposeResponse, TextProvider, Usage, local_timeout_secs, used_whole_timeout};

const KIND: ProviderKind = ProviderKind::Ollama;
const LIST_TIMEOUT_SECS: u64 = 20;
/// A thinking model's answer can carry its (ignored) reasoning too.
const MAX_TEXT_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_LIST_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

pub struct Ollama {
    http: Arc<dyn HttpClient>,
    base_url: String,
}

impl Ollama {
    pub fn new(http: Arc<dyn HttpClient>, base_url: String) -> Self {
        Self { http, base_url }
    }

    fn base(&self) -> &str {
        self.base_url.trim().trim_end_matches('/')
    }

    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        method: HttpMethod,
        path: &str,
        body: Option<&serde_json::Value>,
        timeout_secs: u64,
        max_response_bytes: usize,
    ) -> Result<T> {
        let mut headers = vec![("accept".to_string(), "application/json".to_string())];
        if body.is_some() {
            headers.push(("content-type".to_string(), "application/json".to_string()));
        }
        let request = HttpRequest {
            method,
            url: format!("{}{path}", self.base()),
            policy: HostPolicy::UserEndpoint,
            headers,
            body: body.map(serde_json::Value::to_string).map(String::into_bytes),
            timeout_secs,
            max_response_bytes,
        };
        let started = tokio::time::Instant::now();
        let response = self.http.send(request).await.map_err(|error| match error {
            AutoPaperError::Offline if used_whole_timeout(started, timeout_secs) => {
                AutoPaperError::unavailable(KIND, ProviderUnavailableReason::TimedOut, format!("Ollama didn't answer within {timeout_secs} s"))
            }
            AutoPaperError::Offline => {
                AutoPaperError::unavailable(KIND, ProviderUnavailableReason::NotRunning, format!("Ollama isn't running at {}", self.base()))
            }
            error @ AutoPaperError::ProviderUnavailable { .. } => error.for_provider(KIND),
            other => other,
        })?;
        if let Some(error) = error_for_status(KIND, &response) {
            return Err(error);
        }
        serde_json::from_slice(&response.body)
            .map_err(|_| AutoPaperError::InvalidResponse { detail: format!("Ollama's answer to {path} wasn't the expected JSON") })
    }

    async fn first_model(&self) -> Result<String> {
        self.list_models()
            .await?
            .into_iter()
            .next()
            .map(|model| model.id)
            .ok_or_else(|| AutoPaperError::invalid_input(InvalidInputReason::NoModels, "Ollama has no models yet; download one in Ollama first"))
    }
}

#[async_trait]
impl TextProvider for Ollama {
    fn kind(&self) -> ProviderKind {
        KIND
    }

    /// Empty: the person picks one of their installed models (the first listed is used if unset).
    fn default_model(&self) -> &str {
        ""
    }

    async fn compose(&self, request: ComposeRequest) -> Result<ComposeResponse> {
        let model = match request.model.trim() {
            "" => self.first_model().await?,
            named => named.to_string(),
        };
        let mut messages = Vec::with_capacity(2);
        if !request.system.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": request.system }));
        }
        messages.push(json!({ "role": "user", "content": request.user }));
        let body = json!({
            "model": model,
            "messages": messages,
            "format": request.schema,
            "stream": false,
            "think": false,
            "options": { "temperature": request.temperature },
        });
        let chat: ChatResponse = self.send(HttpMethod::Post, "/api/chat", Some(&body), local_timeout_secs(request.expected_secs), MAX_TEXT_RESPONSE_BYTES).await?;
        let content = chat.message.map(|message| message.content).unwrap_or_default();
        let why = if chat.done_reason.as_deref() == Some("length") {
            "Ollama's answer was cut short"
        } else {
            "Ollama's answer wasn't the JSON that was asked for"
        };
        if content.trim().is_empty() {
            return Err(AutoPaperError::InvalidResponse { detail: why.into() });
        }
        let output = json_in_text(&content).unwrap_or_else(|| {
            tracing::warn!("{why}");
            serde_json::Value::String(content.clone())
        });
        Ok(ComposeResponse {
            output,
            usage: Usage { input_tokens: chat.prompt_eval_count, output_tokens: chat.eval_count },
            model: chat.model.filter(|m| !m.trim().is_empty()).unwrap_or(model),
        })
    }

    /// Installed models, newest first (Ollama's order), without embedding-only models.
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let tags: Tags = self.send(HttpMethod::Get, "/api/tags", None, LIST_TIMEOUT_SECS, MAX_LIST_RESPONSE_BYTES).await?;
        Ok(tags
            .models
            .into_iter()
            .filter(|model| !model.name.trim().is_empty())
            .filter(|model| model.capabilities.as_ref().is_none_or(|caps| caps.iter().any(|c| c == "completion")))
            .map(|model| ModelInfo { display_name: model.name.clone(), id: model.name })
            .collect())
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    message: Option<ChatMessage>,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    eval_count: u64,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct Tags {
    #[serde(default)]
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
    /// Present in recent Ollama releases ("completion", "embedding", "vision", "thinking", …).
    #[serde(default)]
    capabilities: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::providers::ComposeInputs;
    use crate::testing::StubHttp;

    const TAGS: &str = include_str!("../../tests/fixtures/ollama/tags.json");
    const CHAT: &str = include_str!("../../tests/fixtures/ollama/chat-response.json");
    const CHAT_THINKING: &str = include_str!("../../tests/fixtures/ollama/chat-response-thinking.json");
    const NOT_FOUND: &str = include_str!("../../tests/fixtures/ollama/error-model-not-found.json");

    fn ollama(http: &Arc<StubHttp>) -> Ollama {
        Ollama::new(http.clone(), "http://127.0.0.1:11434".into())
    }

    fn request(model: &str) -> ComposeRequest {
        ComposeRequest {
            model: model.into(),
            system: "You write wallpaper concepts.".into(),
            user: "Must: rain, ruins. Maybe: blue hour.".into(),
            schema: json!({ "type": "object", "properties": { "candidates": { "type": "array" } }, "required": ["candidates"] }),
            temperature: 0.56,
            inputs: ComposeInputs::default(),
            expected_secs: None,
        }
    }

    #[tokio::test]
    async fn composes_with_the_schema_as_format() {
        let http = Arc::new(StubHttp::new());
        http.once("/api/chat", 200, CHAT);
        let req = request("ornith:latest");
        let schema = req.schema.clone();
        let response = ollama(&http).compose(req).await.unwrap();

        let sent = &http.requests()[0];
        assert_eq!(sent.method, HttpMethod::Post);
        assert_eq!(sent.url, "http://127.0.0.1:11434/api/chat");
        assert_eq!(sent.policy, HostPolicy::UserEndpoint);
        assert_eq!(sent.timeout_secs, 10 * 60, "a model loaded from scratch can take minutes");
        assert!(sent.headers.iter().all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
        let body = http.json_body(0);
        assert_eq!(body["model"], "ornith:latest");
        assert_eq!(body["format"], schema);
        assert_eq!(body["stream"], false);
        assert_eq!(body["think"], false);
        assert!((body["options"]["temperature"].as_f64().unwrap() - 0.56).abs() < 1e-6);
        assert_eq!(body["messages"][0], json!({"role": "system", "content": "You write wallpaper concepts."}));
        assert_eq!(body["messages"][1], json!({"role": "user", "content": "Must: rain, ruins. Maybe: blue hour."}));

        let candidates = response.output["candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0]["title"], "Rain-Stained Gothic Ruins at Blue Hour");
        assert_eq!(response.usage, Usage { input_tokens: 74, output_tokens: 977 });
        assert_eq!(response.model, "ornith:latest");
    }

    #[tokio::test]
    async fn ignores_a_thinking_models_reasoning() {
        let http = Arc::new(StubHttp::new());
        http.once("/api/chat", 200, CHAT_THINKING);
        let response = ollama(&http).compose(request("ornith:latest")).await.unwrap();
        assert_eq!(response.output["candidates"][0]["title"], "The Rain-Fed Arch");
        assert_eq!(response.usage, Usage { input_tokens: 72, output_tokens: 2186 });
    }

    #[tokio::test]
    async fn an_empty_model_uses_the_first_installed_text_model() {
        let http = Arc::new(StubHttp::new());
        http.once(
            "/api/tags",
            200,
            r#"{"models":[{"name":"nomic-embed-text:latest","capabilities":["embedding"]},{"name":"gemma4:latest","capabilities":["completion","vision"]}]}"#,
        );
        http.once("/api/chat", 200, CHAT);
        ollama(&http).compose(request("")).await.unwrap();
        assert_eq!(http.requests()[0].url, "http://127.0.0.1:11434/api/tags");
        assert_eq!(http.json_body(1)["model"], "gemma4:latest");

        let none = Arc::new(StubHttp::new());
        none.once("/api/tags", 200, r#"{"models":[]}"#);
        let result = ollama(&none).compose(request("  ")).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::NoModels, .. })), "{result:?}");
    }

    #[tokio::test]
    async fn lists_text_models_from_tags() {
        let http = Arc::new(StubHttp::new());
        http.once("/api/tags", 200, TAGS);
        let models = Ollama::new(http.clone(), " http://127.0.0.1:11434/ ".into()).list_models().await.unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["ornith:latest", "qwen2.5vl:7b"], "the embedding model is left out");
        assert_eq!(models[1].display_name, "qwen2.5vl:7b");
        let sent = &http.requests()[0];
        assert_eq!(sent.method, HttpMethod::Get);
        assert_eq!(sent.url, "http://127.0.0.1:11434/api/tags");
        assert!(sent.body.is_none());

        // Older releases don't list capabilities: every model is offered.
        http.once("/api/tags", 200, r#"{"models":[{"name":"llama3:8b"},{"name":""}]}"#);
        let models = ollama(&http).list_models().await.unwrap();
        assert_eq!(models, vec![ModelInfo { id: "llama3:8b".into(), display_name: "llama3:8b".into() }]);
    }

    #[tokio::test]
    async fn not_running_is_unavailable_not_offline() {
        let http = Arc::new(StubHttp::new());
        match ollama(&http).compose(request("gemma4")).await {
            Err(error @ AutoPaperError::ProviderUnavailable { provider: KIND, reason: ProviderUnavailableReason::NotRunning, .. }) => {
                assert!(error.is_transient());
                assert!(error.to_string().contains("Ollama isn't running at http://127.0.0.1:11434"), "{error}");
            }
            other => panic!("{other:?}"),
        }
        let listed = ollama(&http).list_models().await;
        assert!(matches!(listed, Err(AutoPaperError::ProviderUnavailable { provider: KIND, .. })), "{listed:?}");

        http.fail_once("/api/chat", || AutoPaperError::unavailable(ProviderKind::Demo, ProviderUnavailableReason::TimedOut, "timed out"));
        let timed_out = ollama(&http).compose(request("gemma4")).await;
        assert!(
            matches!(&timed_out, Err(AutoPaperError::ProviderUnavailable { provider: KIND, detail, .. }) if detail == "timed out"),
            "{timed_out:?}"
        );
    }

    #[tokio::test]
    async fn maps_errors_and_bad_answers() {
        let http = Arc::new(StubHttp::new());
        http.once("/api/chat", 404, NOT_FOUND);
        match ollama(&http).compose(request("no-such-model")).await {
            Err(AutoPaperError::InvalidResponse { detail }) => assert_eq!(detail, "HTTP 404: model 'no-such-model' not found"),
            other => panic!("{other:?}"),
        }

        http.once("/api/chat", 503, "");
        let busy = ollama(&http).compose(request("m")).await;
        assert!(matches!(busy, Err(AutoPaperError::ProviderUnavailable { provider: KIND, .. })), "{busy:?}");

        // Cut short, or not JSON: passed on as text; the composer finds no candidates in it.
        let cut = json!({ "model": "m", "message": { "role": "assistant", "content": "{\"candidates\":[{\"ti" }, "done": true, "done_reason": "length" });
        http.once("/api/chat", 200, cut.to_string());
        let cut = ollama(&http).compose(request("m")).await.expect("cut short, as text");
        assert_eq!(cut.output, json!("{\"candidates\":[{\"ti"));
        let empty = json!({ "model": "m", "message": { "role": "assistant", "content": "" }, "done": true, "done_reason": "length" });
        http.once("/api/chat", 200, empty.to_string());
        match ollama(&http).compose(request("m")).await {
            Err(AutoPaperError::InvalidResponse { detail }) => assert!(detail.contains("cut short"), "{detail}"),
            other => panic!("{other:?}"),
        }

        let prose = json!({ "model": "m", "message": { "role": "assistant", "content": "Sure! Here are some ideas." }, "done": true, "done_reason": "stop" });
        http.once("/api/chat", 200, prose.to_string());
        let prose = ollama(&http).compose(request("m")).await.expect("not JSON, as text");
        assert_eq!(prose.output, json!("Sure! Here are some ideas."));

        http.once("/api/chat", 200, "not json at all");
        let garbage = ollama(&http).compose(request("m")).await;
        assert!(matches!(garbage, Err(AutoPaperError::InvalidResponse { .. })), "{garbage:?}");
    }

    #[tokio::test]
    async fn a_public_plain_http_address_is_refused() {
        let http = Arc::new(StubHttp::new());
        let result = Ollama::new(http.clone(), "http://ollama.example.com:11434".into()).list_models().await;
        assert!(
            matches!(result, Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::AddressNotAllowed, .. })),
            "{result:?}"
        );
        assert!(http.requests().is_empty());
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

    #[tokio::test]
    async fn a_slow_model_is_given_three_times_what_it_usually_takes() {
        let http = Arc::new(StubHttp::new());
        http.once("/api/chat", 200, CHAT);
        http.once("/api/chat", 200, CHAT);
        let usually_slow = ComposeRequest { expected_secs: Some(400.0), ..request("big:70b") };
        ollama(&http).compose(usually_slow).await.unwrap();
        let usually_quick = ComposeRequest { expected_secs: Some(20.0), ..request("small:1b") };
        ollama(&http).compose(usually_quick).await.unwrap();
        assert_eq!(http.requests()[0].timeout_secs, 1200, "3 × 400 s");
        assert_eq!(http.requests()[1].timeout_secs, 600, "quick history never shortens the cold-start allowance");
    }

    #[tokio::test(start_paused = true)]
    async fn too_slow_is_unavailable_not_absent() {
        match Ollama::new(Arc::new(Stalled), "http://127.0.0.1:11434".into()).compose(request("gemma4")).await {
            Err(AutoPaperError::ProviderUnavailable { provider: KIND, reason, detail }) => {
                assert_eq!(reason, ProviderUnavailableReason::TimedOut);
                assert_eq!(detail, "Ollama didn't answer within 600 s");
            }
            other => panic!("{other:?}"),
        }
    }
}
