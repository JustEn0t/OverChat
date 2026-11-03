//! Tokens live in Windows Credential Manager (DPAPI-encrypted, per Windows user),
//! never in plain files or browser storage.

use keyring::Entry;
use serde::{Deserialize, Serialize};

const SERVICE: &str = "OverChat";
const ACCOUNT: &str = "twitch";

#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

fn entry() -> keyring::Result<Entry> {
    Entry::new(SERVICE, ACCOUNT)
}

pub fn load() -> Option<Credentials> {
    let secret = entry().ok()?.get_password().ok()?;
    serde_json::from_str(&secret).ok()
}

pub fn save(credentials: &Credentials) -> Result<(), String> {
    let secret = serde_json::to_string(credentials).map_err(|e| e.to_string())?;
    entry()
        .and_then(|e| e.set_password(&secret))
        .map_err(|e| format!("не удалось сохранить вход в диспетчере учётных данных Windows: {e}"))
}

pub fn delete() {
    if let Ok(entry) = entry() {
        let _ = entry.delete_credential();
    }
}
