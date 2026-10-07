//! API keys in the Secret Service (GNOME Keyring, KWallet's Secret Service, KeePassXC) through oo7; inside
//! Flatpak, oo7 keeps them in an encrypted file keyed by the Secret portal. Never logged, never in errors.
//!
//! The core calls `SecretStore` synchronously from its own threads (tokio workers during a generation), while
//! oo7 is async. So the keyring lives on one dedicated thread with its own small runtime, and each call waits
//! for its answer there: no runtime is blocked from inside itself, and the session-bus connection is reused.

use std::sync::Arc;
use std::sync::mpsc;

use autopaper_core::{OPENAI_COMPATIBLE_ACCOUNT_PREFIX, SecretStore};

use crate::desktop::APP_ID;

enum Op {
    Get(String),
    Set(String, String),
    Delete(String),
}

struct Request {
    op: Op,
    reply: mpsc::SyncSender<Result<Option<String>, String>>,
}

pub struct KeyringSecrets {
    requests: mpsc::Sender<Request>,
}

impl KeyringSecrets {
    pub fn start() -> Arc<Self> {
        let (requests, incoming) = mpsc::channel::<Request>();
        let spawned = std::thread::Builder::new().name("autopaper-secrets".into()).spawn(move || serve(incoming));
        if let Err(error) = spawned {
            tracing::error!(%error, "couldn't start the keyring thread; keys can't be read or saved");
        }
        Arc::new(Self { requests })
    }

    /// Reads a key. `Err` says why the keyring couldn't be read (locked and the unlock was dismissed, no Secret
    /// Service running); the error never contains the key.
    pub fn read(&self, account: &str) -> Result<Option<String>, String> {
        self.call(Op::Get(account.to_string()))
    }

    /// Saves a key; an empty value deletes it.
    pub fn write(&self, account: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        let op = if value.is_empty() {
            Op::Delete(account.to_string())
        } else {
            Op::Set(account.to_string(), value.to_string())
        };
        self.call(op).map(|_| ())
    }

    fn call(&self, op: Op) -> Result<Option<String>, String> {
        let (reply, answer) = mpsc::sync_channel(1);
        self.requests.send(Request { op, reply }).map_err(|_| "the keyring thread isn't running".to_string())?;
        answer.recv().map_err(|_| "the keyring thread stopped".to_string())?
    }
}

impl SecretStore for KeyringSecrets {
    fn get(&self, account: String) -> Option<String> {
        self.read(&account).unwrap_or_else(|problem| {
            tracing::warn!(account, problem, "couldn't read a key from the keyring");
            None
        })
    }

    fn set(&self, account: String, value: String) {
        if let Err(problem) = self.write(&account, &value) {
            tracing::warn!(account, problem, "couldn't save a key to the keyring");
        }
    }

    fn delete(&self, account: String) {
        if let Err(problem) = self.write(&account, "") {
            tracing::warn!(account, problem, "couldn't delete a key from the keyring");
        }
    }
}

fn serve(incoming: mpsc::Receiver<Request>) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            for request in incoming {
                let _ = request.reply.send(Err(format!("no runtime for the keyring: {error}")));
            }
            return;
        }
    };
    let mut keyring: Option<oo7::Keyring> = None;
    for request in incoming {
        let result = runtime.block_on(handle(&mut keyring, request.op));
        if result.is_err() {
            // Reconnect next time (the Secret Service may have restarted).
            keyring = None;
        }
        let _ = request.reply.send(result);
    }
}

async fn handle(slot: &mut Option<oo7::Keyring>, op: Op) -> Result<Option<String>, String> {
    if slot.is_none() {
        *slot = Some(oo7::Keyring::new().await.map_err(|error| error.to_string())?);
    }
    let Some(keyring) = slot.as_ref() else {
        return Err("no keyring".into());
    };
    let account = match &op {
        Op::Get(account) | Op::Set(account, _) | Op::Delete(account) => account.clone(),
    };
    let attributes = [("application", APP_ID), ("account", account.as_str())];
    let failed = |error: oo7::Error| error.to_string();
    match op {
        Op::Get(_) => {
            let items = keyring.search_items(&attributes).await.map_err(failed)?;
            let Some(item) = items.into_iter().next() else {
                return Ok(None);
            };
            if item.is_locked().await.map_err(failed)? {
                item.unlock().await.map_err(failed)?;
            }
            let secret = item.secret().await.map_err(failed)?;
            Ok(Some(String::from_utf8_lossy(&secret).into_owned()))
        }
        Op::Set(_, value) => {
            if keyring.is_locked().await.map_err(failed)? {
                keyring.unlock().await.map_err(failed)?;
            }
            keyring.create_item(&label(&account), &attributes, value.as_str(), true).await.map_err(failed)?;
            Ok(None)
        }
        Op::Delete(_) => {
            keyring.delete(&attributes).await.map_err(failed)?;
            Ok(None)
        }
    }
}

/// The item's name in the keyring manager (Seahorse, KWalletManager).
fn label(account: &str) -> String {
    match account {
        "openai.api_key" => "AutoPaper: OpenAI API key".into(),
        "google.api_key" => "AutoPaper: Google Gemini API key".into(),
        other => match other.strip_prefix(OPENAI_COMPATIBLE_ACCOUNT_PREFIX) {
            Some(origin) => format!("AutoPaper: key for {origin}"),
            None => format!("AutoPaper: {other}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a running, unlocked Secret Service; `scripts/linux-vm.sh` runs it against a throwaway gnome-keyring
    /// in its own D-Bus session (`cargo test -p autopaper-linux -- --ignored`), never the person's keyring.
    #[test]
    #[ignore = "needs an unlocked Secret Service"]
    fn keys_round_trip_through_the_secret_service() {
        let secrets = KeyringSecrets::start();
        let account = "autopaper-test.api_key";
        assert_eq!(secrets.read(account), Ok(None));
        secrets.write(account, "  sk-test-value  ").expect("save");
        assert_eq!(secrets.read(account), Ok(Some("sk-test-value".into())), "saved trimmed");
        secrets.write(account, "sk-replaced").expect("replace");
        assert_eq!(secrets.get(account.into()), Some("sk-replaced".into()), "SecretStore::get sees the new key");
        secrets.write(account, "").expect("an empty value deletes");
        assert_eq!(secrets.read(account), Ok(None));
    }

    #[test]
    fn keyring_labels_name_the_service() {
        assert_eq!(label("openai.api_key"), "AutoPaper: OpenAI API key");
        assert_eq!(label("google.api_key"), "AutoPaper: Google Gemini API key");
        assert_eq!(
            label(&format!("{OPENAI_COMPATIBLE_ACCOUNT_PREFIX}http://192.168.1.20:1234")),
            "AutoPaper: key for http://192.168.1.20:1234"
        );
    }
}
