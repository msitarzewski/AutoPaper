//! Builds providers from settings. Adding a provider = one file + one arm here.

use std::sync::Arc;

use crate::error::{AutoPaperError, InvalidInputReason, Result};
use crate::model::{ProviderJob, ProviderKind, ProviderSelection};
use crate::net::{HostPolicy, check_url};
use crate::ports::{HttpClient, SecretStore};

use super::comfyui::ComfyUi;
use super::demo::Demo;
use super::google::Google;
use super::ollama::Ollama;
use super::openai::OpenAi;
use super::openai_compat::OpenAiCompatible;
use super::{ImageProvider, TextProvider};

/// What every provider may use.
#[derive(Clone)]
pub struct ProviderDeps {
    pub http: Arc<dyn HttpClient>,
    pub secrets: Arc<dyn SecretStore>,
    /// The person's ComfyUI workflow (API format with placeholders), if they set one.
    pub comfyui_workflow: Option<String>,
}

/// A provider that writes concepts. `Unsupported` for kinds that can't (ComfyUI). Local kinds use
/// `selection.base_url`, else `default_base_url`; OpenAI-compatible needs a base URL (`InvalidInput`
/// `AddressMissing`). A base URL the network policy refuses is `InvalidInput` (`AddressNotAllowed` or
/// `AddressInvalid`) here, before any request. Demo gets a fresh random seed (callers wanting a fixed one build `Demo::new` themselves).
pub fn text_provider(selection: &ProviderSelection, deps: &ProviderDeps) -> Result<Arc<dyn TextProvider>> {
    match selection.kind {
        kind if !kind.writes_concepts() => Err(unsupported(kind, "write concepts")),
        ProviderKind::OpenAi => Ok(Arc::new(OpenAi::new(deps.http.clone(), deps.secrets.clone()))),
        ProviderKind::Google => Ok(Arc::new(Google::new(deps.http.clone(), deps.secrets.clone()))),
        ProviderKind::Ollama => Ok(Arc::new(Ollama::new(deps.http.clone(), base_url(selection)?))),
        ProviderKind::OpenAiCompatible => {
            Ok(Arc::new(OpenAiCompatible::new(deps.http.clone(), deps.secrets.clone(), base_url(selection)?)))
        }
        ProviderKind::Demo => Ok(Arc::new(Demo::new(rand::random()))),
        kind @ ProviderKind::ComfyUi => Err(unsupported(kind, "write concepts")),
    }
}

/// A provider that makes images. `Unsupported` for kinds that can't (Ollama). Base URLs as for
/// `text_provider`; ComfyUI also gets `deps.comfyui_workflow`.
pub fn image_provider(selection: &ProviderSelection, deps: &ProviderDeps) -> Result<Arc<dyn ImageProvider>> {
    match selection.kind {
        kind if !kind.makes_images() => Err(unsupported(kind, "make images")),
        ProviderKind::OpenAi => Ok(Arc::new(OpenAi::new(deps.http.clone(), deps.secrets.clone()))),
        ProviderKind::Google => Ok(Arc::new(Google::new(deps.http.clone(), deps.secrets.clone()))),
        ProviderKind::OpenAiCompatible => {
            Ok(Arc::new(OpenAiCompatible::new(deps.http.clone(), deps.secrets.clone(), base_url(selection)?)))
        }
        ProviderKind::ComfyUi => Ok(Arc::new(ComfyUi::new(deps.http.clone(), base_url(selection)?, deps.comfyui_workflow.clone()))),
        ProviderKind::Demo => Ok(Arc::new(Demo::new(rand::random()))),
        kind @ ProviderKind::Ollama => Err(unsupported(kind, "make images")),
    }
}

/// Default base URL for local kinds; `None` for hosted ones and OpenAI-compatible (the person sets it).
pub fn default_base_url(kind: ProviderKind) -> Option<&'static str> {
    match kind {
        ProviderKind::Ollama => Some("http://127.0.0.1:11434"),
        ProviderKind::ComfyUi => Some("http://127.0.0.1:8188"),
        _ => None,
    }
}

/// `default_base_url` for hosts: the address a local kind uses when none is set (Settings shows it as the
/// address field's placeholder); `None` for hosted kinds, Demo and OpenAI-compatible (the person sets that).
/// Exported through UniFFI as `default_base_url(kind) -> String?`.
#[uniffi::export(name = "default_base_url")]
pub fn default_base_url_for_host(kind: ProviderKind) -> Option<String> {
    default_base_url(kind).map(String::from)
}

/// The model a provider uses for `job` when Settings leaves the model blank, so a model picker can say
/// "Default (gpt-6-luna)". Empty when there's no fixed default: Ollama uses the first model it has installed and
/// an OpenAI-compatible server the first it lists (hosts say "the server's first model" or just "Default"),
/// and when `kind` can't do `job` (ComfyUI writing, Ollama painting). ComfyUI's is the bundled workflow's
/// model; a person's own workflow always uses its own. Demo's is "demo".
#[uniffi::export]
pub fn default_model(kind: ProviderKind, job: ProviderJob) -> String {
    let model = match (kind, job) {
        (ProviderKind::OpenAi, ProviderJob::Concepts) => super::openai::DEFAULT_TEXT_MODEL,
        (ProviderKind::OpenAi, ProviderJob::Images) => super::openai::DEFAULT_IMAGE_MODEL,
        (ProviderKind::Google, ProviderJob::Concepts) => super::google::DEFAULT_TEXT_MODEL,
        (ProviderKind::Google, ProviderJob::Images) => super::google::DEFAULT_IMAGE_MODEL,
        (ProviderKind::ComfyUi, ProviderJob::Images) => return super::comfyui::bundled_model().unwrap_or_default(),
        (ProviderKind::Demo, _) => super::demo::MODEL,
        (ProviderKind::Ollama | ProviderKind::OpenAiCompatible, _) | (ProviderKind::ComfyUi, ProviderJob::Concepts) => "",
    };
    model.to_string()
}

fn unsupported(provider: ProviderKind, job: &str) -> AutoPaperError {
    AutoPaperError::Unsupported { provider, job: job.into() }
}

/// The selection's base URL (trimmed, without a trailing slash), else the kind's default, checked
/// against `HostPolicy::UserEndpoint`.
fn base_url(selection: &ProviderSelection) -> Result<String> {
    let chosen = selection.base_url.as_deref().map(str::trim).filter(|url| !url.is_empty());
    let url = chosen
        .or_else(|| default_base_url(selection.kind))
        .ok_or_else(|| AutoPaperError::invalid_input(InvalidInputReason::AddressMissing, "Enter the server's address"))?;
    check_url(url, &HostPolicy::UserEndpoint)?;
    Ok(url.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{StubHttp, StubSecrets};

    fn deps(workflow: Option<&str>) -> ProviderDeps {
        ProviderDeps {
            http: Arc::new(StubHttp::new()),
            secrets: Arc::new(StubSecrets::default()),
            comfyui_workflow: workflow.map(String::from),
        }
    }

    fn selection(kind: ProviderKind, base_url: Option<&str>) -> ProviderSelection {
        ProviderSelection { kind, model: String::new(), base_url: base_url.map(String::from) }
    }

    const ALL: [ProviderKind; 6] = [
        ProviderKind::OpenAi,
        ProviderKind::Google,
        ProviderKind::Ollama,
        ProviderKind::OpenAiCompatible,
        ProviderKind::ComfyUi,
        ProviderKind::Demo,
    ];

    fn usable(kind: ProviderKind) -> ProviderSelection {
        let base = (kind == ProviderKind::OpenAiCompatible).then_some("http://127.0.0.1:1234");
        selection(kind, base)
    }

    #[test]
    fn maps_each_kind_to_its_provider_or_unsupported() {
        for kind in ALL {
            match text_provider(&usable(kind), &deps(None)) {
                Ok(_) => assert!(kind.writes_concepts(), "{kind:?}"),
                Err(AutoPaperError::Unsupported { provider, job }) => {
                    assert_eq!((provider, job.as_str()), (kind, "write concepts"));
                    assert!(!kind.writes_concepts());
                }
                Err(other) => panic!("{kind:?}: {other:?}"),
            }
            match image_provider(&usable(kind), &deps(None)) {
                Ok(_) => assert!(kind.makes_images(), "{kind:?}"),
                Err(AutoPaperError::Unsupported { provider, job }) => {
                    assert_eq!((provider, job.as_str()), (kind, "make images"));
                    assert!(!kind.makes_images());
                }
                Err(other) => panic!("{kind:?}: {other:?}"),
            }
        }
        assert!(matches!(
            text_provider(&usable(ProviderKind::ComfyUi), &deps(None)),
            Err(AutoPaperError::Unsupported { provider: ProviderKind::ComfyUi, .. })
        ));
        assert!(matches!(
            image_provider(&usable(ProviderKind::Ollama), &deps(None)),
            Err(AutoPaperError::Unsupported { provider: ProviderKind::Ollama, .. })
        ));
    }

    #[test]
    fn local_kinds_offer_their_default_address() {
        assert_eq!(default_base_url(ProviderKind::Ollama), Some("http://127.0.0.1:11434"));
        assert_eq!(default_base_url(ProviderKind::ComfyUi), Some("http://127.0.0.1:8188"));
        for kind in [ProviderKind::OpenAi, ProviderKind::Google, ProviderKind::OpenAiCompatible, ProviderKind::Demo] {
            assert_eq!(default_base_url(kind), None, "{kind:?}");
        }
        for kind in ALL {
            assert_eq!(default_base_url_for_host(kind).as_deref(), default_base_url(kind), "{kind:?}");
        }
    }

    #[test]
    fn default_models_match_what_the_providers_use() {
        let jobs = [ProviderJob::Concepts, ProviderJob::Images];
        for kind in ALL {
            for job in jobs {
                let built = match job {
                    ProviderJob::Concepts => text_provider(&usable(kind), &deps(None)).map(|p| p.default_model().to_string()),
                    ProviderJob::Images => image_provider(&usable(kind), &deps(None)).map(|p| p.default_model().to_string()),
                };
                // A kind that can't do the job has no default; the others say what they'd use.
                assert_eq!(default_model(kind, job), built.unwrap_or_default(), "{kind:?} {job:?}");
            }
        }
        assert_eq!(default_model(ProviderKind::OpenAi, ProviderJob::Concepts), "gpt-6-luna");
        assert_eq!(default_model(ProviderKind::OpenAi, ProviderJob::Images), "gpt-image-2.5-flare");
        assert_eq!(default_model(ProviderKind::Google, ProviderJob::Concepts), "gemini-3.5-flash-lite");
        assert_eq!(default_model(ProviderKind::Google, ProviderJob::Images), "gemini-3.1-flash-image");
        assert_eq!(default_model(ProviderKind::ComfyUi, ProviderJob::Images), "z_image_turbo_bf16.safetensors");
        assert_eq!(default_model(ProviderKind::Ollama, ProviderJob::Concepts), "");
        assert_eq!(default_model(ProviderKind::OpenAiCompatible, ProviderJob::Images), "");
        assert_eq!(default_model(ProviderKind::Demo, ProviderJob::Images), "demo");
    }

    #[test]
    fn chooses_the_base_url() {
        assert_eq!(base_url(&selection(ProviderKind::Ollama, None)).unwrap(), "http://127.0.0.1:11434");
        assert_eq!(base_url(&selection(ProviderKind::Ollama, Some("  "))).unwrap(), "http://127.0.0.1:11434");
        assert_eq!(base_url(&selection(ProviderKind::ComfyUi, None)).unwrap(), "http://127.0.0.1:8188");
        assert_eq!(base_url(&selection(ProviderKind::Ollama, Some(" http://studio.local:11434/ "))).unwrap(), "http://studio.local:11434");
        assert_eq!(
            base_url(&selection(ProviderKind::OpenAiCompatible, Some("https://api.together.example/v1"))).unwrap(),
            "https://api.together.example/v1"
        );
    }

    #[test]
    fn openai_compatible_needs_an_address() {
        for base in [None, Some(""), Some("   ")] {
            for result in [
                text_provider(&selection(ProviderKind::OpenAiCompatible, base), &deps(None)).map(|_| ()),
                image_provider(&selection(ProviderKind::OpenAiCompatible, base), &deps(None)).map(|_| ()),
            ] {
                match result {
                    Err(AutoPaperError::InvalidInput { reason, detail }) => {
                        assert_eq!(reason, InvalidInputReason::AddressMissing);
                        assert_eq!(detail, "Enter the server's address");
                    }
                    other => panic!("{base:?}: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn refuses_addresses_the_network_policy_refuses() {
        for (kind, base) in [
            (ProviderKind::Ollama, "http://ollama.example.com:11434"),
            (ProviderKind::OpenAiCompatible, "ftp://127.0.0.1"),
            (ProviderKind::OpenAiCompatible, "http://user:pw@127.0.0.1:1234"),
        ] {
            let result = text_provider(&selection(kind, Some(base)), &deps(None));
            assert!(
                matches!(result, Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::AddressNotAllowed, .. })),
                "{base}"
            );
        }
        let result = image_provider(&selection(ProviderKind::ComfyUi, Some("http://8.8.8.8:8188")), &deps(None));
        assert!(matches!(result, Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::AddressNotAllowed, .. })));
        let result = text_provider(&selection(ProviderKind::Ollama, Some("127.0.0.1:11434")), &deps(None));
        assert!(matches!(result, Err(AutoPaperError::InvalidInput { reason: InvalidInputReason::AddressInvalid, .. })));
    }

    #[test]
    fn local_providers_report_their_kind() {
        for kind in [ProviderKind::Ollama, ProviderKind::OpenAiCompatible, ProviderKind::Demo] {
            assert_eq!(text_provider(&usable(kind), &deps(None)).map(|p| p.kind()).ok(), Some(kind));
        }
        for kind in [ProviderKind::OpenAiCompatible, ProviderKind::ComfyUi, ProviderKind::Demo] {
            assert_eq!(image_provider(&usable(kind), &deps(None)).map(|p| p.kind()).ok(), Some(kind));
        }
    }

    #[test]
    fn comfyui_gets_the_persons_workflow() {
        let mine = r#"{"4": {"class_type": "CheckpointLoaderSimple", "inputs": {"ckpt_name": "mine.safetensors"}},
                       "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "{{prompt}}"}}}"#;
        let comfy = image_provider(&usable(ProviderKind::ComfyUi), &deps(Some(mine))).unwrap();
        assert_eq!(comfy.default_model(), "mine.safetensors");
        let bundled = image_provider(&usable(ProviderKind::ComfyUi), &deps(None)).unwrap();
        assert_eq!(bundled.default_model(), "z_image_turbo_bf16.safetensors");
    }
}
