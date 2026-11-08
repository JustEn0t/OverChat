//! Anonymous Twitch chat by channel name. No accounts, OAuth or tokens.

mod emotes;
mod irc;
mod message;

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::settings;

pub const CHAT_EVENT: &str = "chat-message";
pub const CHAT_DELETE_EVENT: &str = "chat-delete";
const CHAT_STATUS_EVENT: &str = "chat-status";

const MAX_BACKOFF_SECS: u64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum TwitchError {
    #[error("Нет связи с Twitch. Проверьте подключение к интернету.")]
    Network(#[from] reqwest::Error),
    #[error("Соединение с чатом прервалось: {0}")]
    Socket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("{0}")]
    Api(String),
    /// Retrying won't help (e.g. no such channel).
    #[error("{0}")]
    Fatal(String),
}

#[derive(Clone, Default, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChatStatus {
    #[default]
    Disconnected,
    Connecting {
        channel: String,
    },
    Connected {
        channel: String,
        seventv_emotes: usize,
    },
    Reconnecting {
        channel: String,
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Default)]
pub struct Twitch {
    chat_status: Mutex<ChatStatus>,
    channel: Mutex<Option<String>>,
    chat_task: Mutex<Option<JoinHandle<()>>>,
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("OverChat/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("HTTP client")
}

pub(crate) fn set_chat_status(app: &AppHandle, status: ChatStatus) {
    *app.state::<Twitch>().chat_status.lock().unwrap() = status.clone();
    let _ = app.emit(CHAT_STATUS_EVENT, status);
}

/// Replaces the task in `slot` (aborting the old one), or just stops it when `future` is None.
fn replace_task(slot: &Mutex<Option<JoinHandle<()>>>, future: Option<impl Future<Output = ()> + Send + 'static>) {
    let mut task = slot.lock().unwrap();
    if let Some(previous) = task.take() {
        previous.abort();
    }
    *task = future.map(tauri::async_runtime::spawn);
}

const NO_TASK: Option<std::future::Ready<()>> = None;

// ── Chat ────────────────────────────────────────────────────────────────

/// Accepts `name`, `#name`, `twitch.tv/name` or a full link; returns the login in lower case.
fn normalize_channel(input: &str) -> Option<String> {
    let trimmed = input.trim().trim_end_matches('/');
    let name = trimmed.rsplit('/').next().unwrap_or(trimmed).trim_start_matches('#');
    let name = name.split(['?', '#']).next().unwrap_or(name).to_lowercase();
    let valid = (1..=25).contains(&name.len())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then_some(name)
}

async fn run_chat(app: AppHandle, channel: String) {
    let http = http_client();
    let mut backoff = 1;
    set_chat_status(&app, ChatStatus::Connecting { channel: channel.clone() });

    loop {
        let err = match irc::run(&app, &http, &channel, &mut backoff).await {
            Ok(()) => continue,
            Err(err) => err,
        };
        if let TwitchError::Fatal(message) = err {
            return set_chat_status(&app, ChatStatus::Error { message });
        }
        set_chat_status(
            &app,
            ChatStatus::Reconnecting {
                channel: channel.clone(),
                message: format!("{err} Повтор через {backoff} с…"),
            },
        );
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(MAX_BACKOFF_SECS);
    }
}

fn start_chat(app: &AppHandle, channel: String) {
    *app.state::<Twitch>().channel.lock().unwrap() = Some(channel.clone());
    let handle = app.clone();
    replace_task(&app.state::<Twitch>().chat_task, Some(run_chat(handle, channel)));
}

/// Reconnects to the last channel without signing in.
pub fn resume(app: &AppHandle) {
    let channel = settings::get(app).channel;
    if !channel.is_empty() { start_chat(app, channel); }
}

// ── Commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn chat_status(state: State<Twitch>) -> ChatStatus {
    state.chat_status.lock().unwrap().clone()
}

#[tauri::command]
pub fn chat_connect(app: AppHandle, channel: String) -> Result<String, String> {
    let channel = normalize_channel(&channel).ok_or(
        "Введите ник канала — латиница, цифры и «_», как в ссылке twitch.tv/ник.".to_owned(),
    )?;
    settings::modify(&app, |s| { s.channel = channel.clone(); s.demo_stream = false; });
    let _ = app.emit(CHAT_DELETE_EVENT, message::ChatDeletion::All);
    start_chat(&app, channel.clone());
    Ok(channel)
}

#[tauri::command]
pub fn chat_disconnect(app: AppHandle, state: State<Twitch>) {
    settings::modify(&app, |s| s.channel.clear());
    *state.channel.lock().unwrap() = None;
    replace_task(&state.chat_task, NO_TASK);
    set_chat_status(&app, ChatStatus::Disconnected);
}

#[cfg(test)]
mod tests {
    use super::normalize_channel;

    #[test]
    fn normalizes_channel_input() {
        assert_eq!(normalize_channel(" Shroud ").as_deref(), Some("shroud"));
        assert_eq!(normalize_channel("#shroud").as_deref(), Some("shroud"));
        assert_eq!(normalize_channel("https://www.twitch.tv/Shroud/").as_deref(), Some("shroud"));
        assert_eq!(normalize_channel("twitch.tv/shroud?sr=a").as_deref(), Some("shroud"));
        assert_eq!(normalize_channel(""), None);
        assert_eq!(normalize_channel("not a name"), None);
    }
}
