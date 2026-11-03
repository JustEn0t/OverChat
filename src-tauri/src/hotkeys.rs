//! Global hotkeys via `RegisterHotKey` (tauri-plugin-global-shortcut): they work while the game
//! has focus, without keyboard hooks.

use std::str::FromStr;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::overlay;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    ToggleClickThrough,
    ToggleOverlay,
}

#[derive(Default)]
pub struct Hotkeys {
    active: Mutex<Vec<(Shortcut, Action)>>,
    /// Why the saved hotkeys could not be registered at startup, for the settings window.
    pub startup_error: Mutex<Option<String>>,
}

/// "Ctrl+Shift+X" → Shortcut; an empty string means the action has no hotkey.
fn parse(text: &str) -> Result<Option<Shortcut>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    Shortcut::from_str(text)
        .map(Some)
        .map_err(|_| format!("Не удалось разобрать сочетание «{text}»."))
}

/// Registers both hotkeys, or leaves the previous ones untouched and explains what went wrong.
pub fn apply(app: &AppHandle, click_through: &str, toggle_overlay: &str) -> Result<(), String> {
    let wanted: Vec<(Shortcut, Action, &str)> = [
        (parse(click_through)?, Action::ToggleClickThrough, click_through),
        (parse(toggle_overlay)?, Action::ToggleOverlay, toggle_overlay),
    ]
    .into_iter()
    .filter_map(|(shortcut, action, text)| shortcut.map(|s| (s, action, text.trim())))
    .collect();

    if wanted.len() == 2 && wanted[0].0 == wanted[1].0 {
        return Err("Одно сочетание нельзя назначить на два действия.".into());
    }

    let state = app.state::<Hotkeys>();
    let mut active = state.active.lock().unwrap();
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();

    for (shortcut, _, text) in &wanted {
        if shortcuts.register(*shortcut).is_err() {
            let _ = shortcuts.unregister_all();
            for (old, _) in active.iter() {
                let _ = shortcuts.register(*old);
            }
            return Err(format!(
                "Сочетание {text} уже занято другой программой. Выберите другое."
            ));
        }
    }

    *active = wanted.into_iter().map(|(s, action, _)| (s, action)).collect();
    *state.startup_error.lock().unwrap() = None;
    Ok(())
}

/// While the user records a new combination, our hotkeys must not swallow the key press.
pub fn pause(app: &AppHandle) {
    let _ = app.global_shortcut().unregister_all();
}

pub fn resume(app: &AppHandle) {
    let active = app.state::<Hotkeys>().active.lock().unwrap().clone();
    for (shortcut, _) in active {
        // Already registered (e.g. right after `apply`) is fine.
        let _ = app.global_shortcut().register(shortcut);
    }
}

pub fn handle(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let action = app
        .state::<Hotkeys>()
        .active
        .lock()
        .unwrap()
        .iter()
        .find(|(s, _)| s == shortcut)
        .map(|(_, action)| *action);
    match action {
        Some(Action::ToggleClickThrough) => overlay::toggle_click_through(app),
        Some(Action::ToggleOverlay) => overlay::toggle_visible(app),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_the_format_the_ui_records() {
        for text in ["Ctrl+Shift+X", "Alt+F2", "Ctrl+Alt+5", "Ctrl+Shift+Space", "F8"] {
            assert!(matches!(parse(text), Ok(Some(_))), "{text}");
        }
        assert!(matches!(parse(""), Ok(None)));
        assert!(parse("Ctrl+Nope").is_err());
    }
}
