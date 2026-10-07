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
}
