//! AutoPaper's core: the agent behind every AutoPaper app.
//!
//! Hosts (the macOS, Windows and Linux apps, and the `autopaper` CLI) hold one [`Engine`] and drive it.
//! Everything that decides *what* to make lives here; hosts own the UI, timers and OS integration.
//! Design: `memory-bank/systemPatterns.md`.

uniffi::setup_scaffolding!();

pub mod composer;
pub mod echo;
pub mod embed;
pub mod engine;
pub mod error;
pub mod imaging;
pub mod model;
pub mod net;
pub mod novelty;
pub mod perf;
pub mod ports;
pub mod pricing;
pub mod providers;
pub mod schedule;
pub mod store;
pub mod taste;
pub mod testing;
pub mod text;

pub use composer::surprise_band;
pub use engine::{Engine, EngineConfig, MemoryStatus, RevisitReason, Shown};
pub use error::{AutoPaperError, InvalidInputReason, ProviderUnavailableReason, Result};
pub use model::*;
pub use ports::{ProgressDetailObserver, ProgressObserver, SecretStore};
pub use pricing::prices_as_of;
pub use providers::registry::{default_base_url, default_model};
