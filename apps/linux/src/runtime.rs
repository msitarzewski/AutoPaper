//! The tokio runtime the engine runs on, and the bridge back to the GTK main loop.
//!
//! The GTK main loop never waits on the engine: sync engine calls go to tokio's blocking pool (`blocking`), async
//! ones run on its workers (`spawn`), and the main loop awaits the result inside `glib::spawn_future_local`.
//! The main thread also enters the runtime's context (see `main`), so portal futures (ashpd/zbus on tokio) can be
//! polled from the main loop while the runtime's workers drive their I/O.

use std::future::Future;
use std::sync::OnceLock;

use autopaper_core::AutoPaperError;

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Builds the runtime. Called once, first thing in `main`.
pub fn init() -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("autopaper-engine")
        .enable_all()
        .build()?;
    let _ = RUNTIME.set(runtime);
    Ok(())
}

pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get().expect("runtime::init runs before anything uses the runtime")
}

/// Runs blocking work (a sync engine call, file I/O, image decoding) on the runtime's blocking pool.
pub async fn blocking<T, F>(work: F) -> Result<T, AutoPaperError>
where
    F: FnOnce() -> Result<T, AutoPaperError> + Send + 'static,
    T: Send + 'static,
{
    joined(runtime().spawn_blocking(work).await)
}

/// Runs a future (an async engine call) on the runtime's workers.
pub async fn spawn<T, F>(future: F) -> Result<T, AutoPaperError>
where
    F: Future<Output = Result<T, AutoPaperError>> + Send + 'static,
    T: Send + 'static,
{
    joined(runtime().spawn(future).await)
}

fn joined<T>(result: Result<Result<T, AutoPaperError>, tokio::task::JoinError>) -> Result<T, AutoPaperError> {
    result.unwrap_or_else(|error| Err(AutoPaperError::Internal { detail: format!("a background task failed: {error}") }))
}
