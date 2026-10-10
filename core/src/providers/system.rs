//! The model built into the operating system (Apple's Foundation Models on a Mac), reached through the host's
//! `SystemModel`. It writes ideas and never paints. Its answers are short and its instructions can't be long
//! (a window of 4,096–8,192 tokens), so the composer asks it differently (`composer::OnDevice`).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::{ComposeRequest, ComposeResponse, TextProvider, Usage};
use crate::error::{AutoPaperError, Result};
use crate::model::{ModelInfo, ProviderKind, SystemModelProblem, SystemModelReason, SystemModelStatus};
use crate::ports::SystemModel;

pub const MODEL: &str = "on-device";
/// What a request may take at most: a model that runs on the computer's own chip is slower than a hosted one, and a
/// retry adds to it.
const TIMEOUT: Duration = Duration::from_secs(300);
/// The most the model is asked to write, so an answer that runs away stops at a bounded length.
const MAX_OUTPUT_TOKENS: u32 = 2400;

pub struct System {
    model: Arc<dyn SystemModel>,
}

impl System {
    pub fn new(model: Arc<dyn SystemModel>) -> Self {
        Self { model }
    }
}

fn unavailable(reason: SystemModelReason, status: &SystemModelStatus) -> AutoPaperError {
    let why = match reason {
        SystemModelReason::DeviceNotEligible => "this computer can't run it",
        SystemModelReason::NotEnabled => "it's switched off in the system's settings",
        SystemModelReason::NotReady => "the system is still preparing it",
        SystemModelReason::Other => "the system didn't say why",
    };
    let name = if status.name.is_empty() { "The on-device model".to_string() } else { status.name.clone() };
    AutoPaperError::unavailable(ProviderKind::System, crate::error::ProviderUnavailableReason::NotRunning, format!("{name} isn't available: {why}."))
}

#[async_trait]
impl TextProvider for System {
    fn kind(&self) -> ProviderKind {
        ProviderKind::System
    }

    fn default_model(&self) -> &str {
        MODEL
    }

    fn check_ready(&self) -> Result<()> {
        let status = self.model.status();
        match (status.available, status.reason) {
            (true, _) => Ok(()),
            (false, reason) => Err(unavailable(reason.unwrap_or(SystemModelReason::Other), &status)),
        }
    }

    async fn compose(&self, request: ComposeRequest) -> Result<ComposeResponse> {
        self.check_ready()?;
        let model = self.model.clone();
        let schema = request.schema.to_string();
        let temperature = request.temperature;
        let call = tokio::task::spawn_blocking(move || {
            model.compose(request.system, request.user, schema, temperature, MAX_OUTPUT_TOKENS)
        });
        let outcome = match tokio::time::timeout(TIMEOUT, call).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(error)) => return Err(AutoPaperError::InvalidResponse { detail: format!("the on-device model's call failed: {error}") }),
            Err(_) => {
                return Err(AutoPaperError::unavailable(
                    ProviderKind::System,
                    crate::error::ProviderUnavailableReason::TimedOut,
                    "The on-device model didn't answer in time.",
                ));
            }
        };
        match outcome.problem {
            Some(SystemModelProblem::Unavailable { reason }) => Err(unavailable(reason, &self.model.status())),
            Some(SystemModelProblem::ContextTooSmall) => Err(AutoPaperError::InvalidResponse {
                detail: "the request and the answer didn't fit the on-device model's context window".into(),
            }),
            Some(SystemModelProblem::Refused) => Err(AutoPaperError::Refused { provider: ProviderKind::System }),
            Some(SystemModelProblem::RateLimited) => Err(AutoPaperError::RateLimited { provider: ProviderKind::System, retry_after_secs: 60 }),
            Some(SystemModelProblem::Failed { detail }) => Err(AutoPaperError::InvalidResponse { detail }),
            None => {
                let output = serde_json::from_str(&outcome.json)
                    .map_err(|error| AutoPaperError::InvalidResponse { detail: format!("the on-device model's answer wasn't JSON: {error}") })?;
                Ok(ComposeResponse {
                    output,
                    usage: Usage { input_tokens: outcome.input_tokens, output_tokens: outcome.output_tokens },
                    model: MODEL.into(),
                })
            }
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let status = self.model.status();
        let name = if status.name.is_empty() { "On-device model".to_string() } else { status.name };
        Ok(vec![ModelInfo { id: MODEL.into(), display_name: name }])
    }

    async fn check_available(&self) -> Result<()> {
        self.check_ready()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SystemComposeOutcome;
    use std::sync::Mutex;

    struct Fake {
        status: SystemModelStatus,
        outcome: Mutex<SystemComposeOutcome>,
        seen: Mutex<Vec<(String, f32, u32)>>,
    }

    impl SystemModel for Fake {
        fn status(&self) -> SystemModelStatus {
            self.status.clone()
        }
        fn compose(&self, system: String, _user: String, _schema: String, temperature: f32, max: u32) -> SystemComposeOutcome {
            self.seen.lock().unwrap().push((system, temperature, max));
            self.outcome.lock().unwrap().clone()
        }
    }

    fn fake(available: bool, reason: Option<SystemModelReason>, outcome: SystemComposeOutcome) -> Arc<Fake> {
        Arc::new(Fake {
            status: SystemModelStatus { available, reason, name: "Apple Intelligence".into(), context_tokens: 8192 },
            outcome: Mutex::new(outcome),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn request() -> ComposeRequest {
        ComposeRequest {
            model: String::new(),
            system: "Write ideas.".into(),
            user: "Must: rain.".into(),
            schema: serde_json::json!({"type": "object"}),
            temperature: 0.9,
            inputs: Default::default(),
            expected_secs: None,
        }
    }

    fn ok(json: &str) -> SystemComposeOutcome {
        SystemComposeOutcome { json: json.into(), input_tokens: 120, output_tokens: 80, problem: None }
    }

    #[tokio::test]
    async fn an_answer_comes_back_as_json_with_its_token_counts() {
        let model = fake(true, None, ok(r#"{"candidates": []}"#));
        let response = System::new(model.clone()).compose(request()).await.expect("compose");
        assert_eq!(response.output, serde_json::json!({"candidates": []}));
        assert_eq!((response.usage.input_tokens, response.usage.output_tokens), (120, 80));
        assert_eq!(model.seen.lock().unwrap()[0], ("Write ideas.".to_string(), 0.9, MAX_OUTPUT_TOKENS));
    }

    #[tokio::test]
    async fn an_unavailable_model_is_said_before_anything_is_asked() {
        let model = fake(false, Some(SystemModelReason::NotEnabled), ok("{}"));
        let error = System::new(model.clone()).compose(request()).await.expect_err("unavailable");
        assert!(matches!(error, AutoPaperError::ProviderUnavailable { provider: ProviderKind::System, .. }), "{error:?}");
        assert!(error.to_string().contains("switched off"), "{error}");
        assert!(model.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_models_problems_become_the_cores_errors() {
        for (problem, check) in [
            (SystemModelProblem::Refused, "declined"),
            (SystemModelProblem::RateLimited, "rate-limiting"),
            (SystemModelProblem::ContextTooSmall, "context window"),
            (SystemModelProblem::Failed { detail: "boom".into() }, "boom"),
        ] {
            let model = fake(true, None, SystemComposeOutcome { json: String::new(), input_tokens: 0, output_tokens: 0, problem: Some(problem) });
            let error = System::new(model).compose(request()).await.expect_err("problem");
            assert!(error.to_string().contains(check), "{error}");
        }
    }

    #[tokio::test]
    async fn an_answer_that_isnt_json_is_unreadable() {
        let model = fake(true, None, ok("not json"));
        let error = System::new(model).compose(request()).await.expect_err("not json");
        assert!(matches!(error, AutoPaperError::InvalidResponse { .. }), "{error:?}");
    }
}
