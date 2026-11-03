//! Twitch: the chat itself (anonymous, by channel name) and an optional sign-in that only adds
//! badge images. Each runs as one background task; the UI sends commands and mirrors the
//! `chat-status` / `account-status` events.

mod auth;
mod badges;
mod emotes;
mod irc;
mod message;
mod store;

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::settings;
use badges::BadgeMap;
use store::Credentials;

pub const CHAT_EVENT: &str = "chat-message";
pub const CHAT_DELETE_EVENT: &str = "chat-delete";
const CHAT_STATUS_EVENT: &str = "chat-status";
const ACCOUNT_STATUS_EVENT: &str = "account-status";

const MAX_BACKOFF_SECS: u64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum TwitchError {
    #[error("Нет связи с Twitch. Проверьте подключение к интернету.")]
    Network(#[from] reqwest::Error),
    #[error("Соединение с чатом прервалось: {0}")]
    Socket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("Вход в Twitch больше недействителен.")]
    Unauthorized,
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
        badge_images: bool,
    },
    Reconnecting {
        channel: String,
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Default, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AccountStatus {
    #[default]
    SignedOut,
    Checking,
    AwaitingUser {
        user_code: String,
        verification_uri: String,
    },
    SignedIn {
        login: String,
    },
    Error {
        message: String,
    },
}

#[derive(Default)]
pub struct Twitch {
    chat_status: Mutex<ChatStatus>,
    account_status: Mutex<AccountStatus>,
    channel: Mutex<Option<String>>,
    chat_task: Mutex<Option<JoinHandle<()>>>,
    account_task: Mutex<Option<JoinHandle<()>>>,
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

fn set_account_status(app: &AppHandle, status: AccountStatus) {
    *app.state::<Twitch>().account_status.lock().unwrap() = status.clone();
    let _ = app.emit(ACCOUNT_STATUS_EVENT, status);
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

/// Reconnects the current channel so it picks up (or drops) badge images after sign-in/out.
fn restart_chat(app: &AppHandle) {
    let channel = app.state::<Twitch>().channel.lock().unwrap().clone();
    if let Some(channel) = channel {
        start_chat(app, channel);
    }
}

// ── Optional sign-in ────────────────────────────────────────────────────

/// Stored credentials with a working access token (refreshed if needed), plus the login.
/// Updates the account status as a side effect.
async fn fresh_credentials(app: &AppHandle, http: &Client) -> Option<(Credentials, String)> {
    let mut credentials = store::load()?;
    let info = match auth::validate(http, &credentials.access_token).await {
        Ok(info) => info,
        Err(TwitchError::Unauthorized) => match auth::refresh(http, &mut credentials).await {
            Ok(()) => {
                if let Err(err) = store::save(&credentials) {
                    eprintln!("OverChat: {err}");
                }
                auth::validate(http, &credentials.access_token).await.ok()?
            }
            Err(TwitchError::Unauthorized) => {
                store::delete();
                set_account_status(
                    app,
                    AccountStatus::Error {
                        message: "Вход в Twitch истёк или был отозван. Значки снова показываются буквами — войдите заново, чтобы вернуть картинки.".into(),
                    },
                );
                return None;
            }
            Err(err) => {
                set_account_status(app, AccountStatus::Error { message: err.to_string() });
                return None;
            }
        },
        Err(err) => {
            set_account_status(app, AccountStatus::Error { message: err.to_string() });
            return None;
        }
    };
    set_account_status(app, AccountStatus::SignedIn { login: info.login.clone() });
    Some((credentials, info.login))
}

/// Badge images for the channel if the user is signed in; `None` means letter badges.
pub(crate) async fn load_badges(app: &AppHandle, http: &Client, room_id: &str) -> Option<BadgeMap> {
    let (credentials, _) = fresh_credentials(app, http).await?;
    badges::fetch(http, &credentials, room_id).await
}

async fn login_flow(app: AppHandle, client_id: String) {
    let http = http_client();
    let device = match auth::start_device_flow(&http, &client_id).await {
        Ok(device) => device,
        Err(err) => return set_account_status(&app, AccountStatus::Error { message: err.to_string() }),
    };

    set_account_status(
        &app,
        AccountStatus::AwaitingUser {
            user_code: device.user_code.clone(),
            verification_uri: device.verification_uri.clone(),
        },
    );
    let _ = app.opener().open_url(&device.verification_uri, None::<&str>);

    let credentials = match auth::wait_for_token(&http, &client_id, &device).await {
        Ok(credentials) => credentials,
        Err(err) => return set_account_status(&app, AccountStatus::Error { message: err.to_string() }),
    };
    if let Err(message) = store::save(&credentials) {
        return set_account_status(&app, AccountStatus::Error { message });
    }
    if fresh_credentials(&app, &http).await.is_some() {
        restart_chat(&app);
    }
}

/// Called once at startup: reconnects to the last channel and checks a previous sign-in.
pub fn resume(app: &AppHandle) {
    let channel = settings::get(app).channel;
    if !channel.is_empty() {
        start_chat(app, channel);
    }
    if store::load().is_none() {
        return;
    }
    set_account_status(app, AccountStatus::Checking);
    let handle = app.clone();
    replace_task(
        &app.state::<Twitch>().account_task,
        Some(async move {
            fresh_credentials(&handle, &http_client()).await;
        }),
    );
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
    settings::modify(&app, |s| s.channel = channel.clone());
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

#[tauri::command]
pub fn account_status(state: State<Twitch>) -> AccountStatus {
    state.account_status.lock().unwrap().clone()
}

/// Client ID baked in at build time (`TWITCH_CLIENT_ID=... npm run app:build`), if any.
#[tauri::command]
pub fn account_builtin_client_id() -> Option<&'static str> {
    option_env!("TWITCH_CLIENT_ID")
}

#[tauri::command]
pub fn account_login(app: AppHandle, client_id: String) -> Result<(), String> {
    let client_id = client_id.trim().to_owned();
    if client_id.is_empty() {
        return Err("Укажите Client ID приложения Twitch.".into());
    }
    settings::modify(&app, |s| s.client_id = client_id.clone());
    set_account_status(&app, AccountStatus::Checking);
    let handle = app.clone();
    replace_task(&app.state::<Twitch>().account_task, Some(login_flow(handle, client_id)));
    Ok(())
}

#[tauri::command]
pub fn account_cancel(app: AppHandle, state: State<Twitch>) {
    replace_task(&state.account_task, NO_TASK);
    set_account_status(&app, AccountStatus::SignedOut);
}

#[tauri::command]
pub async fn account_logout(app: AppHandle) -> Result<(), String> {
    replace_task(&app.state::<Twitch>().account_task, NO_TASK);
    if let Some(credentials) = store::load() {
        auth::revoke(&http_client(), &credentials).await;
    }
    store::delete();
    set_account_status(&app, AccountStatus::SignedOut);
    restart_chat(&app);
    Ok(())
}

#[tauri::command]
pub fn account_open_verification(app: AppHandle, state: State<Twitch>) -> Result<(), String> {
    let status = state.account_status.lock().unwrap().clone();
    if let AccountStatus::AwaitingUser { verification_uri, .. } = status {
        app.opener()
            .open_url(verification_uri, None::<&str>)
            .map_err(|e| format!("Не удалось открыть браузер: {e}"))?;
    }
    Ok(())
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
