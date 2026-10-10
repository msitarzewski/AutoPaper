//! Errors the hosts can act on. Each variant is something a UI can say plainly and localise; `detail`
//! strings are for logs and never contain keys, prompts' secrets, or response bodies verbatim.
//!
//! Hosts word errors themselves, in their own language: the variant (and, for `InvalidInput` and
//! `ProviderUnavailable`, its typed `reason`) says what happened. `detail` is English, for logs and support;
//! hosts shouldn't show it or match on its text.

use crate::model::{KeywordWeight, ProviderKind};

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AutoPaperError {
    #[error("no API key for {provider}")]
    MissingKey { provider: ProviderKind },
    #[error("{provider} rejected the API key")]
    InvalidKey { provider: ProviderKind },
    #[error("{provider} is unavailable: {detail}")]
    ProviderUnavailable { provider: ProviderKind, reason: ProviderUnavailableReason, detail: String },
    #[error("{provider} is rate-limiting requests")]
    RateLimited { provider: ProviderKind, retry_after_secs: u32 },
    #[error("{provider} declined to make this image")]
    Refused { provider: ProviderKind },
    #[error("{provider} can't {job}")]
    Unsupported { provider: ProviderKind, job: String },
    #[error("the monthly budget is spent")]
    BudgetReached { budget_cents: u32 },
    #[error("the network is unreachable")]
    Offline,
    #[error("unexpected response: {detail}")]
    InvalidResponse { detail: String },
    /// The painting provider ran but couldn't paint: ComfyUI rejected the workflow (a model file or node it
    /// doesn't have), a node failed while running (a model run through another model's graph, out of memory),
    /// it finished without saving an image, or AutoPaper has no workflow for the chosen model. Waiting won't fix
    /// it (not transient): hosts say so in one line and link to Settings → Providers. `model` is the model in
    /// words as Settings lists it ("Qwen-Image 2.1"; a person's own workflow: its model file without the
    /// extension), empty when it isn't known; `detail` is English, for logs.
    #[error("{provider} couldn't paint with {model}: {detail}")]
    PaintingFailed { provider: ProviderKind, model: String, detail: String },
    /// The writing model kept breaking one of the mood's keywords: no idea it wrote, on any retry, could be used,
    /// and most were turned down for leaving out this Must keyword (`weight` Must) or bringing in this Avoid one
    /// (`weight` Avoid). Not the provider's fault, and asking again rarely helps: hosts name the keyword in one line
    /// and link to the mood (`mood_id`), where it can be reworded, made a Maybe or removed.
    #[error("none of the text model's ideas followed the keywords: {weight:?} keyword {keyword:?}")]
    KeywordNotFollowed { keyword: String, weight: KeywordWeight, mood_id: String },
    #[error("invalid input: {detail}")]
    InvalidInput { reason: InvalidInputReason, detail: String },
    #[error("not found")]
    NotFound,
    #[error("nothing liked to revisit yet")]
    NothingToRevisit,
    #[error("storage error: {detail}")]
    Storage { detail: String },
    #[error("cancelled")]
    Cancelled,
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

/// What was wrong with what the host passed in (`AutoPaperError::InvalidInput`), so hosts can word it and point
/// at the right field. `Other` covers mistakes a host makes rather than a person (a zero-sized image, a
/// malformed header), which hosts can report generically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum InvalidInputReason {
    /// A keyword with no letters or digits (empty once trimmed).
    KeywordEmpty,
    /// A keyword over `Keyword::MAX_LEN` (40) characters.
    KeywordTooLong,
    /// Adding a keyword past `Keyword::MAX_COUNT` (64).
    TooManyKeywords,
    /// Renaming a keyword to another keyword's text (case-insensitive). Adding a duplicate isn't an error:
    /// `add_keyword` returns the existing keyword with its weight updated.
    DuplicateKeyword,
    /// An OpenAI-compatible server needs an address and none is set.
    AddressMissing,
    /// An address the network policy refuses: plain http to a host outside this computer and the local
    /// network, a scheme other than http/https, or a user name or password in the address.
    AddressNotAllowed,
    /// Not a complete web address (no scheme or host).
    AddressInvalid,
    /// A display size outside 1–16384 pixels a side (`set_display_hint`, `render_for_display`).
    DisplaySizeInvalid,
    /// The local server has no models to use yet (Ollama with nothing downloaded; an OpenAI-compatible server
    /// that lists none), and no model is chosen in Settings.
    NoModels,
    /// The person's ComfyUI workflow has no `{{prompt}}` placeholder.
    WorkflowNeedsPrompt,
    /// The person's ComfyUI workflow was saved in UI format, not API format (Export (API) in ComfyUI).
    WorkflowNotApiFormat,
    /// The person's ComfyUI workflow isn't valid JSON once its placeholders are filled in.
    WorkflowInvalid,
    /// "Make an Echo" with nothing to echo yet (no finished wallpaper), or of one that didn't finish.
    NothingToEcho,
    /// A mood name with nothing visible in it (empty once trimmed).
    MoodNameEmpty,
    /// A mood name over `Mood::MAX_NAME_LEN` (40) characters.
    MoodNameTooLong,
    /// Another mood already has this name (case-insensitive).
    DuplicateMoodName,
    /// Deleting the only mood: there is always one.
    LastMood,
    /// Anything else (host mistakes; see `detail`).
    Other,
}

/// Why a provider couldn't be reached or didn't finish (`AutoPaperError::ProviderUnavailable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ProviderUnavailableReason {
    /// The provider took longer than AutoPaper waits (hosted: the request's timeout, HTTP 408/504; local: the
    /// server didn't answer in time, or a ComfyUI job stopped making progress — no new step for a few minutes —
    /// or ran far past its usual time: past 30 minutes and three times its estimate).
    TimedOut,
    /// Nothing answers at a local server's address (Ollama, ComfyUI, a local OpenAI-compatible server): it
    /// isn't running, or the address is wrong.
    NotRunning,
    /// The provider answered with a server error (HTTP 500, 502, 503).
    ServerError,
    /// The job was stopped on the server (ComfyUI's own Cancel or Interrupt).
    Stopped,
    /// Anything else (see `detail`).
    Other,
}

impl AutoPaperError {
    /// Worth retrying later without the person doing anything.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::ProviderUnavailable { .. } | Self::RateLimited { .. } | Self::Offline
        )
    }

    /// `InvalidInput` with its reason and an English detail for logs.
    pub fn invalid_input(reason: InvalidInputReason, detail: impl Into<String>) -> Self {
        Self::InvalidInput { reason, detail: detail.into() }
    }

    /// `ProviderUnavailable` with its reason and an English detail for logs.
    pub fn unavailable(provider: ProviderKind, reason: ProviderUnavailableReason, detail: impl Into<String>) -> Self {
        Self::ProviderUnavailable { provider, reason, detail: detail.into() }
    }

    /// The same error said of `provider` (a `ProviderUnavailable` from the HTTP client names no provider of its
    /// own); other errors are returned unchanged.
    pub fn for_provider(self, provider: ProviderKind) -> Self {
        match self {
            Self::ProviderUnavailable { reason, detail, .. } => Self::ProviderUnavailable { provider, reason, detail },
            other => other,
        }
    }
}

impl From<rusqlite::Error> for AutoPaperError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage { detail: error.to_string() }
    }
}

impl From<std::io::Error> for AutoPaperError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage { detail: error.to_string() }
    }
}

impl From<serde_json::Error> for AutoPaperError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidResponse { detail: error.to_string() }
    }
}

pub type Result<T, E = AutoPaperError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_name_providers_the_way_people_do() {
        let unavailable = AutoPaperError::ProviderUnavailable {
            provider: ProviderKind::OpenAiCompatible,
            reason: ProviderUnavailableReason::Other,
            detail: "The service isn't there.".into(),
        };
        assert_eq!(unavailable.to_string(), "OpenAI-compatible is unavailable: The service isn't there.");
        assert_eq!(AutoPaperError::MissingKey { provider: ProviderKind::Google }.to_string(), "no API key for Google Gemini");
    }
}
