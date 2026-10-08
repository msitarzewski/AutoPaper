//! Provider plugins. Each provider is one file implementing `TextProvider` and/or `ImageProvider`,
//! plus one line in `registry`. Requests go through the shared `HttpClient` (network policy, caps,
//! timeouts); keys come from the host's `SecretStore` at call time and are never stored by the core.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

pub mod comfyui;
pub mod demo;
pub mod google;
pub mod ollama;
pub mod openai;
pub mod openai_compat;
pub mod registry;

use crate::error::Result;
use crate::model::{ImageQuality, ModelInfo, ProviderKind};

/// The HTTP client reports a request that ran out of time as `Offline`, like a connection or DNS failure (it
/// doesn't know the provider). A failure this close to the request's timeout was the timeout.
const TIMEOUT_SLACK: Duration = Duration::from_secs(1);

/// A request sent at `started` with `timeout_secs` failed after (nearly) all of that time: when the client
/// says `Offline`, it was a timeout, so providers report the provider as slow (`ProviderUnavailable`) rather
/// than the network as down. `started` is tokio's clock, so paused-time tests can drive it.
pub(crate) fn used_whole_timeout(started: tokio::time::Instant, timeout_secs: u64) -> bool {
    started.elapsed() + TIMEOUT_SLACK >= Duration::from_secs(timeout_secs)
}

/// How long a request to a server on the person's own computer or network may take at least: a model loaded from scratch
/// (read from disk into memory, a first request that also warms it up) can take minutes. Hosted services keep their short
/// limits; a call to one that takes minutes really is stuck.
pub(crate) const LOCAL_MIN_TIMEOUT_SECS: u64 = 10 * 60;
/// …and at most this, however slow a model has been (the same bound as a ComfyUI job's ceiling, roughly).
const LOCAL_MAX_TIMEOUT_SECS: u64 = 60 * 60;
/// How many times its learned estimate a local call may take before it's given up on.
const LOCAL_TIMEOUT_ESTIMATES: f64 = 3.0;

/// The time a local server gets for one call: 10 minutes, or 3 × how long this call usually takes on this computer
/// (`expected_secs`, from the performance history) when that's longer, never more than an hour. The history only ever
/// lengthens it: a model that is usually quick still has to be allowed a cold start.
pub(crate) fn local_timeout_secs(expected_secs: Option<f64>) -> u64 {
    let learned = expected_secs.filter(|seconds| seconds.is_finite() && *seconds > 0.0).map_or(0.0, |seconds| seconds * LOCAL_TIMEOUT_ESTIMATES);
    (learned.ceil() as u64).clamp(LOCAL_MIN_TIMEOUT_SECS, LOCAL_MAX_TIMEOUT_SECS)
}

/// What the composer asks a text model for: candidate concepts as JSON matching `composer::schema`.
#[derive(Debug, Clone)]
pub struct ComposeRequest {
    /// Model ID; empty = provider default.
    pub model: String,
    /// Full instructions (the composer builds them; providers don't add their own).
    pub system: String,
    pub user: String,
    /// JSON Schema for `{"candidates": [ … ]}` (`composer::schema`).
    pub schema: serde_json::Value,
    /// 0–2, where the model supports it.
    pub temperature: f32,
    /// The same inputs in structured form, for providers that don't read instructions (Demo).
    pub inputs: ComposeInputs,
    /// How long this writer usually takes here (the learned estimate, seconds). Servers on the person's own computer
    /// (Ollama, OpenAI-compatible) wait max(10 minutes, 3 × this); hosted services ignore it.
    pub expected_secs: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComposeInputs {
    pub musts: Vec<String>,
    pub maybes: Vec<String>,
    pub avoids: Vec<String>,
    pub surprise: f32,
    /// How many candidates are asked for.
    pub candidates: usize,
    /// The original concept, for echo requests (candidates then include `echo_note`).
    pub echo_of: Option<crate::model::Concept>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ComposeResponse {
    /// The model's structured output as JSON (the composer parses it; see `composer::parse`).
    pub output: serde_json::Value,
    pub usage: Usage,
    /// The model that actually answered (providers may resolve aliases).
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub model: String,
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    pub quality: ImageQuality,
    pub seed: Option<u64>,
    /// Told about sampler steps as they happen, by providers that know them (ComfyUI; Demo with a delay).
    pub progress: Option<PaintProgress>,
    /// How long this painting usually takes here (the learned estimate, seconds). Providers that run local jobs
    /// (ComfyUI) give up only past max(30 minutes, 3 × this); without one, past 30 minutes.
    pub expected_secs: Option<f64>,
}

/// Sampler steps done so far of the whole workflow's, as a painter reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaintStep {
    pub done: u32,
    pub total: u32,
}

/// Where a painter reports its steps (called on the task running the painting).
#[derive(Clone)]
pub struct PaintProgress(pub Arc<dyn Fn(PaintStep) + Send + Sync>);

impl PaintProgress {
    pub fn report(&self, step: PaintStep) {
        (self.0)(step);
    }
}

impl std::fmt::Debug for PaintProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PaintProgress")
    }
}

#[derive(Debug, Clone)]
pub struct ImageResponse {
    pub bytes: Vec<u8>,
    /// As the provider claims it; the image pipeline sniffs the real format.
    pub mime: String,
    pub model: String,
    /// Cost the provider reported, when it does; otherwise the price table is used.
    pub reported_cost_microusd: Option<u64>,
}

/// The sizes a model can make. The pipeline picks the one closest to the display's aspect, largest first.
#[derive(Debug, Clone)]
pub struct ImageCapabilities {
    pub sizes: Vec<(u32, u32)>,
    /// Free-form sizes (ComfyUI, many local servers): any multiple of `step` up to `max_side`.
    pub free_size: Option<FreeSize>,
}

#[derive(Debug, Clone, Copy)]
pub struct FreeSize {
    pub step: u32,
    pub max_side: u32,
    pub max_pixels: u64,
}

#[async_trait]
pub trait TextProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn default_model(&self) -> &str;
    /// Whether this provider can be called now as far as AutoPaper can tell without calling it: a hosted service
    /// needs its key (`MissingKey`, or `InvalidKey` for one that couldn't be a key). Checked before anything is
    /// spent (the engine's `prepare`), so a missing painting key isn't found only after a paid idea.
    fn check_ready(&self) -> Result<()> {
        Ok(())
    }
    async fn compose(&self, request: ComposeRequest) -> Result<ComposeResponse>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    /// A read-only service check before any paid work. Local Demo providers need no network.
    async fn check_available(&self) -> Result<()> {
        self.list_models().await.map(|_| ())
    }
}

#[async_trait]
pub trait ImageProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;
    fn default_model(&self) -> &str;
    /// Whether this provider can be asked for `model` (blank: its default) at all, checked before anything is
    /// spent (the engine's `prepare`). ComfyUI's own workflows exist for a few models only (`PaintingFailed`
    /// otherwise); providers that pass any model name on and let the service decide say yes.
    fn check_model(&self, _model: &str) -> Result<()> {
        Ok(())
    }
    /// As `TextProvider::check_ready`: a hosted painter needs its key.
    fn check_ready(&self) -> Result<()> {
        Ok(())
    }
    fn capabilities(&self, model: &str) -> ImageCapabilities;
    /// Sampler steps a painting with `model` (blank: the default) runs, when the provider knows them (ComfyUI:
    /// the `steps` of the workflow's samplers). Recorded with each timing and used to scale estimates.
    fn steps(&self, _model: &str) -> Option<u32> {
        None
    }
    async fn generate(&self, request: ImageRequest) -> Result<ImageResponse>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    /// A read-only service check before any paid work. Local Demo providers need no network.
    async fn check_available(&self) -> Result<()> {
        self.list_models().await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_server_gets_ten_minutes_unless_its_history_says_longer() {
        assert_eq!(local_timeout_secs(None), 600, "nothing learned yet: the cold-start allowance");
        assert_eq!(local_timeout_secs(Some(20.0)), 600, "a quick history never shortens it");
        assert_eq!(local_timeout_secs(Some(200.0)), 600, "3 × 200 s is exactly 10 minutes");
        assert_eq!(local_timeout_secs(Some(400.0)), 1200, "3 × 400 s");
        assert_eq!(local_timeout_secs(Some(400.4)), 1202, "rounded up to whole seconds");
        assert_eq!(local_timeout_secs(Some(100_000.0)), 3600, "never more than an hour");
    }

    #[test]
    fn nonsense_estimates_are_ignored() {
        for odd in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -5.0] {
            assert_eq!(local_timeout_secs(Some(odd)), 600, "{odd}");
        }
    }
}
