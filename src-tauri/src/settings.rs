//! User settings: one JSON file in %APPDATA%\app.overchat.desktop, owned by Rust.
//! Both windows read them with `get_settings` and follow the `settings-changed` event.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

pub const CHANGED_EVENT: &str = "settings-changed";
const FILE_NAME: &str = "settings.json";

/// Mirrors `Settings` in src/shared/types.ts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    // Appearance
    pub bg_opacity: u8,
    /// Opacity of the messages themselves (text, badges, emotes), 10–100 %.
    pub chat_opacity: u8,
    pub font_family: String,
    pub font_size: u8,
    pub compact: bool,
    pub max_messages: u16,
    /// Hide a message this many seconds after it arrived; 0 keeps messages until they scroll away.
    pub fade_after_secs: u32,
    pub hide_commands: bool,
    /// Off: emotes show their first frame — noticeably less GPU work on fast chats.
    pub animated_emotes: bool,
    pub demo_stream: bool,
    // Behaviour
    pub click_through: bool,
    pub game_mode: bool,
    pub hotkey_click_through: String,
    pub hotkey_toggle_overlay: String,
    /// Closing the settings window hides it to the tray instead of quitting.
    pub close_to_tray: bool,
    // Twitch
    pub channel: String,
    pub client_id: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bg_opacity: 45,
            chat_opacity: 100,
            font_family: "Segoe UI".into(),
            font_size: 14,
            compact: false,
            max_messages: 50,
            fade_after_secs: 0,
            hide_commands: false,
            animated_emotes: true,
            demo_stream: true,
            click_through: false,
            game_mode: false,
            hotkey_click_through: "Ctrl+Shift+X".into(),
            hotkey_toggle_overlay: "Ctrl+Shift+H".into(),
            close_to_tray: true,
            channel: String::new(),
            client_id: String::new(),
        }
    }
}

impl Settings {
    /// Keeps values in the ranges the UI offers, whatever ends up in the file.
    fn clamped(mut self) -> Self {
        self.bg_opacity = self.bg_opacity.min(100);
        // Never fully invisible: otherwise the chat looks broken rather than transparent.
        self.chat_opacity = self.chat_opacity.clamp(10, 100);
        self.font_size = self.font_size.clamp(10, 32);
        self.max_messages = self.max_messages.clamp(5, 200);
        self.fade_after_secs = self.fade_after_secs.min(3600);
        if self.font_family.trim().is_empty() || self.font_family.len() > 64 {
            self.font_family = Settings::default().font_family;
        }
        self
    }
}

pub struct SettingsStore {
    path: Option<PathBuf>,
    current: Mutex<Settings>,
}

/// Loads settings before any window exists: a webview may ask for them before `setup` runs.
/// Same folder Tauri uses for `app_config_dir` on Windows: %APPDATA%\<identifier>.
pub fn init(identifier: &str) -> SettingsStore {
    let path = std::env::var_os("APPDATA")
        .map(|dir| PathBuf::from(dir).join(identifier).join(FILE_NAME));
    let current = path
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
        .unwrap_or_default()
        .clamped();
    SettingsStore { path, current: Mutex::new(current) }
}

pub fn get(app: &AppHandle) -> Settings {
    app.state::<SettingsStore>().current.lock().unwrap().clone()
}

/// Write to a temp file, then rename: a crash mid-write never leaves a half-written file.
fn write_file(path: &PathBuf, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
    fs::rename(tmp, path)
}

pub fn save(app: &AppHandle, settings: Settings) -> Settings {
    let settings = settings.clamped();
    let store = app.state::<SettingsStore>();
    *store.current.lock().unwrap() = settings.clone();
    if let Some(path) = &store.path {
        if let Err(err) = write_file(path, &settings) {
            eprintln!("OverChat: не удалось сохранить настройки: {err}");
        }
    }
    let _ = app.emit(CHANGED_EVENT, &settings);
    settings
}

pub fn modify(app: &AppHandle, change: impl FnOnce(&mut Settings)) -> Settings {
    let mut settings = get(app);
    change(&mut settings);
    save(app, settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"fontSize": 18}"#).unwrap();
        assert_eq!(settings.font_size, 18);
        assert_eq!(settings.hotkey_click_through, "Ctrl+Shift+X");
    }

    #[test]
    fn clamps_out_of_range_values() {
        let settings = Settings { bg_opacity: 250, chat_opacity: 0, font_size: 3, max_messages: 0, font_family: " ".into(), ..Default::default() }
            .clamped();
        assert_eq!((settings.bg_opacity, settings.font_size, settings.max_messages), (100, 10, 5));
        assert_eq!(settings.chat_opacity, 10);
        assert_eq!(settings.font_family, "Segoe UI");
    }
}
