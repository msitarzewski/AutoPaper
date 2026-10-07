//! ComfyUI (local): images from a workflow template in API format.
//!
//! AutoPaper's own workflows ship in `core/resources/comfyui/` (`include_str!`), one per supported model, each the
//! graph that painted the website's examples on the user's Mac: Z-Image Turbo (the default, Apache-2.0), Krea 2
//! Turbo and Qwen-Image 2.1. Or the person supplies their own (`Settings::comfyui_workflow`, with `{{prompt}}`,
//! `{{width}}`, `{{height}}`, `{{seed}}` placeholders). Flow: POST /prompt {prompt, client_id} → poll GET
//! /history/{prompt_id} (1 s, then 2 s) until outputs or an error status → GET /view?filename&subfolder&type. Base
//! URL default http://127.0.0.1:8188 (`HostPolicy::UserEndpoint`). Free-form sizes, multiples of 16, within the
//! limits of the diffusion model the workflow loads (its loader node's `unet_name`/`ckpt_name`, bundled template
//! or the person's own): see `MODEL_LIMITS`. The engine asks for the display's aspect within them and scales the
//! picture up per display. Shapes: `core/tests/fixtures/comfyui/` (recorded from real runs).
//!
//! **Progress and timeouts (2026-10-06).** Before queueing, `generate` opens ComfyUI's `/ws?clientId=<id>` (the
//! `client_id` it queues the job with: ComfyUI sends a job's progress only to the client that queued it) and reads
//! its events while it polls: `progress` {value, max, node} for sampler steps, `executing`/`executed`/
//! `execution_*` for nodes. Steps go to `ImageRequest::progress` as done/total of the whole workflow (total: the
//! `steps` of every sampler in the graph — `ImageProvider::steps` — or what the server reports, whichever is
//! more). /history stays the authority on how the job ended: without the socket (an https server, a broken
//! connection) the job is still followed and finished by polling, only without step progress and stall
//! detection. A local job fails only when it stops making progress — once it runs, no event for 3 minutes (or
//! 4 × its slowest step) after a sampler step, or 10 minutes while a node without steps runs (loading a model,
//! encoding, decoding) — or runs past max(30 minutes, 3 × `ImageRequest::expected_secs`) from the start; either
//! way `ProviderUnavailable` `TimedOut`, and the job is stopped. Waiting in ComfyUI's queue behind someone else's
//! job is never a stall (the ceiling still applies).
//!
//! A diffusion model only paints with its own text encoder, VAE and sampling (2026-10-06: Qwen-Image 2.1 run
//! through Z-Image Turbo's graph failed in KSampler, "Given normalized_shape=[4096], … got input of size[1, 95,
//! 2560]"), so a bundled workflow is never given another model: the chosen model (a diffusion model file) picks
//! the workflow made for it, blank picks the default, and a model without one is `InvalidInput` (`Other`) before
//! anything is spent (`check_model`). Each bundled template is filled by its node map (`*.map.json`: `name`,
//! `placeholders.{prompt,width,height,seed}` → node + input, `output.node`), never by text replacement. The map
//! declares what the workflow needs on the server (`requires`: diffusion model, text encoder, VAE, custom nodes);
//! `list_models` reads GET /object_info once and offers a bundled workflow's model only when those files are
//! among the stock loaders' choices (UNETLoader, CLIPLoader, VAELoader), every node class of its graph exists,
//! and every other choice it makes (a CLIP type, a sampler) is one the server offers — what ComfyUI itself would
//! validate. Listed: the default first, then by name, each named in words ("Krea 2 Turbo").
//!
//! A person's own template is filled by literal placeholder replacement: `{{prompt}}` becomes the JSON-escaped
//! prompt (a whole `"{{prompt}}"` string becomes the prompt string), and `{{width}}`/`{{height}}`/`{{seed}}`
//! become numbers (quoted or not); its model is never overridden, and `list_models` lists its loader's choices. A
//! template's model is its first loader node's `ckpt_name`, else `unet_name`.
//!
//! With no seed requested, a random 53-bit one is used; a requested seed keeps its low 53 bits. Validation errors
//! from /prompt → `PaintingFailed` with the model in words and the failing node's message (redacted); a failed
//! run → `PaintingFailed` "ComfyUI couldn't paint with <model>: the <class> node (<id>) failed: <exception>"; so
//! do a run that saved no image and a model AutoPaper has no workflow for. Not transient: the engine treats them
//! as a setup problem, not something waiting fixes (`engine::needs_setup`). A run stopped in ComfyUI →
//! `ProviderUnavailable` `Stopped`; nothing answering → `ProviderUnavailable` `NotRunning` "ComfyUI isn't running at
//! …" (one call using its whole timeout: `TimedOut` "didn't answer"). A person's workflow that can't be used →
//! `InvalidInput` (`WorkflowNeedsPrompt`, `WorkflowNotApiFormat`, `WorkflowInvalid`).
//!
//! When `generate` gives up on a job that may still be running — a stall or the ceiling, a poll that failed, or the
//! engine dropping the call because the person cancelled — the job is stopped: `POST /api/jobs/{id}/cancel`
//! (in ComfyUI since June 2026, PR #14493 — the user's 0.36.0 has it — and documented in its openapi.yaml:
//! interrupts that job if it's running, dequeues it if it's waiting, and nothing else). A server without that route (404/405) only has the job removed from its queue
//! (`POST /queue {"delete": [id]}`): its `/interrupt` stops whatever is running, someone else's job too, so
//! it's never used.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason, Result};
use crate::model::{ModelInfo, ProviderKind};
use crate::net::{HostPolicy, error_for_status, redact};
use crate::ports::{HttpClient, HttpMethod, HttpRequest, HttpResponse, Socket};

use super::{
    FreeSize, ImageCapabilities, ImageProvider, ImageRequest, ImageResponse, PaintProgress, PaintStep, used_whole_timeout,
};

const KIND: ProviderKind = ProviderKind::ComfyUi;
/// The least time a job gets, from the start of `generate` (the download has its own limit)…
const MIN_CEILING: Duration = Duration::from_secs(30 * 60);
/// …or this many times its learned estimate, when that's longer.
const CEILING_ESTIMATES: f64 = 3.0;
/// After a sampler step, the job has stalled when nothing more happens for this long…
const STEP_STALL: Duration = Duration::from_secs(3 * 60);
/// …or for this many times its slowest step so far, when that's longer (a slow computer's steps).
const STEP_STALL_GAPS: u32 = 4;
/// While a node without steps runs (loading a model, encoding the prompt, decoding the picture).
const NODE_STALL: Duration = Duration::from_secs(10 * 60);
/// Connecting to /ws (it's best effort: polling follows the job without it).
const SOCKET_TIMEOUT_SECS: u64 = 5;
/// One /ws message (binary preview images are read and skipped; a larger one closes the socket).
const MAX_SOCKET_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const CALL_TIMEOUT_SECS: u64 = 30;
const VIEW_TIMEOUT_SECS: u64 = 120;
/// History entries carry the submitted graph back.
const MAX_JSON_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const FIRST_POLL: Duration = Duration::from_secs(1);
const NEXT_POLL: Duration = Duration::from_secs(2);
/// Seeds stay below 2^53 so every JSON reader (and ComfyUI's web UI) keeps them exact.
const SEED_BITS: u32 = 53;
const SEED_MASK: u64 = (1 << SEED_BITS) - 1;
/// Error details shown in logs are cut to this many characters.
const DETAIL_CHARS: usize = 300;
/// The model name reported for a person's template whose loader node isn't recognised.
const CUSTOM_MODEL: &str = "custom workflow";
/// Loader inputs that name the model file, in the order they're looked for.
const MODEL_INPUTS: [&str; 2] = ["ckpt_name", "unet_name"];
/// Sizes asked of ComfyUI are multiples of 16 (every model below needs it).
const SIZE_STEP: u32 = 16;

/// What one diffusion model can paint, as AutoPaper asks it: the longest side and the most pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelLimits {
    /// For logs and tests.
    pub name: &'static str,
    pub max_side: u32,
    pub max_pixels: u64,
}

/// Apple silicon (MPS): ComfyUI's VAEDecode fails with "MPSGraph does not support tensor dims larger than INT_MAX"
/// once the VAE decoder's self-attention matrix, ((W/8)·(H/8))² elements for an 8× VAE, passes INT_MAX: ComfyUI
/// slices it only when free memory runs short, so a large enough machine computes it whole. Whole, it fits up
/// to 46,340 latent tokens, i.e. 46,340 × 64 = 2,965,760 pixels. Measured 2026-10-06 on an M5 Max (128 GB),
/// ComfyUI 0.36.0, with each model's real graph: Z-Image Turbo and Krea 2 Turbo decode 2048×1440 (46,080
/// tokens) and fail at 2048×1456 (46,592) and 2048×2048; Z-Image Turbo's 2544×1632 failed in the macOS app's run
/// and passed in a later one (ComfyUI happened to slice it), so sizes past the bound are unreliable, not
/// impossible. AutoPaper can't tell which GPU a ComfyUI server uses when it picks a size, so models with an 8×
/// VAE are held to this everywhere (a CUDA server could paint more; the renderer upscales either way).
const MPS_VAE8_MAX_PIXELS: u64 = 46_340 * 64;

/// Per model, matched by the loader's file name (lower-cased, `-`/`.`/space read as `_`), first match wins.
/// Sources (each checked 2026-10-06):
/// - Z-Image Turbo: the Z-Image README (github.com/Tongyi-MAI/Z-Image, commit 26f23eda) gives the family's
///   range as "512×512 to 2048×2048 (total pixel area, any aspect ratio)"; the Turbo model card
///   (huggingface.co/Tongyi-MAI/Z-Image-Turbo) states no limit (its example is 1024×1024); the official Turbo
///   demo (huggingface.co/spaces/Tongyi-MAI/Z-Image-Turbo, app.py at 768cb50d) offers sizes up to 2048 a side
///   (2048×1152 16:9, 2016×864 21:9, 1536×1536). So: sides ≤ 2048, pixels ≤ 2048² — stepped down to the Apple
///   silicon VAE bound above (it decodes with the 8× `ae.safetensors`).
/// - Qwen-Image 2.1: the model card (huggingface.co/Qwen/Qwen-Image-2.1, d26bb612) lists its supported sizes:
///   2048×2048, 2400×1792, 2528×1696, 2752×1536 (and their portrait twins). So: sides ≤ 2752, pixels ≤
///   2400×1792 = 4,300,800. Its VAE is 16× (ComfyUI `sd.py`: Wan 2.2 layout, `downscale_ratio = 16`), so its
///   attention sees a quarter of the tokens: measured decoding 2752×1536 and 2048×2048 fine on the M5 Max.
/// - Krea 2 Turbo: the Krea 2 README (github.com/krea-ai/krea-2, commit db3984fb) says Turbo "can generate images
///   from 1k ~ 2k resolution", `--width`/`--height` "1024 ~ 2048", "padded up to a multiple of 16"; the model card
///   (huggingface.co/krea/Krea-2-Turbo, gated; its usage section is public) runs it at 2048×2048. So: sides ≤
///   2048, pixels ≤ 2048² — stepped down to the Apple silicon bound (it decodes with the 8× Qwen-Image VAE).
/// - Krea 2 Raw: the same README: "trained to generate upto 1k resolution". So: pixels ≤ 1024².
/// - Anything else (another model, a workflow whose loader isn't recognised): about 2 MP, sides ≤ 2048
///   (1920×1088, the size the bundled template was first tested at).
const MODEL_LIMITS: &[(&[&str], ModelLimits)] = &[
    (
        &["z_image_turbo", "zimage_turbo"],
        ModelLimits { name: "Z-Image Turbo", max_side: 2048, max_pixels: MPS_VAE8_MAX_PIXELS },
    ),
    (&["qwen_image_2_1"], ModelLimits { name: "Qwen-Image 2.1", max_side: 2752, max_pixels: 2400 * 1792 }),
    (
        &["krea2_turbo", "krea_2_turbo"],
        ModelLimits { name: "Krea 2 Turbo", max_side: 2048, max_pixels: MPS_VAE8_MAX_PIXELS },
    ),
    (&["krea2", "krea_2"], ModelLimits { name: "Krea 2 Raw", max_side: 2048, max_pixels: 1024 * 1024 }),
];

/// Limits for a model this table doesn't know.
pub const UNKNOWN_MODEL_LIMITS: ModelLimits = ModelLimits { name: "unknown model", max_side: 2048, max_pixels: 1920 * 1088 };

/// The limits of the diffusion model in `file` (a loader's `unet_name`/`ckpt_name`, folders allowed), else
/// `UNKNOWN_MODEL_LIMITS`.
pub fn model_limits(file: &str) -> ModelLimits {
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file).to_lowercase().replace(['-', '.', ' '], "_");
    MODEL_LIMITS
        .iter()
        .find(|(patterns, _)| patterns.iter().any(|pattern| name.contains(pattern)))
        .map_or(UNKNOWN_MODEL_LIMITS, |(_, limits)| *limits)
}

/// AutoPaper's own workflows, (template, node map), the default first.
const BUNDLED_SOURCES: [(&str, &str); 3] = [
    (
        include_str!("../../resources/comfyui/z-image-turbo-t2i.json"),
        include_str!("../../resources/comfyui/z-image-turbo-t2i.map.json"),
    ),
    (
        include_str!("../../resources/comfyui/krea-2-turbo-t2i.json"),
        include_str!("../../resources/comfyui/krea-2-turbo-t2i.map.json"),
    ),
    (
        include_str!("../../resources/comfyui/qwen-image-2.1-t2i.json"),
        include_str!("../../resources/comfyui/qwen-image-2.1-t2i.map.json"),
    ),
];

/// The bundled templates, parsed once.
static BUNDLED: LazyLock<std::result::Result<Vec<Mapped>, String>> =
    LazyLock::new(|| BUNDLED_SOURCES.iter().map(|(workflow, node_map)| Mapped::parse(workflow, node_map)).collect());

/// Where a ComfyUI server lists the files of each kind a bundled workflow declares (`requires`): the stock loader
/// node, its input, and the kind in words (for logs).
const FILE_LOADERS: [(&str, &str, &str); 3] =
    [("UNETLoader", "unet_name", "diffusion model"), ("CLIPLoader", "clip_name", "text encoder"), ("VAELoader", "vae_name", "VAE")];

pub struct ComfyUi {
    http: Arc<dyn HttpClient>,
    base_url: String,
    /// The person's template, or `None` for the bundled default.
    workflow: Option<String>,
    /// The template's own model (its loader node's file name).
    model: String,
    /// How long a running job may go without an event (`STEP_STALL`, `NODE_STALL`; live tests shorten them).
    stall: StallWindows,
}

/// See `STEP_STALL` and `NODE_STALL`.
#[derive(Debug, Clone, Copy)]
struct StallWindows {
    step: Duration,
    node: Duration,
}

impl ComfyUi {
    /// A blank `workflow` counts as none.
    pub fn new(http: Arc<dyn HttpClient>, base_url: String, workflow: Option<String>) -> Self {
        let workflow = workflow.filter(|template| !template.trim().is_empty());
        let model = match &workflow {
            Some(template) => fill_placeholders(template, "", 1024, 576, 0)
                .ok()
                .and_then(|graph| loader(&graph).map(|found| found.model))
                .unwrap_or_else(|| CUSTOM_MODEL.to_string()),
            None => bundled_model().unwrap_or_default(),
        };
        Self { http, base_url, workflow, model, stall: StallWindows { step: STEP_STALL, node: NODE_STALL } }
    }

    fn base(&self) -> &str {
        self.base_url.trim().trim_end_matches('/')
    }

    /// `base` + path segments, each percent-encoded.
    fn endpoint(&self, segments: &[&str]) -> Result<url::Url> {
        let not_an_address =
            || AutoPaperError::invalid_input(InvalidInputReason::AddressInvalid, "that isn't a complete web address");
        let mut url = url::Url::parse(self.base()).map_err(|_| not_an_address())?;
        url.path_segments_mut().map_err(|_| not_an_address())?.pop_if_empty().extend(segments);
        Ok(url)
    }

    async fn send(&self, method: HttpMethod, url: url::Url, body: Option<&Value>, timeout_secs: u64, max: usize) -> Result<HttpResponse> {
        let mut headers = vec![("accept".to_string(), "application/json".to_string())];
        if body.is_some() {
            headers.push(("content-type".to_string(), "application/json".to_string()));
        }
        let request = HttpRequest {
            method,
            url: url.to_string(),
            policy: HostPolicy::UserEndpoint,
            headers,
            body: body.map(Value::to_string).map(String::into_bytes),
            timeout_secs,
            max_response_bytes: max,
        };
        let started = tokio::time::Instant::now();
        self.http.send(request).await.map_err(|error| match error {
            AutoPaperError::Offline if used_whole_timeout(started, timeout_secs) => AutoPaperError::unavailable(
                KIND,
                ProviderUnavailableReason::TimedOut,
                format!("ComfyUI didn't answer within {timeout_secs} s"),
            ),
            AutoPaperError::Offline => AutoPaperError::unavailable(
                KIND,
                ProviderUnavailableReason::NotRunning,
                format!("ComfyUI isn't running at {}", self.base()),
            ),
            other => other.for_provider(KIND),
        })
    }

    async fn get_json(&self, segments: &[&str]) -> Result<Value> {
        let response = self.send(HttpMethod::Get, self.endpoint(segments)?, None, CALL_TIMEOUT_SECS, MAX_JSON_RESPONSE_BYTES).await?;
        if let Some(error) = error_for_status(KIND, &response) {
            return Err(error);
        }
        serde_json::from_slice(&response.body).map_err(|_| invalid(format!("ComfyUI's answer to /{} wasn't JSON", segments.join("/"))))
    }

    /// The graph to run: the person's own workflow, else the bundled one made for the chosen model.
    fn graph_for(&self, request: &ImageRequest, seed: u64) -> Result<Graph> {
        match &self.workflow {
            Some(template) => Ok(Graph {
                nodes: fill_placeholders(template, &request.prompt, request.width, request.height, seed)?,
                model: self.model.clone(),
                label: own_label(&self.model),
                model_name: own_model_name(&self.model),
                output: None,
            }),
            None => {
                let workflow = bundled_for(&request.model)?;
                Ok(Graph {
                    nodes: workflow.fill(&request.prompt, request.width, request.height, seed)?,
                    model: workflow.model().unwrap_or_default(),
                    label: workflow.map.name.clone(),
                    model_name: workflow.map.name.clone(),
                    output: workflow.map.output.as_ref().map(|output| output.node.clone()),
                })
            }
        }
    }

    /// POST /prompt as `client_id` (the /ws client that hears its progress); the queued job's id.
    async fn queue(&self, graph: &Graph, client_id: &str) -> Result<String> {
        let body = json!({ "prompt": graph.nodes, "client_id": client_id });
        let response =
            self.send(HttpMethod::Post, self.endpoint(&["prompt"])?, Some(&body), CALL_TIMEOUT_SECS, MAX_JSON_RESPONSE_BYTES).await?;
        if response.status == 400 {
            return Err(rejection(&response.body, graph));
        }
        if let Some(error) = error_for_status(KIND, &response) {
            return Err(error);
        }
        let queued: Queued =
            serde_json::from_slice(&response.body).map_err(|_| invalid("ComfyUI's answer to /prompt wasn't the expected JSON"))?;
        let id_is_safe =
            !queued.prompt_id.is_empty() && queued.prompt_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !id_is_safe {
            return Err(invalid("ComfyUI returned an unexpected job id"));
        }
        Ok(queued.prompt_id)
    }

    /// `/ws?clientId=…`, or `None` when it can't be opened (logged): progress then comes only from polling.
    async fn open_events(&self, client_id: &str) -> Option<Box<dyn Socket>> {
        let mut url = self.endpoint(&["ws"]).ok()?;
        url.query_pairs_mut().append_pair("clientId", client_id);
        let request = HttpRequest {
            method: HttpMethod::Get,
            url: url.to_string(),
            policy: HostPolicy::UserEndpoint,
            headers: Vec::new(),
            body: None,
            timeout_secs: SOCKET_TIMEOUT_SECS,
            max_response_bytes: MAX_SOCKET_MESSAGE_BYTES,
        };
        match self.http.open_socket(request).await {
            Ok(socket) => Some(socket),
            Err(error) => {
                tracing::info!(%error, "no progress events from ComfyUI; following the job by polling only");
                None
            }
        }
    }

    /// Polls /history/{id} (after 1 s, then every 2 s, and at once when the socket says the job ended) until the
    /// job has an image or failed, while reading the socket's events into `watch`; gives up at `deadline` or when
    /// `watch` says the job stalled. Once the job has ended (an image, an error, or done without an image),
    /// `running` is disarmed: there's nothing left to stop.
    async fn wait_for_image(
        &self,
        id: &str,
        graph: &Graph,
        deadline: tokio::time::Instant,
        mut socket: Option<Box<dyn Socket>>,
        watch: &mut JobWatch,
        running: &mut StopOnDrop,
    ) -> Result<ImageRef> {
        let mut next_poll = tokio::time::Instant::now() + FIRST_POLL;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                let minutes = (deadline - watch.queued).as_secs().div_ceil(60);
                return Err(AutoPaperError::unavailable(
                    KIND,
                    ProviderUnavailableReason::TimedOut,
                    format!("ComfyUI didn't finish within {minutes} minutes"),
                ));
            }
            if let Some(window) = watch.stalled(now) {
                return Err(AutoPaperError::unavailable(
                    KIND,
                    ProviderUnavailableReason::TimedOut,
                    format!("ComfyUI made no progress for {} s", window.as_secs()),
                ));
            }
            if now >= next_poll {
                if let Some(ended) = self.poll(id, graph, running).await? {
                    return ended;
                }
                next_poll = tokio::time::Instant::now() + NEXT_POLL;
                continue;
            }
            let wake = [Some(next_poll), Some(deadline), watch.stall_at()].into_iter().flatten().min().unwrap_or(next_poll);
            let event = match socket.as_mut() {
                Some(events) => tokio::select! {
                    biased;
                    message = events.next_text() => Some(message),
                    () = tokio::time::sleep_until(wake) => None,
                },
                None => {
                    tokio::time::sleep_until(wake).await;
                    None
                }
            };
            match event {
                Some(Ok(Some(text))) => {
                    if watch.on_message(&text, tokio::time::Instant::now()) {
                        next_poll = tokio::time::Instant::now();
                    }
                }
                Some(Ok(None) | Err(_)) => {
                    tracing::info!("ComfyUI's progress events stopped; following the job by polling only");
                    socket = None;
                    watch.lost_events();
                }
                None => {}
            }
        }
    }

    /// One look at /history/{id}: `Some` with how the job ended, `None` while it hasn't.
    async fn poll(&self, id: &str, graph: &Graph, running: &mut StopOnDrop) -> Result<Option<Result<ImageRef>>> {
        let history = self.get_json(&["history", id]).await?;
        let Some(entry) = history.get(id) else { return Ok(None) };
        if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
            running.disarm();
            return Ok(Some(Err(run_failure(entry, graph))));
        }
        if let Some(image) = find_image(entry, &graph.nodes, graph.output.as_deref()) {
            running.disarm();
            return Ok(Some(Ok(image)));
        }
        if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
            running.disarm();
            let detail = format!("the ComfyUI workflow for {} finished without saving an image", graph.label);
            return Ok(Some(Err(painting_failed(graph, &detail))));
        }
        Ok(None)
    }

    /// What stops job `id` (see the module docs); `None` when no URL can be built from the base URL.
    fn job_stop(&self, id: &str) -> Option<JobStop> {
        Some(JobStop {
            http: self.http.clone(),
            cancel_url: self.endpoint(&["api", "jobs", id, "cancel"]).ok()?.to_string(),
            queue_url: self.endpoint(&["queue"]).ok()?.to_string(),
            id: id.to_string(),
        })
    }

    /// GET /view?filename=…&subfolder=…&type=… (query properly encoded).
    async fn download(&self, image: &ImageRef) -> Result<(Vec<u8>, String)> {
        let mut url = self.endpoint(&["view"])?;
        url.query_pairs_mut()
            .append_pair("filename", &image.filename)
            .append_pair("subfolder", &image.subfolder)
            .append_pair("type", &image.kind);
        let response = self.send(HttpMethod::Get, url, None, VIEW_TIMEOUT_SECS, MAX_IMAGE_BYTES).await?;
        if let Some(error) = error_for_status(KIND, &response) {
            return Err(error);
        }
        if response.body.is_empty() {
            return Err(invalid("ComfyUI returned an empty image"));
        }
        let mime = response.header("content-type").unwrap_or("image/png").to_string();
        Ok((response.body, mime))
    }
}

#[async_trait]
impl ImageProvider for ComfyUi {
    fn kind(&self) -> ProviderKind {
        KIND
    }

    /// The default bundled workflow's model (for a person's template: its loader's model file, else
    /// "custom workflow").
    fn default_model(&self) -> &str {
        &self.model
    }

    /// With AutoPaper's workflows, `model` must be one a bundled workflow is made for (blank: the default); a
    /// person's own workflow loads its own model, whatever is chosen.
    fn check_model(&self, model: &str) -> Result<()> {
        match &self.workflow {
            Some(_) => Ok(()),
            None => bundled_for(model).map(|_| ()),
        }
    }

    /// Multiples of 16 within the limits of the model the workflow will load (`model_limits`): the person's own
    /// workflow always loads its own model; a bundled one loads the chosen model (its workflow's), else the
    /// default's.
    fn capabilities(&self, model: &str) -> ImageCapabilities {
        let loads = match (&self.workflow, model.trim()) {
            (None, chosen) if !chosen.is_empty() => chosen,
            _ => self.model.as_str(),
        };
        let limits = model_limits(loads);
        ImageCapabilities {
            sizes: Vec::new(),
            free_size: Some(FreeSize { step: SIZE_STEP, max_side: limits.max_side, max_pixels: limits.max_pixels }),
        }
    }

    /// Sampler steps of the graph `model` paints with (see `sampler_steps`); `None` when there's no such graph.
    fn steps(&self, model: &str) -> Option<u32> {
        let graph = match &self.workflow {
            Some(template) => fill_placeholders(template, "", 1024, 576, 0).ok()?,
            None => bundled_for(model).ok()?.graph.clone(),
        };
        Some(sampler_steps(&graph)).filter(|steps| *steps > 0)
    }

    async fn generate(&self, request: ImageRequest) -> Result<ImageResponse> {
        if request.prompt.trim().is_empty() {
            return Err(AutoPaperError::invalid_input(InvalidInputReason::Other, "the image prompt is empty"));
        }
        let started = tokio::time::Instant::now();
        let deadline = started + ceiling(request.expected_secs);
        // Every seed stays below 2^53, the caller's too (their low bits).
        let seed = request.seed.map_or_else(|| rand::random::<u64>() >> (64 - SEED_BITS), |seed| seed & SEED_MASK);
        let graph = self.graph_for(&request, seed)?;
        let client_id = uuid::Uuid::now_v7().to_string();
        // Opened before the job is queued, so none of its events are missed.
        let socket = self.open_events(&client_id).await;
        let id = self.queue(&graph, &client_id).await?;
        // From here until the job ends, dropping this call (the person cancelled) stops the job.
        let mut running = StopOnDrop(self.job_stop(&id));
        let mut watch = JobWatch::new(&id, started, sampler_steps(&graph.nodes), request.progress.clone());
        watch.windows = self.stall;
        let image = match self.wait_for_image(&id, &graph, deadline, socket, &mut watch, &mut running).await {
            Ok(image) => image,
            Err(error) => {
                // Timed out, or the job's state couldn't be read: it may still be painting.
                if let Some(stop) = running.disarm() {
                    stop.run().await;
                }
                return Err(error);
            }
        };
        let (bytes, mime) = self.download(&image).await?;
        Ok(ImageResponse { bytes, mime, model: graph.model, reported_cost_microusd: None })
    }

    /// With AutoPaper's workflows: the models whose bundled workflow this server can run (one GET /object_info),
    /// named in words, the default first, then by name. With the person's own workflow: its loader's choices.
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let Some(template) = &self.workflow else {
            return self.runnable_bundled_models().await;
        };
        let Some(Loader { class_type, input, .. }) = loader(&fill_placeholders(template, "", 1024, 576, 0)?) else {
            return Ok(Vec::new());
        };
        let info = self.get_json(&["object_info", &class_type]).await?;
        let inputs = info.get(&class_type).and_then(|node| node.get("input"));
        let spec = ["required", "optional"].iter().find_map(|group| inputs?.get(group)?.get(&input));
        Ok(combo_choices(spec).into_iter().map(|name| ModelInfo { display_name: display_name(&name), id: name }).collect())
    }
}

impl ComfyUi {
    /// The bundled workflows' models this server has everything for (see `Mapped::missing`).
    async fn runnable_bundled_models(&self) -> Result<Vec<ModelInfo>> {
        let server = self.get_json(&["object_info"]).await?;
        let server = server.as_object().ok_or_else(|| invalid("ComfyUI's answer to /object_info wasn't the expected JSON"))?;
        let workflows = bundled()?;
        let default = workflows.first().and_then(Mapped::model);
        let mut models = Vec::new();
        for workflow in workflows {
            let (Some(model), missing) = (workflow.model(), workflow.missing(server)) else { continue };
            if missing.is_empty() {
                models.push(ModelInfo { id: model, display_name: workflow.map.name.clone() });
            } else {
                tracing::info!(model = %workflow.map.name, missing = %short(&missing.join(", ")), "ComfyUI can't run this bundled workflow; not offering it");
            }
        }
        models.sort_by(|a, b| (Some(&a.id) != default.as_ref(), &a.display_name).cmp(&(Some(&b.id) != default.as_ref(), &b.display_name)));
        Ok(models)
    }
}

/// A graph ready to queue: its nodes, the model file it loads, the model in words for error details (`label`) and
/// for hosts (`model_name`: `PaintingFailed::model`), and the node whose saved image is the result (bundled
/// workflows name it).
struct Graph {
    nodes: Map<String, Value>,
    model: String,
    label: String,
    model_name: String,
    output: Option<String>,
}

/// A person's own workflow in error details: "your workflow (sd_xl_base_1.0)".
fn own_label(model: &str) -> String {
    if model == CUSTOM_MODEL { "your workflow".to_string() } else { format!("your workflow ({})", display_name(model)) }
}

/// A person's own workflow's model for hosts: its file without the extension, or empty when its loader isn't
/// recognised.
fn own_model_name(model: &str) -> String {
    if model == CUSTOM_MODEL { String::new() } else { display_name(model) }
}

/// How long a job may take in all: max(30 minutes, 3 × its learned estimate).
fn ceiling(expected_secs: Option<f64>) -> Duration {
    let learned = expected_secs.map_or(Duration::ZERO, |seconds| crate::perf::duration_secs(seconds * CEILING_ESTIMATES));
    MIN_CEILING.max(learned)
}

/// The steps of every sampler in a graph: the integer `steps` inputs (KSampler, KSamplerAdvanced, BasicScheduler…).
fn sampler_steps(graph: &Map<String, Value>) -> u32 {
    let steps = graph.values().filter_map(|node| node.get("inputs")?.get("steps")?.as_u64()).sum::<u64>();
    u32::try_from(steps).unwrap_or(u32::MAX)
}

// ── Following a job ───────────────────────────────────────────────────────────────────────────

/// What /ws says about one job: its sampler steps (reported to the engine) and when it last did anything (for
/// stall detection). See the module docs.
struct JobWatch {
    id: String,
    /// When `generate` started (the ceiling counts from here).
    queued: tokio::time::Instant,
    /// The graph's sampler steps.
    graph_steps: u32,
    /// Per node reporting progress: (value, max).
    nodes: BTreeMap<String, (u32, u32)>,
    progress: Option<PaintProgress>,
    reported: Option<PaintStep>,
    /// The last event about this job; `None` until it runs (waiting in the queue is never a stall).
    last_activity: Option<tokio::time::Instant>,
    /// That event was a sampler step (else a node without steps is running).
    stepping: bool,
    last_step_at: Option<tokio::time::Instant>,
    slowest_step: Duration,
    windows: StallWindows,
}

impl JobWatch {
    fn new(id: &str, queued: tokio::time::Instant, graph_steps: u32, progress: Option<PaintProgress>) -> Self {
        Self {
            id: id.to_string(),
            queued,
            graph_steps,
            nodes: BTreeMap::new(),
            progress,
            reported: None,
            last_activity: None,
            stepping: false,
            last_step_at: None,
            slowest_step: Duration::ZERO,
            windows: StallWindows { step: STEP_STALL, node: NODE_STALL },
        }
    }

    /// Reads one /ws message; true when it says the job ended (poll now).
    fn on_message(&mut self, text: &str, now: tokio::time::Instant) -> bool {
        let Ok(message) = serde_json::from_str::<Value>(text) else { return false };
        let Some(data) = message.get("data") else { return false };
        if data.get("prompt_id").and_then(Value::as_str) != Some(self.id.as_str()) {
            return false;
        }
        match message.get("type").and_then(Value::as_str).unwrap_or_default() {
            "progress" => {
                let count = |key: &str| data.get(key).and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0);
                let node = data.get("node").and_then(Value::as_str).unwrap_or_default().to_string();
                if let (Some(value), Some(max)) = (count("value"), count("max"))
                    && max >= 1.0
                {
                    let max = max.min(f64::from(u32::MAX)) as u32;
                    self.nodes.insert(node, ((value.min(f64::from(max))) as u32, max));
                }
                self.active(now, Some(true));
                self.report();
                false
            }
            // The whole job's state on each update: alive, but it says nothing about which node runs.
            "progress_state" => {
                self.active(now, None);
                false
            }
            "executing" => {
                self.active(now, Some(false));
                data.get("node").is_some_and(Value::is_null)
            }
            "execution_success" | "execution_error" | "execution_interrupted" => {
                self.active(now, Some(false));
                true
            }
            _ => {
                self.active(now, Some(false));
                false
            }
        }
    }

    /// Something happened at `now`; `stepping` says whether a sampler step did (`None`: unchanged).
    fn active(&mut self, now: tokio::time::Instant, stepping: Option<bool>) {
        if stepping == Some(true) {
            if let Some(previous) = self.last_step_at {
                self.slowest_step = self.slowest_step.max(now.saturating_duration_since(previous));
            }
            self.last_step_at = Some(now);
        }
        if let Some(stepping) = stepping {
            self.stepping = stepping;
        }
        self.last_activity = Some(now);
    }

    /// The events stopped (the socket closed or broke): nothing can tell a stall any more.
    fn lost_events(&mut self) {
        self.last_activity = None;
    }

    /// Done/total of the whole workflow, once a sampler has reported.
    fn steps(&self) -> Option<PaintStep> {
        if self.nodes.is_empty() {
            return None;
        }
        let done: u32 = self.nodes.values().map(|(value, _)| *value).fold(0, u32::saturating_add);
        let seen: u32 = self.nodes.values().map(|(_, max)| *max).fold(0, u32::saturating_add);
        let total = self.graph_steps.max(seen);
        Some(PaintStep { done: done.min(total), total })
    }

    fn report(&mut self) {
        let steps = self.steps();
        if steps != self.reported
            && let (Some(steps), Some(progress)) = (steps, &self.progress)
        {
            progress.report(steps);
        }
        self.reported = steps;
    }

    /// How long the job may go without an event now.
    fn stall_window(&self) -> Duration {
        if self.stepping { self.windows.step.max(self.slowest_step * STEP_STALL_GAPS) } else { self.windows.node }
    }

    /// When the job counts as stalled if nothing happens before then.
    fn stall_at(&self) -> Option<tokio::time::Instant> {
        self.last_activity.map(|at| at + self.stall_window())
    }

    /// The window it went without an event, once it has stalled.
    fn stalled(&self, now: tokio::time::Instant) -> Option<Duration> {
        self.stall_at().filter(|at| now >= *at).map(|_| self.stall_window())
    }
}

// ── Templates ─────────────────────────────────────────────────────────────────────────────────

/// Where one value goes in the graph.
#[derive(Debug, Clone, Deserialize)]
struct Slot {
    node: String,
    input: String,
}

/// A bundled template's `*.map.json` (other keys there document the template and are ignored).
#[derive(Debug, Clone, Deserialize)]
struct NodeMap {
    /// The model in words ("Krea 2 Turbo"), as Settings lists it and error details name it.
    #[serde(default)]
    name: String,
    placeholders: Placeholders,
    /// The node whose saved image is the result.
    #[serde(default)]
    output: Option<OutputNode>,
    /// What the workflow needs on the server.
    #[serde(default)]
    requires: Requires,
}

/// Files (by the stock loader that lists them: `FILE_LOADERS`) and custom node classes a bundled workflow needs.
#[derive(Debug, Clone, Default, Deserialize)]
struct Requires {
    #[serde(default)]
    diffusion_models: Vec<String>,
    #[serde(default)]
    text_encoders: Vec<String>,
    #[serde(default)]
    vae: Vec<String>,
    #[serde(default)]
    custom_nodes: Vec<String>,
}

impl Requires {
    /// The declared files, in `FILE_LOADERS` order.
    fn files(&self) -> [&[String]; 3] {
        [&self.diffusion_models, &self.text_encoders, &self.vae]
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Placeholders {
    prompt: Slot,
    width: Slot,
    height: Slot,
    seed: Slot,
}

#[derive(Debug, Clone, Deserialize)]
struct OutputNode {
    node: String,
}

/// A template filled by its node map.
#[derive(Debug, Clone)]
struct Mapped {
    graph: Map<String, Value>,
    map: NodeMap,
}

impl Mapped {
    fn parse(workflow: &str, node_map: &str) -> std::result::Result<Self, String> {
        let graph = serde_json::from_str::<Value>(workflow)
            .ok()
            .and_then(|value| api_graph(value).ok())
            .ok_or("the bundled ComfyUI workflow isn't an API-format graph")?;
        let map: NodeMap = serde_json::from_str(node_map).map_err(|error| format!("the bundled ComfyUI node map: {error}"))?;
        let mapped = Self { graph, map };
        let mut probe = mapped.graph.clone();
        for slot in mapped.slots() {
            set_input(&mut probe, slot, Value::Null).map_err(|error| error.to_string())?;
        }
        if let Some(output) = &mapped.map.output
            && !mapped.graph.contains_key(&output.node)
        {
            return Err(format!("the bundled ComfyUI template has no output node {}", output.node));
        }
        Ok(mapped)
    }

    fn slots(&self) -> [&Slot; 4] {
        let placeholders = &self.map.placeholders;
        [&placeholders.prompt, &placeholders.width, &placeholders.height, &placeholders.seed]
    }

    fn model(&self) -> Option<String> {
        self.loader().map(|found| found.model)
    }

    fn loader(&self) -> Option<Loader> {
        loader(&self.graph)
    }

    fn fill(&self, prompt: &str, width: u32, height: u32, seed: u64) -> Result<Map<String, Value>> {
        let mut graph = self.graph.clone();
        let placeholders = &self.map.placeholders;
        set_input(&mut graph, &placeholders.prompt, json!(prompt))?;
        set_input(&mut graph, &placeholders.width, json!(width))?;
        set_input(&mut graph, &placeholders.height, json!(height))?;
        set_input(&mut graph, &placeholders.seed, json!(seed))?;
        Ok(graph)
    }

    /// What this workflow needs that `server` (its GET /object_info) lacks, in words for logs; empty when it can
    /// run there. Checked: each declared file is among its loader's choices (`FILE_LOADERS`), each declared custom
    /// node and each node class of the graph exists, and every other fixed choice of the graph (a CLIP type, a
    /// sampler, a scheduler) is one the server offers — the combo inputs ComfyUI would reject.
    fn missing(&self, server: &Map<String, Value>) -> Vec<String> {
        let mut missing: Vec<String> = Vec::new();
        let mut note = |what: String| {
            if !missing.contains(&what) {
                missing.push(what);
            }
        };
        for ((class_type, input, kind), files) in FILE_LOADERS.iter().zip(self.map.requires.files()) {
            let offered = combo_options(input_spec(server, class_type, input)).unwrap_or_default();
            for file in files.iter().filter(|file| !offered.contains(file)) {
                note(format!("{kind} {file}"));
            }
        }
        let classes = self.graph.values().filter_map(|node| node.get("class_type")?.as_str());
        for class_type in self.map.requires.custom_nodes.iter().map(String::as_str).chain(classes) {
            if !server.contains_key(class_type) {
                note(format!("node {class_type}"));
            }
        }
        for node in self.graph.values() {
            let (Some(class_type), Some(inputs)) =
                (node.get("class_type").and_then(Value::as_str), node.get("inputs").and_then(Value::as_object))
            else {
                continue;
            };
            for (input, value) in inputs {
                let Some(value) = value.as_str() else { continue };
                let is_file = FILE_LOADERS.iter().any(|(loader, file_input, _)| *loader == class_type && file_input == input);
                let Some(choices) = combo_options(input_spec(server, class_type, input)) else { continue };
                if !is_file && !choices.iter().any(|choice| choice == value) {
                    note(format!("{class_type} {input} {value}"));
                }
            }
        }
        missing
    }
}

/// The spec of `class_type`'s input `input` in a GET /object_info answer (required, then optional).
fn input_spec<'a>(server: &'a Map<String, Value>, class_type: &str, input: &str) -> Option<&'a Value> {
    let inputs = server.get(class_type)?.get("input")?;
    ["required", "optional"].iter().find_map(|group| inputs.get(group)?.get(input))
}

fn bundled() -> Result<&'static [Mapped]> {
    BUNDLED.as_deref().map_err(|detail| AutoPaperError::Internal { detail: detail.clone() })
}

/// The bundled workflow for `model` (a diffusion model file; blank: the default). `PaintingFailed` when AutoPaper has
/// none for it, e.g. a model chosen before Settings listed only models with a workflow.
fn bundled_for(model: &str) -> Result<&'static Mapped> {
    let workflows = bundled()?;
    let model = model.trim();
    if model.is_empty() {
        return workflows.first().ok_or_else(|| AutoPaperError::Internal { detail: "no bundled ComfyUI workflow".into() });
    }
    workflows.iter().find(|workflow| workflow.model().as_deref() == Some(model)).ok_or_else(|| AutoPaperError::PaintingFailed {
        provider: KIND,
        model: display_name(model),
        detail: short(&format!("AutoPaper has no ComfyUI workflow for {model}; choose a model Settings lists, or use your own workflow")),
    })
}

/// The default bundled workflow's model file ("z_image_turbo_bf16.safetensors"): ComfyUI's model when the person
/// hasn't chosen one or supplied their own workflow.
pub fn bundled_model() -> Option<String> {
    bundled().ok().and_then(|workflows| workflows.first()).and_then(Mapped::model)
}

/// The default bundled workflow's model in words ("Z-Image Turbo"): how a model picker names its blank choice while
/// ComfyUI can't be asked which models it has (Rust hosts; `default_model` gives the file).
pub fn bundled_model_name() -> Option<String> {
    bundled().ok().and_then(|workflows| workflows.first()).map(|workflow| workflow.map.name.clone())
}

// ── Stopping a job ────────────────────────────────────────────────────────────────────────────

/// Stops one job: `POST /api/jobs/{id}/cancel`, else (no such route) `POST /queue {"delete": [id]}`. Best
/// effort: failures are logged.
struct JobStop {
    http: Arc<dyn HttpClient>,
    cancel_url: String,
    queue_url: String,
    id: String,
}

impl JobStop {
    async fn run(self) {
        let post = |url: String, body: Option<Value>| {
            let mut headers = vec![("accept".to_string(), "application/json".to_string())];
            if body.is_some() {
                headers.push(("content-type".to_string(), "application/json".to_string()));
            }
            HttpRequest {
                method: HttpMethod::Post,
                url,
                policy: HostPolicy::UserEndpoint,
                headers,
                body: body.as_ref().map(Value::to_string).map(String::into_bytes),
                timeout_secs: CALL_TIMEOUT_SECS,
                max_response_bytes: MAX_JSON_RESPONSE_BYTES,
            }
        };
        match self.http.send(post(self.cancel_url, None)).await {
            Ok(response) if (200..300).contains(&response.status) => return,
            Ok(response) => {
                tracing::debug!(status = response.status, "ComfyUI can't cancel one job; removing it from the queue instead");
            }
            Err(error) => {
                tracing::debug!(%error, "couldn't stop a ComfyUI job");
                return;
            }
        }
        let dequeue = post(self.queue_url, Some(json!({ "delete": [self.id] })));
        if let Err(error) = self.http.send(dequeue).await {
            tracing::debug!(%error, "couldn't remove a job from ComfyUI's queue");
        }
    }
}

/// Holds the `JobStop` of a job that is running: dropped while still armed (the engine dropped `generate`
/// because the person cancelled), it stops the job in the background, so ComfyUI doesn't go on painting a
/// wallpaper nobody will see.
struct StopOnDrop(Option<JobStop>);

impl StopOnDrop {
    /// The job ended (or is being stopped by the caller): nothing to do on drop.
    fn disarm(&mut self) -> Option<JobStop> {
        self.0.take()
    }
}

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(stop.run());
        }
    }
}

fn set_input(graph: &mut Map<String, Value>, slot: &Slot, value: Value) -> Result<()> {
    let inputs = graph
        .get_mut(&slot.node)
        .and_then(|node| node.get_mut("inputs"))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| AutoPaperError::Internal { detail: format!("the ComfyUI template has no node {}", slot.node) })?;
    inputs.insert(slot.input.clone(), value);
    Ok(())
}

/// A person's template with its placeholders filled. Numbers first, the prompt last, so placeholder-like
/// text inside the prompt is left alone.
fn fill_placeholders(template: &str, prompt: &str, width: u32, height: u32, seed: u64) -> Result<Map<String, Value>> {
    if !template.contains("{{prompt}}") {
        return Err(AutoPaperError::invalid_input(
            InvalidInputReason::WorkflowNeedsPrompt,
            "your ComfyUI workflow needs a {{prompt}} placeholder",
        ));
    }
    let mut text = template.to_string();
    for (name, value) in [("width", u64::from(width)), ("height", u64::from(height)), ("seed", seed)] {
        let placeholder = format!("{{{{{name}}}}}");
        text = text.replace(&format!("\"{placeholder}\""), &value.to_string()).replace(&placeholder, &value.to_string());
    }
    let quoted = serde_json::to_string(prompt)?;
    let escaped = quoted.strip_prefix('"').and_then(|inner| inner.strip_suffix('"')).unwrap_or(&quoted);
    let text = text.replace("\"{{prompt}}\"", &quoted).replace("{{prompt}}", escaped);
    let value: Value = serde_json::from_str(&text).map_err(|_| {
        AutoPaperError::invalid_input(
            InvalidInputReason::WorkflowInvalid,
            "your ComfyUI workflow isn't valid JSON once its placeholders are filled in",
        )
    })?;
    api_graph(value)
}

/// The node graph of an API-format workflow (also accepted: a whole /prompt body `{"prompt": {…}}`).
fn api_graph(value: Value) -> Result<Map<String, Value>> {
    let not_api = |detail: &str| AutoPaperError::invalid_input(InvalidInputReason::WorkflowNotApiFormat, detail);
    let Value::Object(mut root) = value else {
        return Err(not_api("your ComfyUI workflow isn't in API format; save it with Export (API)"));
    };
    if root.get("nodes").is_some_and(Value::is_array) {
        return Err(not_api("that's a ComfyUI workflow in UI format; save it with Export (API)"));
    }
    if let Some(Value::Object(inner)) = root.get("prompt")
        && !root.values().all(is_node)
    {
        root = inner.clone();
    }
    if root.is_empty() || !root.values().all(is_node) {
        return Err(not_api("your ComfyUI workflow isn't in API format; save it with Export (API)"));
    }
    Ok(root)
}

fn is_node(value: &Value) -> bool {
    value.get("class_type").is_some_and(Value::is_string) && value.get("inputs").is_none_or(Value::is_object)
}

/// A node that loads the model, and the input naming its file.
#[derive(Debug, Clone, PartialEq)]
struct Loader {
    node: String,
    class_type: String,
    input: String,
    model: String,
}

/// The first node (by id) with a `ckpt_name`, else `unet_name`, string input.
fn loader(graph: &Map<String, Value>) -> Option<Loader> {
    MODEL_INPUTS.iter().find_map(|input| {
        let mut ids: Vec<&String> = graph.keys().collect();
        ids.sort_by_key(|id| (id.parse::<u64>().unwrap_or(u64::MAX), id.as_str()));
        ids.into_iter().find_map(|id| {
            let node = graph.get(id)?;
            let model = node.get("inputs")?.get(*input)?.as_str()?;
            let class_type = node.get("class_type")?.as_str()?;
            Some(Loader { node: id.clone(), class_type: class_type.to_string(), input: input.to_string(), model: model.to_string() })
        })
    })
}

/// A combo input's choices: `[[choice, …], {…}]`, or the newer `["COMBO", {"options": [choice, …]}]`; empty for
/// any other input.
fn combo_choices(spec: Option<&Value>) -> Vec<String> {
    combo_options(spec).unwrap_or_default()
}

/// A combo input's choices (possibly none: a loader with no files), or `None` when the input isn't a combo.
fn combo_options(spec: Option<&Value>) -> Option<Vec<String>> {
    let spec = spec?.as_array()?;
    let choices = match spec.first()? {
        Value::Array(choices) => choices,
        Value::String(kind) if kind == "COMBO" => spec.get(1)?.get("options")?.as_array()?,
        _ => return None,
    };
    Some(choices.iter().filter_map(Value::as_str).filter(|s| !s.is_empty()).map(String::from).collect())
}

/// "krea2_turbo_fp8_scaled.safetensors" → "krea2_turbo_fp8_scaled"; folders are kept.
fn display_name(file: &str) -> String {
    [".safetensors", ".ckpt", ".gguf", ".pt", ".pth", ".bin"]
        .iter()
        .find_map(|extension| file.strip_suffix(extension))
        .unwrap_or(file)
        .to_string()
}

// ── Responses ─────────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Queued {
    prompt_id: String,
}

/// One saved image in a history entry.
#[derive(Debug, Clone, PartialEq)]
struct ImageRef {
    filename: String,
    subfolder: String,
    /// ComfyUI's `type`: "output" for saved images, "temp" for previews.
    kind: String,
}

/// The result image: the template's output node first, then SaveImage nodes, then any node; saved
/// ("output") images before previews; then the lowest node id (numerically), so the answer doesn't depend on
/// the order of the JSON object.
fn find_image(entry: &Value, graph: &Map<String, Value>, output: Option<&str>) -> Option<ImageRef> {
    let outputs = entry.get("outputs")?.as_object()?;
    let node_rank = |id: &str| {
        let class_type = graph.get(id).and_then(|node| node.get("class_type")).and_then(Value::as_str);
        if Some(id) == output {
            0
        } else if class_type == Some("SaveImage") {
            1
        } else {
            2
        }
    };
    outputs
        .iter()
        .flat_map(|(id, node)| {
            node.get("images").and_then(Value::as_array).into_iter().flatten().filter_map(move |image| {
                let filename = image.get("filename")?.as_str()?.to_string();
                let subfolder = image.get("subfolder").and_then(Value::as_str).unwrap_or_default().to_string();
                let kind = image.get("type").and_then(Value::as_str).unwrap_or("output").to_string();
                Some((id.as_str(), ImageRef { filename, subfolder, kind }))
            })
        })
        .filter(|(_, image)| !image.filename.is_empty())
        .min_by_key(|(id, image)| (node_rank(id), image.kind != "output", id.parse::<u64>().unwrap_or(u64::MAX), *id))
        .map(|(_, image)| image)
}

/// /prompt's 400 for `graph`: the first node error's message and details, else the overall error.
fn rejection(body: &[u8], graph: &Graph) -> AutoPaperError {
    let label = &graph.label;
    let Ok(json) = serde_json::from_slice::<Value>(body) else {
        return painting_failed(graph, &format!("ComfyUI rejected the workflow for {label} (HTTP 400)"));
    };
    let node_error = json.get("node_errors").and_then(Value::as_object).and_then(|errors| {
        errors.iter().find_map(|(id, node)| {
            let first = node.get("errors")?.as_array()?.first()?;
            let class_type = node.get("class_type").and_then(Value::as_str).unwrap_or("node");
            Some(format!("{class_type} (node {id}): {}", describe(first)))
        })
    });
    let detail = node_error.or_else(|| json.get("error").map(describe)).unwrap_or_else(|| "HTTP 400".into());
    painting_failed(graph, &format!("ComfyUI rejected the workflow for {label}: {detail}"))
}

/// `PaintingFailed` for `graph`'s model, with `detail` cut and redacted.
fn painting_failed(graph: &Graph, detail: &str) -> AutoPaperError {
    AutoPaperError::PaintingFailed { provider: KIND, model: graph.model_name.clone(), detail: short(detail) }
}

/// "message: details" of a ComfyUI error object.
fn describe(error: &Value) -> String {
    let message = error.get("message").and_then(Value::as_str).unwrap_or("error");
    match error.get("details").and_then(Value::as_str).filter(|d| !d.is_empty()) {
        Some(details) => format!("{message}: {details}"),
        None => message.to_string(),
    }
}

/// A history entry with status "error", for `graph`: which node failed and its exception (`PaintingFailed`), or
/// `Stopped` for an interrupt.
fn run_failure(entry: &Value, graph: &Graph) -> AutoPaperError {
    let label = &graph.label;
    let messages = entry.pointer("/status/messages").and_then(Value::as_array).into_iter().flatten();
    for message in messages {
        let event = message.get(0).and_then(Value::as_str);
        let data = message.get(1);
        let field = |name: &str| data.and_then(|d| d.get(name)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        match event {
            Some("execution_error") => {
                let node = match (field("node_type"), field("node_id")) {
                    (Some(class_type), Some(id)) => format!("the {class_type} node ({id})"),
                    (Some(class_type), None) => format!("the {class_type} node"),
                    (None, _) => "a node".to_string(),
                };
                let exception = field("exception_message").unwrap_or("an error");
                return painting_failed(graph, &format!("ComfyUI couldn't paint with {label}: {node} failed: {exception}"));
            }
            Some("execution_interrupted") => {
                return AutoPaperError::unavailable(KIND, ProviderUnavailableReason::Stopped, "the job was stopped in ComfyUI");
            }
            _ => {}
        }
    }
    painting_failed(graph, &format!("ComfyUI couldn't paint with {label}"))
}

/// First `DETAIL_CHARS` characters, redacted.
fn short(text: &str) -> String {
    redact(&text.chars().take(DETAIL_CHARS).collect::<String>())
}

fn invalid(detail: impl Into<String>) -> AutoPaperError {
    AutoPaperError::InvalidResponse { detail: detail.into() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ImageQuality;
    use crate::testing::{SocketStep, StubHttp};

    const PNG: &[u8] = include_bytes!("../../tests/fixtures/openai_compat/tiny.png");

    // Recorded from the user's ComfyUI 0.36.0 with the bundled template (core/tests/fixtures/comfyui/).
    const RECORDED_REQUEST: &str = include_str!("../../tests/fixtures/comfyui/prompt.request.json");
    const QUEUED: &str = include_str!("../../tests/fixtures/comfyui/prompt.response.json");
    const HISTORY: &str = include_str!("../../tests/fixtures/comfyui/history.response.json");
    const REJECTED: &str = include_str!("../../tests/fixtures/comfyui/prompt-invalid.response.json");
    const PROMPT_ID: &str = "0e152c3c-547d-4a80-a2c3-fc70737ed74d";

    /// A history entry for the recorded job with other outputs and status (failure shapes per
    /// ComfyUI's execution.py).
    fn history(outputs: Value, status: Value) -> String {
        json!({ PROMPT_ID: {
            "prompt": [5, PROMPT_ID, { "10": { "class_type": "SaveImage", "inputs": { "images": ["9", 0], "filename_prefix": "autopaper/wallpaper" } } }, {}, ["10"]],
            "outputs": outputs,
            "status": status,
            "meta": {},
        }})
        .to_string()
    }

    fn png_response() -> HttpResponse {
        HttpResponse { status: 200, headers: vec![("Content-Type".into(), "image/png".into())], body: PNG.to_vec() }
    }

    fn comfy(http: &Arc<StubHttp>, workflow: Option<&str>) -> ComfyUi {
        ComfyUi::new(http.clone(), "http://127.0.0.1:8188".into(), workflow.map(String::from))
    }

    /// The recorded run's prompt, size and seed.
    fn request(seed: Option<u64>) -> ImageRequest {
        let recorded: Value = serde_json::from_str(RECORDED_REQUEST).unwrap();
        ImageRequest {
            model: String::new(),
            prompt: recorded["prompt"]["5"]["inputs"]["text"].as_str().unwrap().to_string(),
            width: 1920,
            height: 1088,
            quality: ImageQuality::High,
            seed,
            progress: None,
            expected_secs: None,
        }
    }

    /// The scripted happy path: queued, not done yet, done, image.
    fn script_success(http: &StubHttp) {
        http.once("/prompt", 200, QUEUED);
        http.once("/history/", 200, "{}");
        http.once("/history/", 200, HISTORY);
        http.once_response("/view", png_response());
    }

    /// The default bundled workflow (Z-Image Turbo).
    fn default_workflow() -> &'static Mapped {
        &bundled().unwrap()[0]
    }

    /// The bundled workflow named `name`.
    fn workflow(name: &str) -> &'static Mapped {
        bundled().unwrap().iter().find(|workflow| workflow.map.name == name).unwrap()
    }

    #[test]
    fn every_bundled_workflow_parses_maps_its_placeholders_and_declares_what_it_loads() {
        let names: Vec<(&str, Option<String>)> =
            bundled().unwrap().iter().map(|workflow| (workflow.map.name.as_str(), workflow.model())).collect();
        assert_eq!(
            names,
            [
                ("Z-Image Turbo", Some("z_image_turbo_bf16.safetensors".into())),
                ("Krea 2 Turbo", Some("krea2_turbo_fp8_scaled.safetensors".into())),
                ("Qwen-Image 2.1", Some("qwen_image_2.1_int8_convrot.safetensors".into())),
            ],
            "the default first"
        );
        assert_eq!(bundled_model().as_deref(), Some("z_image_turbo_bf16.safetensors"));
        assert_eq!(bundled_model_name().as_deref(), Some("Z-Image Turbo"));
        for workflow in bundled().unwrap() {
            let name = &workflow.map.name;
            let filled = workflow.fill("a quiet lake", 1920, 1088, 77).unwrap();
            for slot in workflow.slots() {
                let value = &filled[&slot.node]["inputs"][&slot.input];
                assert!(!value.as_str().is_some_and(|text| text.contains("{{")), "{name}: {} still a placeholder", slot.input);
            }
            assert!(!Value::Object(filled.clone()).to_string().contains("{{"), "{name}: a placeholder the map doesn't fill");
            let placeholders = &workflow.map.placeholders;
            let get = |slot: &Slot| filled[&slot.node]["inputs"][&slot.input].clone();
            assert_eq!(get(&placeholders.prompt), json!("a quiet lake"), "{name}");
            assert_eq!((get(&placeholders.width), get(&placeholders.height)), (json!(1920), json!(1088)), "{name}");
            assert_eq!(get(&placeholders.seed), json!(77), "{name}");
            let output = workflow.map.output.as_ref().map(|o| o.node.as_str()).unwrap();
            assert_eq!(filled[output]["class_type"], "SaveImage", "{name}");
            assert_eq!(filled[output]["inputs"]["filename_prefix"], "autopaper/wallpaper", "{name}");
            // `requires` declares exactly the files the graph's loaders load, and no custom node is needed.
            for ((class_type, input, kind), declared) in FILE_LOADERS.iter().zip(workflow.map.requires.files()) {
                let loaded: Vec<String> = filled
                    .values()
                    .filter(|node| node["class_type"] == *class_type)
                    .filter_map(|node| node["inputs"][*input].as_str().map(String::from))
                    .collect();
                assert_eq!(declared, loaded.as_slice(), "{name}: the {kind}");
            }
            assert!(workflow.map.requires.custom_nodes.is_empty(), "{name}: stock nodes only");
            // Its sizes come from MODEL_LIMITS under the same name.
            assert_eq!(model_limits(&workflow.model().unwrap()).name, name.as_str());
        }
    }

    #[test]
    fn filling_each_bundled_template_reproduces_the_graph_verified_on_the_mac() {
        // Z-Image Turbo: recorded through AutoPaper; Qwen-Image 2.1 and Krea 2 Turbo: the graphs that painted the
        // website's examples (rain-ruins, rooftops-spring).
        for (name, recorded, prompt_node) in [
            ("Z-Image Turbo", RECORDED_REQUEST, ("5", "text")),
            ("Qwen-Image 2.1", include_str!("../../tests/fixtures/comfyui/prompt-qwen-image-2.1.request.json"), ("4", "prompt")),
            ("Krea 2 Turbo", include_str!("../../tests/fixtures/comfyui/prompt-krea-2-turbo.request.json"), ("4", "text")),
        ] {
            let recorded: Value = serde_json::from_str(recorded).unwrap();
            let mut expected = recorded["prompt"].as_object().unwrap().clone();
            let prompt = expected[prompt_node.0]["inputs"][prompt_node.1].as_str().unwrap().to_string();
            let seed = expected["8"]["inputs"]["seed"].as_u64().unwrap();
            let size = &expected["7"]["inputs"];
            let (width, height) = (size["width"].as_u64().unwrap() as u32, size["height"].as_u64().unwrap() as u32);
            if name == "Krea 2 Turbo" {
                // Without the third-party ConditioningKrea2Rebalance node: the sampler takes the text encoding
                // itself, as in Comfy-Org's own Krea 2 Turbo template.
                assert_eq!(expected.remove("5").unwrap()["class_type"], "ConditioningKrea2Rebalance");
                expected["8"]["inputs"]["positive"] = json!(["4", 0]);
            }
            let mut filled = workflow(name).fill(&prompt, width, height, seed).unwrap();
            // The recordings named their files after the examples; the templates save as "autopaper/wallpaper".
            for graph in [&mut filled, &mut expected] {
                graph["10"]["inputs"].as_object_mut().unwrap().remove("filename_prefix");
            }
            assert_eq!(filled, expected, "{name}");
        }
    }

    /// GET /object_info from the user's ComfyUI 0.36.0 (2026-10-06), trimmed to the node classes the bundled
    /// workflows use; every bundled workflow can run there.
    fn object_info() -> Map<String, Value> {
        serde_json::from_str(include_str!("../../tests/fixtures/comfyui/object_info.response.json")).unwrap()
    }

    /// Removes `choice` from `class_type`'s combo `input` in an /object_info answer.
    fn without_choice(info: &mut Map<String, Value>, class_type: &str, input: &str, choice: &str) {
        let choices = info[class_type]["input"]["required"][input][0].as_array_mut().unwrap();
        let before = choices.len();
        choices.retain(|offered| offered != choice);
        assert_eq!(choices.len(), before - 1, "{class_type}.{input} offered {choice}");
    }

    #[test]
    fn a_bundled_workflow_runs_only_where_its_files_nodes_and_choices_are() {
        let info = object_info();
        for workflow in bundled().unwrap() {
            assert_eq!(workflow.missing(&info), Vec::<String>::new(), "{}", workflow.map.name);
        }
        let qwen = workflow("Qwen-Image 2.1");
        let mut no_encoder = object_info();
        without_choice(&mut no_encoder, "CLIPLoader", "clip_name", "qwen3vl_8b_int8_convrot.safetensors");
        assert_eq!(qwen.missing(&no_encoder), ["text encoder qwen3vl_8b_int8_convrot.safetensors"]);
        let mut old_server = object_info();
        old_server.remove("TextEncodeQwenImage21");
        assert_eq!(qwen.missing(&old_server), ["node TextEncodeQwenImage21"]);

        let krea = workflow("Krea 2 Turbo");
        let mut no_krea_type = object_info();
        without_choice(&mut no_krea_type, "CLIPLoader", "type", "krea2");
        assert_eq!(krea.missing(&no_krea_type), ["CLIPLoader type krea2"]);
        let mut nothing = object_info();
        for (class_type, input, _) in FILE_LOADERS {
            nothing[class_type]["input"]["required"][input][0] = json!([]);
        }
        assert_eq!(
            krea.missing(&nothing),
            [
                "diffusion model krea2_turbo_fp8_scaled.safetensors",
                "text encoder qwen3vl_4b_fp8_scaled.safetensors",
                "VAE qwen_image_vae.safetensors"
            ]
        );
        assert_eq!(default_workflow().missing(&Map::new()).len(), 3 + 10, "three files and every node class");
    }

    #[test]
    fn a_broken_node_map_is_reported() {
        let workflow = r#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": ""}}}"#;
        let slots = r#""prompt": {"node": "1", "input": "text"}, "width": {"node": "9", "input": "width"},
                       "height": {"node": "1", "input": "h"}, "seed": {"node": "1", "input": "s"}"#;
        let map = format!(r#"{{"placeholders": {{{slots}}}}}"#);
        assert!(Mapped::parse(workflow, &map).unwrap_err().contains("no node 9"));
        let fine = slots.replace(r#""node": "9""#, r#""node": "1""#);
        let bad_output = format!(r#"{{"placeholders": {{{fine}}}, "output": {{"node": "10"}}}}"#);
        assert!(Mapped::parse(workflow, &bad_output).unwrap_err().contains("no output node 10"));
        assert!(Mapped::parse(workflow, &format!(r#"{{"placeholders": {{{fine}}}}}"#)).is_ok());
        assert!(Mapped::parse("[]", &map).is_err());
        assert!(Mapped::parse(workflow, "{}").is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn generates_with_the_bundled_template() {
        let http = Arc::new(StubHttp::new());
        script_success(&http);
        let started = tokio::time::Instant::now();
        let image = comfy(&http, None).generate(request(Some(1001))).await.unwrap();
        assert_eq!(image.bytes, PNG);
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.model, "z_image_turbo_bf16.safetensors");
        assert_eq!(image.reported_cost_microusd, None);
        assert_eq!(started.elapsed(), Duration::from_secs(3), "polled after 1 s, then 2 s");

        let requests = http.requests();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].method, HttpMethod::Post);
        assert_eq!(requests[0].url, "http://127.0.0.1:8188/prompt");
        assert_eq!(requests[0].policy, HostPolicy::UserEndpoint);
        let body = http.json_body(0);
        assert!(uuid::Uuid::parse_str(body["client_id"].as_str().unwrap()).is_ok());
        let placeholders = &default_workflow().map.placeholders;
        let input = |slot: &Slot| body["prompt"][&slot.node]["inputs"][&slot.input].clone();
        assert_eq!(input(&placeholders.prompt), json!(request(None).prompt));
        assert_eq!(input(&placeholders.width), json!(1920));
        assert_eq!(input(&placeholders.height), json!(1088));
        assert_eq!(input(&placeholders.seed), json!(1001));

        assert_eq!(requests[1].url, format!("http://127.0.0.1:8188/history/{PROMPT_ID}"));
        assert_eq!(requests[1].method, HttpMethod::Get);
        assert_eq!(requests[2].url, requests[1].url);
        assert_eq!(
            requests[3].url,
            "http://127.0.0.1:8188/view?filename=desert-glass__zimage__s1001_00001_.png&subfolder=autopaper&type=output"
        );
        assert_eq!(requests[3].max_response_bytes, MAX_IMAGE_BYTES);
    }

    #[tokio::test(start_paused = true)]
    async fn a_random_seed_when_none_is_given() {
        let http = Arc::new(StubHttp::new());
        script_success(&http);
        script_success(&http);
        let provider = comfy(&http, None);
        provider.generate(request(None)).await.unwrap();
        provider.generate(request(None)).await.unwrap();
        let seed_slot = &default_workflow().map.placeholders.seed;
        let seed = |n: usize| http.json_body(n)["prompt"][&seed_slot.node]["inputs"][&seed_slot.input].as_u64().unwrap();
        let (first, second) = (seed(0), seed(4));
        assert_ne!(first, second);
        assert!(first < 1 << SEED_BITS && second < 1 << SEED_BITS);
    }

    #[tokio::test(start_paused = true)]
    async fn a_requested_seed_stays_below_2_to_the_53() {
        let http = Arc::new(StubHttp::new());
        script_success(&http);
        comfy(&http, None).generate(request(Some(u64::MAX))).await.unwrap();
        let seed_slot = &default_workflow().map.placeholders.seed;
        let seed = http.json_body(0)["prompt"][&seed_slot.node]["inputs"][&seed_slot.input].as_u64().unwrap();
        assert_eq!(seed, SEED_MASK, "the engine's 64-bit seeds keep their low 53 bits");
    }

    #[tokio::test(start_paused = true)]
    async fn the_chosen_model_paints_with_its_own_bundled_workflow() {
        // The user's bug (2026-10-06): Qwen-Image 2.1 chosen, run through Z-Image Turbo's text encoder and VAE.
        for (model, encoder, vae, text_node) in [
            ("", "qwen_3_4b.safetensors", "ae.safetensors", "CLIPTextEncode"),
            ("qwen_image_2.1_int8_convrot.safetensors", "qwen3vl_8b_int8_convrot.safetensors", "qwen_image_2.1_vae_bf16.safetensors", "TextEncodeQwenImage21"),
            ("krea2_turbo_fp8_scaled.safetensors", "qwen3vl_4b_fp8_scaled.safetensors", "qwen_image_vae.safetensors", "CLIPTextEncode"),
        ] {
            let http = Arc::new(StubHttp::new());
            script_success(&http);
            let mut req = request(Some(1));
            req.model = format!(" {model} ");
            let provider = comfy(&http, None);
            assert!(provider.check_model(model).is_ok(), "{model}");
            let image = provider.generate(req).await.unwrap();
            let expected = if model.is_empty() { "z_image_turbo_bf16.safetensors" } else { model };
            assert_eq!(image.model, expected);
            let graph = http.json_body(0)["prompt"].as_object().unwrap().clone();
            let input = |class_type: &str, input: &str| {
                graph.values().find(|node| node["class_type"] == class_type).map(|node| node["inputs"][input].clone())
            };
            assert_eq!(input("UNETLoader", "unet_name"), Some(json!(expected)));
            assert_eq!(input("CLIPLoader", "clip_name"), Some(json!(encoder)), "{model}");
            assert_eq!(input("VAELoader", "vae_name"), Some(json!(vae)), "{model}");
            assert!(input(text_node, "clip").is_some(), "{model}: the prompt goes through {text_node}");
            assert!(!graph.values().any(|node| node["class_type"] == "ConditioningKrea2Rebalance"), "stock nodes only");
        }
    }

    #[tokio::test]
    async fn a_model_without_a_bundled_workflow_is_refused_before_anything_is_sent() {
        let http = Arc::new(StubHttp::new());
        let provider = comfy(&http, None);
        // Settings listed every diffusion model before 2026-10-06; this one has no workflow.
        let model = "ltx-2.5-22b-distilled-transformer-comfy-int8-convrot.safetensors";
        for result in [provider.check_model(model), provider.generate(ImageRequest { model: model.into(), ..request(Some(1)) }).await.map(|_| ())] {
            match result {
                Err(AutoPaperError::PaintingFailed { provider: KIND, model: named, detail }) => {
                    assert_eq!(named, "ltx-2.5-22b-distilled-transformer-comfy-int8-convrot", "the model in words");
                    assert!(detail.starts_with(&format!("AutoPaper has no ComfyUI workflow for {model}")), "{detail}");
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(http.requests().is_empty());
        // A person's own workflow loads its own model, whatever is chosen.
        assert!(comfy(&http, Some(CUSTOM)).check_model(model).is_ok());
    }

    const CUSTOM: &str = r#"{
        "3": {"class_type": "KSampler", "inputs": {"seed": {{seed}}, "steps": 20, "model": ["4", 0], "latent_image": ["5", 0]}},
        "4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "sd_xl_base_1.0.safetensors"}},
        "5": {"class_type": "EmptyLatentImage", "inputs": {"width": "{{width}}", "height": {{height}}, "batch_size": 1}},
        "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}", "clip": ["4", 1]}},
        "7": {"class_type": "CLIPTextEncode", "inputs": {"text": "masterpiece, {{prompt}}, 8k", "clip": ["4", 1]}},
        "9": {"class_type": "SaveImage", "inputs": {"filename_prefix": "mine", "images": ["8", 0]}}
    }"#;

    #[tokio::test(start_paused = true)]
    async fn fills_a_persons_template_literally() {
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 200, QUEUED);
        http.once(
            "/history/",
            200,
            history(
                json!({ "9": { "images": [{ "filename": "mine_00001_.png", "subfolder": "", "type": "output" }] } }),
                json!({ "status_str": "success", "completed": true, "messages": [] }),
            ),
        );
        http.once_response("/view", png_response());
        let provider = comfy(&http, Some(CUSTOM));
        assert_eq!(provider.default_model(), "sd_xl_base_1.0.safetensors");

        let tricky = "A \"quoted\" sky\nwith a back\\slash, {{width}} and émoji ✨";
        let mut req = request(Some(5));
        req.prompt = tricky.into();
        req.model = "ignored-for-custom.safetensors".into();
        let image = provider.generate(req).await.unwrap();
        assert_eq!(image.model, "sd_xl_base_1.0.safetensors", "a person's template keeps its model");

        let graph = &http.json_body(0)["prompt"];
        assert_eq!(graph["6"]["inputs"]["text"], tricky);
        assert_eq!(graph["7"]["inputs"]["text"], format!("masterpiece, {tricky}, 8k"));
        assert_eq!(graph["5"]["inputs"]["width"], 1920);
        assert_eq!(graph["5"]["inputs"]["height"], 1088);
        assert_eq!(graph["3"]["inputs"]["seed"], 5);
        assert_eq!(graph["4"]["inputs"]["ckpt_name"], "sd_xl_base_1.0.safetensors");
    }

    #[test]
    fn rejects_unusable_templates() {
        let no_prompt = r#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "fixed"}}}"#;
        let ui_format =
            r#"{"last_node_id": 9, "nodes": [{"id": 1, "type": "CLIPTextEncode", "widgets_values": ["{{prompt}}"]}], "links": []}"#;
        let broken = r#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": {{prompt}}}}}"#;
        let not_nodes = r#"{"text": "{{prompt}}"}"#;
        for (template, why, says) in [
            (no_prompt, InvalidInputReason::WorkflowNeedsPrompt, "{{prompt}}"),
            (ui_format, InvalidInputReason::WorkflowNotApiFormat, "UI format"),
            (broken, InvalidInputReason::WorkflowInvalid, "valid JSON"),
            (not_nodes, InvalidInputReason::WorkflowNotApiFormat, "API format"),
        ] {
            match fill_placeholders(template, "x", 1, 1, 1) {
                Err(AutoPaperError::InvalidInput { reason, detail }) => {
                    assert_eq!(reason, why, "{template}");
                    assert!(detail.contains(says), "{detail}");
                }
                other => panic!("{template}: {other:?}"),
            }
        }
        let wrapped = r#"{"prompt": {"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}, "client_id": "x"}"#;
        assert_eq!(fill_placeholders(wrapped, "hi", 1, 1, 1).unwrap()["1"]["inputs"]["text"], "hi");
    }

    #[tokio::test]
    async fn a_blank_template_means_the_bundled_one() {
        let http = Arc::new(StubHttp::new());
        let provider = ComfyUi::new(http, "http://127.0.0.1:8188".into(), Some("  \n".into()));
        assert!(provider.workflow.is_none());
        assert_eq!(provider.default_model(), default_workflow().model().unwrap());
        let unparseable = ComfyUi::new(Arc::new(StubHttp::new()), "http://127.0.0.1:8188".into(), Some("{{prompt}} nope".into()));
        assert_eq!(unparseable.default_model(), CUSTOM_MODEL);
    }

    #[tokio::test]
    async fn node_errors_are_painting_failures_with_the_nodes_message() {
        // Recorded: the bundled template with a model file that isn't installed.
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 400, REJECTED);
        match comfy(&http, None).generate(request(Some(1))).await {
            Err(error @ AutoPaperError::PaintingFailed { provider: KIND, .. }) => {
                assert!(!error.is_transient());
                let AutoPaperError::PaintingFailed { model, detail, .. } = error else { unreachable!() };
                assert_eq!(model, "Z-Image Turbo");
                assert!(
                    detail.starts_with(
                        "ComfyUI rejected the workflow for Z-Image Turbo: UNETLoader (node 1): Value not in list: unet_name: 'not-installed.safetensors' not in ["
                    ),
                    "{detail}"
                );
                assert!(detail.chars().count() <= DETAIL_CHARS, "{detail}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(http.requests().len(), 1, "nothing is polled after a rejection");

        let mut rejected: Value = serde_json::from_str(REJECTED).unwrap();
        rejected["node_errors"]["1"]["errors"][0]["details"] = json!("unet_name: token sk-abcdefghijklmnopqrstuvwxyz0123 not allowed");
        http.once("/prompt", 400, rejected.to_string());
        match comfy(&http, None).generate(request(Some(1))).await {
            Err(AutoPaperError::PaintingFailed { detail, .. }) => {
                assert!(detail.ends_with("unet_name: token [redacted] not allowed"), "redacted: {detail}");
            }
            other => panic!("{other:?}"),
        }

        let no_nodes = json!({ "error": { "type": "invalid_prompt", "message": "Cannot execute because a node is missing the class_type property.", "details": "Node ID '#3'", "extra_info": {} }, "node_errors": [] });
        http.once("/prompt", 400, no_nodes.to_string());
        match comfy(&http, None).generate(request(Some(1))).await {
            Err(AutoPaperError::PaintingFailed { detail, .. }) => {
                assert!(detail.contains("missing the class_type property.: Node ID '#3'"), "{detail}")
            }
            other => panic!("{other:?}"),
        }

        http.once("/prompt", 400, "<html>");
        let html = comfy(&http, None).generate(request(Some(1))).await;
        assert!(
            matches!(&html, Err(AutoPaperError::PaintingFailed { detail, .. }) if detail.ends_with("(HTTP 400)")),
            "{html:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_run_reports_the_nodes_exception() {
        let failed = history(
            json!({}),
            json!({ "status_str": "error", "completed": false, "messages": [
                ["execution_start", { "prompt_id": PROMPT_ID }],
                ["execution_error", { "prompt_id": PROMPT_ID, "node_id": "8", "node_type": "KSampler",
                    "exception_message": "MPS backend out of memory\n", "exception_type": "torch.OutOfMemoryError", "traceback": ["…"] }],
            ]}),
        );
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 200, QUEUED);
        http.once("/history/", 200, failed);
        match comfy(&http, None).generate(request(Some(1))).await {
            Err(AutoPaperError::PaintingFailed { provider: KIND, model, detail }) => {
                assert_eq!(model, "Z-Image Turbo");
                assert_eq!(detail, "ComfyUI couldn't paint with Z-Image Turbo: the KSampler node (8) failed: MPS backend out of memory");
            }
            other => panic!("{other:?}"),
        }

        // The user's run (2026-10-06, before each model had its own workflow), as ComfyUI reported it; a secret in an
        // exception is redacted, and a person's workflow is named by the model it loads.
        let mismatch = "Given normalized_shape=[4096], expected input with shape [*4096], but got input of size[1, 95, 2560]";
        let failure = |exception: &str| {
            history(
                json!({}),
                json!({ "status_str": "error", "completed": false, "messages": [
                    ["execution_error", { "prompt_id": PROMPT_ID, "node_id": "8", "node_type": "KSampler", "exception_message": exception }],
                ]}),
            )
        };
        http.once("/prompt", 200, QUEUED);
        http.once("/history/", 200, failure(mismatch));
        let qwen = ImageRequest { model: "qwen_image_2.1_int8_convrot.safetensors".into(), ..request(Some(1)) };
        match comfy(&http, None).generate(qwen).await {
            Err(AutoPaperError::PaintingFailed { model, detail, .. }) => {
                assert_eq!(model, "Qwen-Image 2.1");
                assert_eq!(detail, format!("ComfyUI couldn't paint with Qwen-Image 2.1: the KSampler node (8) failed: {mismatch}"));
            }
            other => panic!("{other:?}"),
        }
        http.once("/prompt", 200, QUEUED);
        http.once("/history/", 200, failure("bad header Authorization: Bearer sk-abcdefghijklmnopqrstuvwxyz0123"));
        match comfy(&http, Some(CUSTOM)).generate(request(Some(1))).await {
            Err(AutoPaperError::PaintingFailed { model, detail, .. }) => {
                assert_eq!(model, "sd_xl_base_1.0", "a person's workflow: its model file");
                assert!(detail.starts_with("ComfyUI couldn't paint with your workflow (sd_xl_base_1.0): the KSampler node (8) failed: "), "{detail}");
                assert!(!detail.contains("sk-abcdefghij"), "{detail}");
            }
            other => panic!("{other:?}"),
        }

        let stopped = history(
            json!({}),
            json!({ "status_str": "error", "completed": false, "messages": [
                ["execution_interrupted", { "prompt_id": PROMPT_ID, "node_id": "8", "node_type": "KSampler", "executed": [] }],
            ]}),
        );
        http.once("/prompt", 200, QUEUED);
        http.once("/history/", 200, stopped);
        let result = comfy(&http, None).generate(request(Some(1))).await;
        assert!(
            matches!(result, Err(AutoPaperError::ProviderUnavailable { provider: KIND, reason: ProviderUnavailableReason::Stopped, .. })),
            "{result:?}"
        );
        assert!(http.requests().iter().all(|r| !r.url.contains("/api/jobs/")), "an ended job isn't stopped again");

        let no_image = history(json!({ "10": { "images": [] } }), json!({ "status_str": "success", "completed": true, "messages": [] }));
        http.once("/prompt", 200, QUEUED);
        http.once("/history/", 200, no_image);
        match comfy(&http, None).generate(request(Some(1))).await {
            Err(AutoPaperError::PaintingFailed { detail, .. }) => assert!(detail.contains("without saving an image"), "{detail}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_past_the_ceiling_and_cancels_the_job() {
        // Without progress events nothing can tell a stall: only the ceiling, 30 minutes without an estimate.
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 200, QUEUED);
        http.always("/history/", 200, "{}");
        http.once("/api/jobs/", 200, r#"{"cancelled": true}"#);
        let started = tokio::time::Instant::now();
        let result = comfy(&http, None).generate(request(Some(1))).await;
        assert!(
            matches!(&result, Err(AutoPaperError::ProviderUnavailable { provider: KIND, reason: ProviderUnavailableReason::TimedOut, detail }) if detail == "ComfyUI didn't finish within 30 minutes"),
            "{result:?}"
        );
        assert_eq!(started.elapsed(), MIN_CEILING);
        let requests = http.requests();
        let last = requests.last().unwrap();
        assert_eq!((last.method, last.url.as_str()), (HttpMethod::Post, format!("http://127.0.0.1:8188/api/jobs/{PROMPT_ID}/cancel").as_str()));
        assert!(last.body.is_none());
        assert!(requests.iter().all(|r| !r.url.ends_with("/interrupt") && !r.url.ends_with("/queue")), "only this job is cancelled");
        let polls = requests.iter().filter(|r| r.url.contains("/history/")).count();
        assert_eq!(polls, 900, "at 1 s and every 2 s to 1799 s");

        // A painting that usually takes 15 minutes here gets three times that.
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 200, QUEUED);
        http.always("/history/", 200, "{}");
        http.once("/api/jobs/", 200, r#"{"cancelled": true}"#);
        let started = tokio::time::Instant::now();
        let slow = ImageRequest { expected_secs: Some(900.0), ..request(Some(1)) };
        let result = comfy(&http, None).generate(slow).await;
        assert!(
            matches!(&result, Err(AutoPaperError::ProviderUnavailable { reason: ProviderUnavailableReason::TimedOut, detail, .. }) if detail == "ComfyUI didn't finish within 45 minutes"),
            "{result:?}"
        );
        assert_eq!(started.elapsed(), Duration::from_secs(45 * 60));
        assert_eq!(ceiling(Some(60.0)), MIN_CEILING, "a fast painter still gets 30 minutes");
    }

    /// A /ws event about the recorded job.
    fn event(kind: &str, data: Value) -> SocketStep {
        let mut data = data;
        data["prompt_id"] = json!(PROMPT_ID);
        SocketStep::Text(json!({ "type": kind, "data": data }).to_string())
    }

    fn wait(seconds: u64) -> SocketStep {
        SocketStep::Wait(Duration::from_secs(seconds))
    }

    /// The steps a painting reported, with when they came.
    type Reported = Arc<std::sync::Mutex<Vec<(Duration, PaintStep)>>>;

    /// Records the steps a painting reports.
    fn recorder() -> (PaintProgress, Reported) {
        let started = tokio::time::Instant::now();
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        (PaintProgress(Arc::new(move |step| sink.lock().unwrap().push((started.elapsed(), step)))), seen)
    }

    #[tokio::test(start_paused = true)]
    async fn sampler_steps_from_the_socket_reach_the_engine() {
        let http = Arc::new(StubHttp::new());
        http.socket(
            "/ws",
            vec![
                // Other clients' news and another job's events are ignored.
                SocketStep::Text(json!({ "type": "status", "data": { "status": { "exec_info": { "queue_remaining": 1 } } } }).to_string()),
                SocketStep::Text(json!({ "type": "progress", "data": { "value": 3, "max": 9, "prompt_id": "someone-else", "node": "3" } }).to_string()),
                wait(1),
                event("execution_start", json!({ "timestamp": 1 })),
                event("executing", json!({ "node": "1" })),
                wait(1),
                event("executing", json!({ "node": "8" })),
                event("progress_state", json!({ "nodes": {} })),
                event("progress", json!({ "value": 1, "max": 8, "node": "8" })),
                wait(1),
                event("progress", json!({ "value": 2, "max": 8, "node": "8" })),
                event("progress", json!({ "value": 2, "max": 8, "node": "8" })),
                wait(1),
                event("progress", json!({ "value": 8, "max": 8, "node": "8" })),
                event("executing", json!({ "node": "9" })),
                event("executing", json!({ "node": null })),
            ],
        );
        http.once("/prompt", 200, QUEUED);
        http.once("/history/", 200, "{}");
        http.once("/history/", 200, "{}");
        http.once("/history/", 200, HISTORY);
        http.once_response("/view", png_response());
        let (progress, seen) = recorder();
        let started = tokio::time::Instant::now();
        let request = ImageRequest { progress: Some(progress), ..request(Some(1)) };
        comfy(&http, None).generate(request).await.unwrap();
        // The socket said it was done at 4 s: polled then, not at the next 2 s mark (5 s).
        assert_eq!(started.elapsed(), Duration::from_secs(4));
        let steps: Vec<(u64, u32, u32)> =
            seen.lock().unwrap().iter().map(|(at, step)| (at.as_secs(), step.done, step.total)).collect();
        assert_eq!(steps, [(2, 1, 8), (3, 2, 8), (4, 8, 8)], "once per change, of the graph's 8 steps");
        // The socket was opened as the client the job was queued for.
        let opened = http.sockets_opened();
        assert_eq!(opened.len(), 1);
        let client_id = http.json_body(0)["client_id"].as_str().unwrap().to_string();
        assert_eq!(opened[0].url, format!("http://127.0.0.1:8188/ws?clientId={client_id}"));
        assert_eq!(opened[0].policy, HostPolicy::UserEndpoint);
        assert_eq!(opened[0].max_response_bytes, MAX_SOCKET_MESSAGE_BYTES);
    }

    #[tokio::test(start_paused = true)]
    async fn a_job_that_stops_stepping_is_stopped_after_three_minutes() {
        let http = Arc::new(StubHttp::new());
        http.socket(
            "/ws",
            vec![
                event("execution_start", json!({})),
                event("executing", json!({ "node": "8" })),
                wait(10),
                event("progress", json!({ "value": 1, "max": 8, "node": "8" })),
                wait(10),
                event("progress", json!({ "value": 2, "max": 8, "node": "8" })),
                // Then nothing: ComfyUI hangs (a stuck GPU, a deadlocked node).
            ],
        );
        http.once("/prompt", 200, QUEUED);
        http.always("/history/", 200, "{}");
        http.once("/api/jobs/", 200, r#"{"cancelled": true}"#);
        let started = tokio::time::Instant::now();
        let result = comfy(&http, None).generate(request(Some(1))).await;
        assert!(
            matches!(&result, Err(AutoPaperError::ProviderUnavailable { reason: ProviderUnavailableReason::TimedOut, detail, .. }) if detail == "ComfyUI made no progress for 180 s"),
            "{result:?}"
        );
        assert_eq!(started.elapsed(), Duration::from_secs(20 + 180), "3 minutes after the last step");
        let last = http.requests().last().unwrap().clone();
        assert_eq!(last.url, format!("http://127.0.0.1:8188/api/jobs/{PROMPT_ID}/cancel"), "and the job is stopped");
    }

    #[tokio::test(start_paused = true)]
    async fn slow_steps_and_steps_less_nodes_get_longer_windows_and_the_queue_none() {
        // 100 s a step (a slow computer): 4 × 100 s may pass without a step.
        let http = Arc::new(StubHttp::new());
        http.socket(
            "/ws",
            vec![
                event("executing", json!({ "node": "8" })),
                event("progress", json!({ "value": 1, "max": 8, "node": "8" })),
                wait(100),
                event("progress", json!({ "value": 2, "max": 8, "node": "8" })),
            ],
        );
        http.once("/prompt", 200, QUEUED);
        http.always("/history/", 200, "{}");
        http.once("/api/jobs/", 200, r#"{"cancelled": true}"#);
        let started = tokio::time::Instant::now();
        let result = comfy(&http, None).generate(request(Some(1))).await;
        assert!(matches!(&result, Err(AutoPaperError::ProviderUnavailable { detail, .. }) if detail == "ComfyUI made no progress for 400 s"), "{result:?}");
        assert_eq!(started.elapsed(), Duration::from_secs(100 + 400));

        // Loading a model (a node without steps) may take 10 minutes.
        let http = Arc::new(StubHttp::new());
        http.socket("/ws", vec![event("execution_start", json!({})), event("executing", json!({ "node": "1" }))]);
        http.once("/prompt", 200, QUEUED);
        http.always("/history/", 200, "{}");
        http.once("/api/jobs/", 200, r#"{"cancelled": true}"#);
        let started = tokio::time::Instant::now();
        let result = comfy(&http, None).generate(request(Some(1))).await;
        assert!(matches!(&result, Err(AutoPaperError::ProviderUnavailable { detail, .. }) if detail == "ComfyUI made no progress for 600 s"), "{result:?}");
        assert_eq!(started.elapsed(), NODE_STALL);

        // Waiting in the queue behind someone else's 25-minute job is never a stall; it finishes by polling.
        let http = Arc::new(StubHttp::new());
        http.socket("/ws", vec![SocketStep::Text(json!({ "type": "status", "data": {} }).to_string())]);
        http.once("/prompt", 200, QUEUED);
        for _ in 0..750 {
            http.once("/history/", 200, "{}");
        }
        http.once("/history/", 200, HISTORY);
        http.once_response("/view", png_response());
        let started = tokio::time::Instant::now();
        comfy(&http, None).generate(request(Some(1))).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(1501));
    }

    #[tokio::test(start_paused = true)]
    async fn without_the_socket_the_job_is_still_followed_by_polling() {
        // The socket breaks mid-run: no stall detection after that, and the painting still arrives.
        let http = Arc::new(StubHttp::new());
        http.socket("/ws", vec![event("executing", json!({ "node": "8" })), wait(5), SocketStep::Break]);
        http.once("/prompt", 200, QUEUED);
        for _ in 0..200 {
            http.once("/history/", 200, "{}");
        }
        http.once("/history/", 200, HISTORY);
        http.once_response("/view", png_response());
        let started = tokio::time::Instant::now();
        comfy(&http, None).generate(request(Some(1))).await.unwrap();
        assert_eq!(started.elapsed(), Duration::from_secs(401), "past the 10-minute window would have been fine too");

        // No socket at all (none scripted: an https server, an old ComfyUI): polling only, as before.
        let http = Arc::new(StubHttp::new());
        script_success(&http);
        let (progress, seen) = recorder();
        comfy(&http, None).generate(ImageRequest { progress: Some(progress), ..request(Some(1)) }).await.unwrap();
        assert!(seen.lock().unwrap().is_empty(), "no steps to report");
        assert_eq!(http.sockets_opened().len(), 1, "it tried");
    }

    #[test]
    fn steps_come_from_the_samplers_of_the_graph_that_would_run() {
        let http = Arc::new(StubHttp::new());
        let bundled = comfy(&http, None);
        assert_eq!(bundled.steps(""), Some(8), "Z-Image Turbo");
        assert_eq!(bundled.steps("krea2_turbo_fp8_scaled.safetensors"), Some(8));
        assert_eq!(bundled.steps("qwen_image_2.1_int8_convrot.safetensors"), Some(25));
        assert_eq!(bundled.steps("no-workflow.safetensors"), None);
        assert_eq!(comfy(&http, Some(CUSTOM)).steps("anything"), Some(20), "a person's workflow: its own sampler");
        let refined = CUSTOM.replace(r#""9": {"class_type": "SaveImage""#, r#""10": {"class_type": "KSamplerAdvanced", "inputs": {"steps": 10}}, "9": {"class_type": "SaveImage""#);
        assert_eq!(comfy(&http, Some(&refined)).steps(""), Some(30), "every sampler counts");
    }

    #[test]
    fn a_job_watch_counts_every_sampler_and_never_more_than_the_total() {
        let start = tokio::time::Instant::now();
        let mut watch = JobWatch::new(PROMPT_ID, start, 30, None);
        let progress = |node: &str, value: u32, max: u32| {
            json!({ "type": "progress", "data": { "value": value, "max": max, "node": node, "prompt_id": PROMPT_ID } }).to_string()
        };
        assert_eq!(watch.steps(), None);
        assert!(watch.stall_at().is_none(), "queued: no stall window");
        watch.on_message(&progress("3", 20, 20), start);
        watch.on_message(&progress("10", 4, 10), start);
        assert_eq!(watch.steps(), Some(PaintStep { done: 24, total: 30 }));
        // A node reporting more than the graph said (a tiled decoder's tiles) raises the total.
        watch.on_message(&progress("12", 2, 16), start);
        assert_eq!(watch.steps(), Some(PaintStep { done: 26, total: 46 }));
        assert!(!watch.on_message("not json", start));
        assert!(!watch.on_message(&json!({ "type": "executing", "data": { "node": null, "prompt_id": "other" } }).to_string(), start));
        assert!(watch.on_message(&json!({ "type": "execution_success", "data": { "prompt_id": PROMPT_ID } }).to_string(), start));
    }

    #[tokio::test(start_paused = true)]
    async fn an_older_server_only_has_the_job_unqueued() {
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 200, QUEUED);
        http.always("/history/", 200, "{}");
        http.once("/api/jobs/", 404, "404: Not Found");
        http.once("/queue", 200, "");
        let result = comfy(&http, None).generate(request(Some(1))).await;
        assert!(matches!(result, Err(AutoPaperError::ProviderUnavailable { reason: ProviderUnavailableReason::TimedOut, .. })), "{result:?}");
        let requests = http.requests();
        let last = requests.last().unwrap();
        assert_eq!((last.method, last.url.as_str()), (HttpMethod::Post, "http://127.0.0.1:8188/queue"));
        assert_eq!(http.json_body(requests.len() - 1), json!({ "delete": [PROMPT_ID] }));
        assert!(requests.iter().all(|r| !r.url.ends_with("/interrupt")), "never a global interrupt");
    }

    /// Waits until the stub has seen a request whose URL contains `part` (a spawned task sends it).
    async fn until_requested(http: &StubHttp, part: &str) -> bool {
        for _ in 0..100 {
            if http.requests().iter().any(|r| r.url.contains(part)) {
                return true;
            }
            tokio::task::yield_now().await;
        }
        false
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_call_stops_its_job() {
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 200, QUEUED);
        http.always("/history/", 200, "{}");
        http.once("/api/jobs/", 200, r#"{"cancelled": true}"#);
        let provider = comfy(&http, None);
        // The engine drops the call when the person cancels; here, after 5 s of painting.
        let dropped = tokio::time::timeout(Duration::from_secs(5), provider.generate(request(Some(1)))).await;
        assert!(dropped.is_err(), "still painting");
        assert!(until_requested(&http, "/api/jobs/").await, "the job is cancelled in the background");
        let last = http.requests().last().unwrap().clone();
        assert_eq!(last.url, format!("http://127.0.0.1:8188/api/jobs/{PROMPT_ID}/cancel"));
        assert_eq!(last.method, HttpMethod::Post);

        // A call dropped after its job ended (downloading) leaves ComfyUI alone.
        let http = Arc::new(StubHttp::new());
        script_success(&http);
        comfy(&http, None).generate(request(Some(1))).await.unwrap();
        assert!(!until_requested(&http, "/api/jobs/").await);
    }

    #[tokio::test]
    async fn not_running_is_unavailable() {
        let http = Arc::new(StubHttp::new());
        match comfy(&http, None).generate(request(Some(1))).await {
            Err(error @ AutoPaperError::ProviderUnavailable { provider: KIND, reason: ProviderUnavailableReason::NotRunning, .. }) => {
                assert!(error.to_string().contains("ComfyUI isn't running at http://127.0.0.1:8188"), "{error}");
            }
            other => panic!("{other:?}"),
        }
        let listed = comfy(&http, None).list_models().await;
        assert!(matches!(listed, Err(AutoPaperError::ProviderUnavailable { provider: KIND, .. })), "{listed:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn the_view_query_is_url_encoded() {
        let outputs = json!({ "9": { "images": [{ "filename": "a b&c=d#e?ü.png", "subfolder": "my dir/x+y", "type": "output" }] } });
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 200, QUEUED);
        http.once("/history/", 200, history(outputs, json!({ "status_str": "success", "completed": true, "messages": [] })));
        http.once_response("/view", png_response());
        ComfyUi::new(http.clone(), "http://127.0.0.1:8188/".into(), None).generate(request(Some(1))).await.unwrap();
        let view = &http.requests()[2].url;
        assert_eq!(view, "http://127.0.0.1:8188/view?filename=a+b%26c%3Dd%23e%3F%C3%BC.png&subfolder=my+dir%2Fx%2By&type=output");
        let parsed = url::Url::parse(view).unwrap();
        let pairs: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
        assert_eq!(pairs[0], ("filename".into(), "a b&c=d#e?ü.png".into()));
        assert_eq!(pairs[1], ("subfolder".into(), "my dir/x+y".into()));
    }

    #[test]
    fn picks_the_saved_image() {
        let graph: Map<String, Value> = serde_json::from_str(
            r#"{"10": {"class_type": "SaveImage", "inputs": {}}, "11": {"class_type": "PreviewImage", "inputs": {}},
                "12": {"class_type": "SaveImage", "inputs": {}}}"#,
        )
        .unwrap();
        let entry = json!({ "outputs": {
            "11": { "images": [{ "filename": "preview.png", "subfolder": "", "type": "temp" }] },
            "12": { "images": [{ "filename": "second.png", "subfolder": "s", "type": "output" }] },
            "10": { "images": [{ "filename": "first.png", "subfolder": "", "type": "output" }] },
        }});
        assert_eq!(find_image(&entry, &graph, None).unwrap().filename, "first.png");
        assert_eq!(
            find_image(&entry, &graph, Some("12")).unwrap(),
            ImageRef { filename: "second.png".into(), subfolder: "s".into(), kind: "output".into() }
        );
        let previews_only = json!({ "outputs": { "11": { "images": [{ "filename": "preview.png", "subfolder": "", "type": "temp" }] } } });
        assert_eq!(find_image(&previews_only, &graph, None).unwrap().kind, "temp");
        assert_eq!(find_image(&json!({ "outputs": {} }), &graph, None), None);
        assert_eq!(find_image(&json!({}), &graph, None), None);
    }

    #[tokio::test]
    async fn refuses_an_odd_job_id() {
        let http = Arc::new(StubHttp::new());
        http.once("/prompt", 200, r#"{"prompt_id": "../../etc", "number": 1, "node_errors": {}}"#);
        let result = comfy(&http, None).generate(request(Some(1))).await;
        assert!(matches!(result, Err(AutoPaperError::InvalidResponse { .. })), "{result:?}");
        assert_eq!(http.requests().len(), 1);
    }

    #[tokio::test]
    async fn lists_only_the_models_whose_bundled_workflow_the_server_can_run() {
        let listed = |info: &Map<String, Value>| {
            let http = Arc::new(StubHttp::new());
            http.once("/object_info", 200, Value::Object(info.clone()).to_string());
            async move {
                let models = comfy(&http, None).list_models().await.unwrap();
                let requests = http.requests();
                assert_eq!(requests.len(), 1, "one GET /object_info");
                assert_eq!((requests[0].method, requests[0].url.as_str()), (HttpMethod::Get, "http://127.0.0.1:8188/object_info"));
                models.into_iter().map(|model| (model.display_name, model.id)).collect::<Vec<_>>()
            }
        };
        // The user's ComfyUI has every file: the default first, then by name, each named in words. Its other
        // diffusion models (LTX, MiniMax video models) have no workflow and aren't listed.
        let everything = object_info();
        assert!(combo_choices(input_spec(&everything, "UNETLoader", "unet_name")).len() > 3);
        let pair = |name: &str, file: &str| (name.to_string(), file.to_string());
        assert_eq!(
            listed(&everything).await,
            [
                pair("Z-Image Turbo", "z_image_turbo_bf16.safetensors"),
                pair("Krea 2 Turbo", "krea2_turbo_fp8_scaled.safetensors"),
                pair("Qwen-Image 2.1", "qwen_image_2.1_int8_convrot.safetensors"),
            ]
        );
        // Without Qwen-Image 2.1's text encoder, or on a ComfyUI too old for Krea 2's CLIP type: not offered.
        let mut fewer = object_info();
        without_choice(&mut fewer, "CLIPLoader", "clip_name", "qwen3vl_8b_int8_convrot.safetensors");
        without_choice(&mut fewer, "CLIPLoader", "type", "krea2");
        assert_eq!(listed(&fewer).await, [pair("Z-Image Turbo", "z_image_turbo_bf16.safetensors")]);
        // Without the default's VAE, the others still come by name.
        let mut no_default = object_info();
        without_choice(&mut no_default, "VAELoader", "vae_name", "ae.safetensors");
        let names: Vec<String> = listed(&no_default).await.into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["Krea 2 Turbo", "Qwen-Image 2.1"]);

        let http = Arc::new(StubHttp::new());
        http.once("/object_info", 200, "[]");
        assert!(matches!(comfy(&http, None).list_models().await, Err(AutoPaperError::InvalidResponse { .. })));
    }

    #[tokio::test]
    async fn lists_the_loaders_choices_of_a_persons_workflow() {
        let http = Arc::new(StubHttp::new());
        // A person's template: its checkpoint loader, in the newer COMBO shape.
        let combo = json!({ "CheckpointLoaderSimple": { "input": { "required": { "ckpt_name": ["COMBO", { "options": ["sd_xl_base_1.0.safetensors", "flux/dev.gguf"] }] } } } });
        http.once("/object_info/CheckpointLoaderSimple", 200, combo.to_string());
        let models = comfy(&http, Some(CUSTOM)).list_models().await.unwrap();
        let names: Vec<(&str, &str)> = models.iter().map(|m| (m.id.as_str(), m.display_name.as_str())).collect();
        assert_eq!(names, [("sd_xl_base_1.0.safetensors", "sd_xl_base_1.0"), ("flux/dev.gguf", "flux/dev")]);

        // No recognisable loader: nothing to list.
        let no_loader = r#"{"1": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}"#;
        assert!(comfy(&http, Some(no_loader)).list_models().await.unwrap().is_empty());
    }

    #[test]
    fn reads_combo_choices_in_both_shapes() {
        assert_eq!(combo_choices(Some(&json!([["a", "b"], {}]))), ["a", "b"]);
        assert_eq!(combo_choices(Some(&json!(["COMBO", { "options": ["c"] }]))), ["c"]);
        assert!(combo_choices(Some(&json!(["INT", { "default": 1 }]))).is_empty());
        assert!(combo_choices(None).is_empty());
    }

    #[test]
    fn knows_each_models_limits_by_its_file_name() {
        for (file, name) in [
            ("z_image_turbo_bf16.safetensors", "Z-Image Turbo"),
            ("z_image_turbo_fp8_e4m3fn.safetensors", "Z-Image Turbo"),
            ("zimage/Z-Image-Turbo-Q8_0.gguf", "Z-Image Turbo"),
            ("qwen_image_2.1_int8_convrot.safetensors", "Qwen-Image 2.1"),
            ("Qwen-Image-2.1-bf16.safetensors", "Qwen-Image 2.1"),
            ("krea2_turbo_fp8_scaled.safetensors", "Krea 2 Turbo"),
            ("krea\\Krea-2-Turbo.safetensors", "Krea 2 Turbo"),
            ("krea2_raw_bf16.safetensors", "Krea 2 Raw"),
            ("qwen_image_fp8_e4m3fn.safetensors", "unknown model"),
            ("sd_xl_base_1.0.safetensors", "unknown model"),
            (CUSTOM_MODEL, "unknown model"),
            ("", "unknown model"),
        ] {
            assert_eq!(model_limits(file).name, name, "{file}");
        }
        let z = model_limits("z_image_turbo_bf16.safetensors");
        assert_eq!((z.max_side, z.max_pixels), (2048, 2_965_760));
        let qwen = model_limits("qwen_image_2.1_int8_convrot.safetensors");
        assert_eq!((qwen.max_side, qwen.max_pixels), (2752, 4_300_800));
        assert_eq!(UNKNOWN_MODEL_LIMITS.max_pixels, 1920 * 1088);
        // The Apple silicon bound: the 8× VAE's attention matrix stays within INT_MAX elements.
        assert!((MPS_VAE8_MAX_PIXELS / 64).pow(2) <= i32::MAX as u64);
        assert!((MPS_VAE8_MAX_PIXELS / 64 + 1).pow(2) > i32::MAX as u64);
    }

    /// The size the engine asks of `provider` for a display (as `Engine::request_size` does for a free-size
    /// provider: no more pixels than the display).
    fn size_for(provider: &ComfyUi, model: &str, display: (u32, u32)) -> (u32, u32) {
        let mut caps = provider.capabilities(model);
        if let Some(free) = caps.free_size.as_mut() {
            free.max_pixels = free.max_pixels.min(u64::from(display.0) * u64::from(display.1));
        }
        crate::imaging::choose_size(&caps, display.0, display.1)
    }

    #[test]
    fn sizes_follow_the_model_the_workflow_loads() {
        let http = Arc::new(StubHttp::new());
        let bundled = comfy(&http, None);
        let retina = (4112, 2658);
        // The bundled Z-Image Turbo: 2544×1632 broke VAEDecode on Apple silicon; now ≤ 2048 a side, ≤ 2.97 MP.
        assert_eq!(size_for(&bundled, "", retina), (2032, 1312));
        assert_eq!(size_for(&bundled, "", (3840, 2160)), (2048, 1152));
        assert_eq!(size_for(&bundled, "", (5120, 2880)), (2048, 1152));
        assert_eq!(size_for(&bundled, "", (1920, 1080)), (1904, 1072), "never more pixels than the display");
        // A model chosen for the bundled workflow is the one it loads.
        assert_eq!(size_for(&bundled, "qwen_image_2.1_int8_convrot.safetensors", retina), (2576, 1664));
        assert_eq!(size_for(&bundled, "sd_xl_base_1.0.safetensors", (3840, 2160)), (1904, 1072));
        // A person's workflow loads its own model, whatever is chosen in Settings.
        let krea = CUSTOM.replace("sd_xl_base_1.0.safetensors", "krea2_turbo_fp8_scaled.safetensors");
        let own = comfy(&http, Some(&krea));
        assert_eq!(size_for(&own, "qwen_image_2.1_int8_convrot.safetensors", retina), size_for(&bundled, "", retina));
        assert_eq!(size_for(&comfy(&http, Some(CUSTOM)), "", (3840, 2160)), (1904, 1072), "an unknown model");

        for model in ["", "qwen_image_2.1_int8_convrot.safetensors", "krea2_turbo_fp8_scaled.safetensors", "unknown.gguf"] {
            let limits = model_limits(if model.is_empty() { bundled.default_model() } else { model });
            for display in [(4112, 2658), (3456, 2234), (3840, 2160), (2560, 1600), (3440, 1440), (2160, 3840), (1000, 1000), (7680, 4320)] {
                let (w, h) = size_for(&bundled, model, display);
                let pixels = u64::from(w) * u64::from(h);
                assert!(w % 16 == 0 && h % 16 == 0, "{model} {display:?} -> {w}x{h}");
                assert!(w <= limits.max_side && h <= limits.max_side && pixels <= limits.max_pixels, "{model} {display:?} -> {w}x{h}");
                let aspect = |w: u32, h: u32| f64::from(w) / f64::from(h);
                let error = (aspect(w, h) / aspect(display.0, display.1)).ln().abs();
                assert!(error < 0.01, "{model} {display:?} -> {w}x{h}: aspect off by {error}");
            }
        }
    }

    #[tokio::test]
    async fn an_empty_prompt_is_refused() {
        let http = Arc::new(StubHttp::new());
        let mut req = request(None);
        req.prompt = " ".into();
        assert!(matches!(comfy(&http, None).generate(req).await, Err(AutoPaperError::InvalidInput { .. })));
        assert!(http.requests().is_empty());
    }

    /// Paints one small image on the ComfyUI at 127.0.0.1:8188 with the bundled template, if it answers.
    /// Run with `cargo test -p autopaper-core --lib comfyui::tests::live -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs a running ComfyUI with the bundled template's models"]
    async fn live_generation_against_local_comfyui() {
        let http = Arc::new(crate::net::ReqwestClient::new("live-test").unwrap());
        let probe = HttpRequest {
            method: HttpMethod::Get,
            url: "http://127.0.0.1:8188/system_stats".into(),
            policy: HostPolicy::UserEndpoint,
            headers: Vec::new(),
            body: None,
            timeout_secs: 5,
            max_response_bytes: 1 << 20,
        };
        match http.send(probe).await {
            Ok(response) if response.status == 200 => {}
            other => {
                eprintln!("skipped: ComfyUI isn't answering at 127.0.0.1:8188 ({:?})", other.map(|r| r.status));
                return;
            }
        }
        let provider = ComfyUi::new(http, "http://127.0.0.1:8188".into(), None);
        let started = std::time::Instant::now();
        let image = provider
            .generate(ImageRequest {
                model: String::new(),
                prompt: "Desktop wallpaper: a calm alpine lake at dawn, soft mist over the water, pale pink and blue sky, wide open space"
                    .into(),
                width: 512,
                height: 288,
                quality: ImageQuality::Standard,
                seed: Some(1001),
                progress: None,
                expected_secs: None,
            })
            .await
            .unwrap();
        let decoded = image::load_from_memory(&image.bytes).unwrap();
        eprintln!(
            "painted {}×{} {} ({} bytes) with {} in {:.1} s",
            decoded.width(),
            decoded.height(),
            image.mime,
            image.bytes.len(),
            image.model,
            started.elapsed().as_secs_f64()
        );
        assert_eq!((decoded.width(), decoded.height()), (512, 288));
    }

    /// Starts a 1920 × 1088 painting on the ComfyUI at 127.0.0.1:8188, drops the call after 8 s (as the engine
    /// does when the person cancels), and checks that ComfyUI stopped the job: its queue empties within seconds
    /// (the painting takes ~45 s on an M5 Max) and its history says it was interrupted. Run with
    /// `cargo test -p autopaper-core --lib comfyui::tests::live_cancel -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs a running ComfyUI (with /api/jobs/{id}/cancel, e.g. 0.36.0) with the bundled template's models, and nothing else queued"]
    async fn live_cancel_stops_the_job_in_comfyui() {
        let http: Arc<dyn HttpClient> = Arc::new(crate::net::ReqwestClient::new("live-test").unwrap());
        let get = |path: &str| HttpRequest {
            method: HttpMethod::Get,
            url: format!("http://127.0.0.1:8188{path}"),
            policy: HostPolicy::UserEndpoint,
            headers: Vec::new(),
            body: None,
            timeout_secs: 5,
            max_response_bytes: 16 << 20,
        };
        let queue = || async {
            let response = http.send(get("/queue")).await.unwrap();
            let queue: Value = serde_json::from_slice(&response.body).unwrap();
            let count = |key: &str| queue[key].as_array().map_or(0, Vec::len);
            (count("queue_running"), count("queue_pending"))
        };
        match http.send(get("/system_stats")).await {
            Ok(response) if response.status == 200 => {}
            other => {
                eprintln!("skipped: ComfyUI isn't answering at 127.0.0.1:8188 ({:?})", other.map(|r| r.status));
                return;
            }
        }
        assert_eq!(queue().await, (0, 0), "ComfyUI is busy; run this when nothing else is queued");
        let provider = ComfyUi::new(http.clone(), "http://127.0.0.1:8188".into(), None);
        let request = ImageRequest {
            model: String::new(),
            prompt: "Watercolour of a lighthouse on sea cliffs at dusk, soft fog over the water, a wide full-bleed scene".into(),
            width: 1920,
            height: 1088,
            quality: ImageQuality::High,
            seed: Some(7),
            progress: None,
            expected_secs: None,
        };
        let dropped = tokio::time::timeout(Duration::from_secs(8), provider.generate(request)).await;
        assert!(dropped.is_err(), "finished within 8 s: {dropped:?}");
        let stopped = std::time::Instant::now();
        let mut idle = false;
        for _ in 0..30 {
            if queue().await == (0, 0) {
                idle = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert!(idle, "ComfyUI is still painting {:.1} s after the cancel", stopped.elapsed().as_secs_f64());
        let history: Value = serde_json::from_slice(&http.send(get("/history?max_items=1")).await.unwrap().body).unwrap();
        let entry = history.as_object().and_then(|entries| entries.values().next()).cloned().unwrap_or_default();
        let messages = entry.pointer("/status/messages").cloned().unwrap_or_default();
        eprintln!(
            "ComfyUI idle {:.1} s after the cancel; last job: status {}, events {}",
            stopped.elapsed().as_secs_f64(),
            entry.pointer("/status/status_str").unwrap_or(&Value::Null),
            messages.as_array().map(|m| m.iter().filter_map(|e| e.get(0)?.as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default()
        );
        assert!(messages.to_string().contains("execution_interrupted"), "{entry}");
    }

    // ── Live (ignored by default): the ComfyUI at 127.0.0.1:8188, small sizes only ─────────────────────────────

    /// The real client, if ComfyUI answers and nothing is queued (so these tests never wait behind the person's own
    /// jobs or disturb them); `None` (skipped) otherwise.
    async fn live_client() -> Option<Arc<crate::net::ReqwestClient>> {
        let http = Arc::new(crate::net::ReqwestClient::new("live-test").unwrap());
        let queue = http.send(live_get("/queue")).await.ok().filter(|response| response.status == 200)?;
        let queue: Value = serde_json::from_slice(&queue.body).ok()?;
        let busy = ["queue_running", "queue_pending"].iter().any(|key| queue[key].as_array().is_some_and(|jobs| !jobs.is_empty()));
        if busy {
            eprintln!("skipped: ComfyUI is busy");
            return None;
        }
        Some(http)
    }

    fn live_get(path: &str) -> HttpRequest {
        HttpRequest {
            method: HttpMethod::Get,
            url: format!("http://127.0.0.1:8188{path}"),
            policy: HostPolicy::UserEndpoint,
            headers: Vec::new(),
            body: None,
            timeout_secs: 5,
            max_response_bytes: 16 << 20,
        }
    }

    /// The running job's id, from GET /queue.
    async fn live_running(http: &dyn HttpClient) -> Option<String> {
        let queue: Value = serde_json::from_slice(&http.send(live_get("/queue")).await.ok()?.body).ok()?;
        queue["queue_running"].get(0)?.get(1)?.as_str().map(String::from)
    }

    /// Waits (up to a minute) until ComfyUI's queue is empty; true when it is.
    async fn live_idle(http: &dyn HttpClient) -> bool {
        for _ in 0..120 {
            let queue: Value = match http.send(live_get("/queue")).await {
                Ok(response) => serde_json::from_slice(&response.body).unwrap_or_default(),
                Err(_) => Value::Null,
            };
            if ["queue_running", "queue_pending"].iter().all(|key| queue[key].as_array().is_some_and(Vec::is_empty)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        false
    }

    /// The last job's history events (`execution_success`, `execution_interrupted`, …).
    async fn live_last_events(http: &dyn HttpClient) -> String {
        let history: Value = serde_json::from_slice(&http.send(live_get("/history?max_items=1")).await.unwrap().body).unwrap();
        let entry = history.as_object().and_then(|entries| entries.values().next()).cloned().unwrap_or_default();
        let events = entry.pointer("/status/messages").and_then(Value::as_array).cloned().unwrap_or_default();
        events.iter().filter_map(|event| event.get(0)?.as_str().map(String::from)).collect::<Vec<_>>().join(", ")
    }

    fn live_request(model: &str, progress: Option<PaintProgress>) -> ImageRequest {
        ImageRequest {
            model: model.into(),
            prompt: "Watercolour of a lighthouse on sea cliffs at dusk, soft fog over the water, a wide full-bleed scene".into(),
            width: 768,
            height: 432,
            quality: ImageQuality::Standard,
            seed: Some(7),
            progress,
            expected_secs: None,
        }
    }

    const QWEN: &str = "qwen_image_2.1_int8_convrot.safetensors";

    /// Paints 768 × 432 with the default workflow and checks that every sampler step arrived from /ws, in order.
    /// `cargo test -p autopaper-core --lib comfyui::tests::live_progress -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs an idle ComfyUI at 127.0.0.1:8188 with the bundled template's models"]
    async fn live_progress_reaches_the_painter_s_observer() {
        let Some(http) = live_client().await else { return };
        let (progress, seen) = recorder();
        let started = std::time::Instant::now();
        let image = ComfyUi::new(http, "http://127.0.0.1:8188".into(), None).generate(live_request("", Some(progress))).await.unwrap();
        let steps = seen.lock().unwrap().clone();
        for (at, step) in &steps {
            eprintln!("  {:>6.2} s  step {}/{}", at.as_secs_f64(), step.done, step.total);
        }
        eprintln!("painted {} bytes with {} in {:.1} s", image.bytes.len(), image.model, started.elapsed().as_secs_f64());
        let counts: Vec<(u32, u32)> = steps.iter().map(|(_, step)| (step.done, step.total)).collect();
        assert_eq!(counts, (1..=8).map(|done| (done, 8)).collect::<Vec<_>>());
    }

    /// The real client whose /ws goes quiet after `steps` progress events while ComfyUI goes on painting: from
    /// AutoPaper's side, a job that stopped making progress.
    struct Muted {
        inner: Arc<crate::net::ReqwestClient>,
        steps: usize,
    }

    struct MutedSocket {
        inner: Box<dyn crate::ports::Socket>,
        left: usize,
    }

    #[async_trait]
    impl crate::ports::Socket for MutedSocket {
        async fn next_text(&mut self) -> Result<Option<String>> {
            if self.left == 0 {
                std::future::pending::<()>().await;
            }
            let text = self.inner.next_text().await?;
            if text.as_deref().is_some_and(|text| text.contains(r#""type": "progress""#) || text.contains(r#""type":"progress""#)) {
                self.left -= 1;
            }
            Ok(text)
        }
    }

    #[async_trait]
    impl HttpClient for Muted {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse> {
            self.inner.send(request).await
        }

        async fn open_socket(&self, request: HttpRequest) -> Result<Box<dyn crate::ports::Socket>> {
            Ok(Box::new(MutedSocket { inner: self.inner.open_socket(request).await?, left: self.steps }))
        }
    }

    /// Qwen-Image 2.1 at 768 × 432 (25 steps), with the step stall window shortened to 3 s: the events stop after
    /// step 3, so AutoPaper must call it stalled, say `TimedOut`, and stop the job in ComfyUI (idle at once, history
    /// `execution_interrupted`). `cargo test -p autopaper-core --lib comfyui::tests::live_stall -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs an idle ComfyUI at 127.0.0.1:8188 with Qwen-Image 2.1's files"]
    async fn live_stall_is_detected_and_the_job_stopped() {
        let Some(http) = live_client().await else { return };
        let muted: Arc<dyn HttpClient> = Arc::new(Muted { inner: http.clone(), steps: 3 });
        let mut provider = ComfyUi::new(muted, "http://127.0.0.1:8188".into(), None);
        provider.stall = StallWindows { step: Duration::from_secs(3), node: Duration::from_secs(300) };
        let (progress, seen) = recorder();
        let started = std::time::Instant::now();
        let result = provider.generate(live_request(QWEN, Some(progress))).await;
        let steps = seen.lock().unwrap().clone();
        let last = steps.last().map(|(at, step)| (at.as_secs_f64(), step.done));
        eprintln!("{result:?} after {:.1} s; last step heard: {last:?}", started.elapsed().as_secs_f64());
        assert!(
            matches!(&result, Err(AutoPaperError::ProviderUnavailable { reason: ProviderUnavailableReason::TimedOut, detail, .. }) if detail.starts_with("ComfyUI made no progress for")),
            "{result:?}"
        );
        let stopped = std::time::Instant::now();
        assert!(live_idle(http.as_ref()).await, "ComfyUI still painting");
        let events = live_last_events(http.as_ref()).await;
        eprintln!("ComfyUI idle {:.1} s after; last job's events: {events}", stopped.elapsed().as_secs_f64());
        assert!(events.contains("execution_interrupted"), "{events}");
    }

    /// The person stops the job in ComfyUI (its Cancel: POST /api/jobs/{id}/cancel) after a few steps: AutoPaper
    /// hears it on /ws and says `Stopped` within a second or two, not at the next poll or a timeout.
    /// `cargo test -p autopaper-core --lib comfyui::tests::live_stopped -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs an idle ComfyUI at 127.0.0.1:8188 with Qwen-Image 2.1's files"]
    async fn live_stopped_in_comfyui_is_reported_at_once() {
        let Some(http) = live_client().await else { return };
        let (progress, seen) = recorder();
        let provider = ComfyUi::new(http.clone(), "http://127.0.0.1:8188".into(), None);
        let painting = provider.generate(live_request(QWEN, Some(progress)));
        tokio::pin!(painting);
        let stopper = async {
            while seen.lock().unwrap().last().is_none_or(|(_, step)| step.done < 3) {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let id = live_running(http.as_ref()).await.expect("a running job");
            let cancel = HttpRequest { method: HttpMethod::Post, url: format!("http://127.0.0.1:8188/api/jobs/{id}/cancel"), ..live_get("") };
            http.send(cancel).await.unwrap();
            std::time::Instant::now()
        };
        let (result, cancelled_at) = tokio::join!(&mut painting, stopper);
        let heard_after = cancelled_at.elapsed();
        eprintln!("{result:?} {:.2} s after the cancel", heard_after.as_secs_f64());
        assert!(
            matches!(&result, Err(AutoPaperError::ProviderUnavailable { reason: ProviderUnavailableReason::Stopped, .. })),
            "{result:?}"
        );
        assert!(heard_after < Duration::from_secs(2), "heard on the socket, not at a later poll: {heard_after:?}");
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
    async fn too_slow_is_unavailable_not_absent() {
        match ComfyUi::new(Arc::new(Stalled), "http://127.0.0.1:8188".into(), None).generate(request(Some(1))).await {
            Err(AutoPaperError::ProviderUnavailable { provider: KIND, reason, detail }) => {
                assert_eq!(reason, ProviderUnavailableReason::TimedOut);
                assert_eq!(detail, "ComfyUI didn't answer within 30 s");
            }
            other => panic!("{other:?}"),
        }
    }
}
