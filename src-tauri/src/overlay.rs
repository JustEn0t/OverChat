//! The chat window: click-through, show/hide and the "borderless game" keep-on-top mode.
//! Everything here only touches our own window — nothing is injected into the game.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::{settings, tray};

pub const OVERLAY: &str = "overlay";
const CLICK_THROUGH_EVENT: &str = "click-through-changed";
const VISIBILITY_EVENT: &str = "overlay-visibility";
const FULLSCREEN_EVENT: &str = "exclusive-fullscreen";
/// How often we restore topmost (game mode) and check for exclusive-fullscreen games.
const WATCH_EVERY: Duration = Duration::from_millis(1500);

static EXCLUSIVE_FULLSCREEN: AtomicBool = AtomicBool::new(false);

fn window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window(OVERLAY).ok_or_else(|| "окно чата не найдено".into())
}

pub fn set_click_through(app: &AppHandle, enabled: bool) -> Result<(), String> {
    // WS_EX_TRANSPARENT under the hood: the window stays visible, mouse input goes to what's below.
    window(app)?
        .set_ignore_cursor_events(enabled)
        .map_err(|e| e.to_string())?;
    settings::modify(app, |s| s.click_through = enabled);
    tray::sync(app);
    app.emit(CLICK_THROUGH_EVENT, enabled).map_err(|e| e.to_string())
}

pub fn toggle_click_through(app: &AppHandle) {
    let enabled = settings::get(app).click_through;
    let _ = set_click_through(app, !enabled);
}

pub fn is_visible(app: &AppHandle) -> bool {
    window(app).and_then(|w| w.is_visible().map_err(|e| e.to_string())).unwrap_or(false)
}

pub fn set_visible(app: &AppHandle, visible: bool) -> Result<(), String> {
    let window = window(app)?;
    if visible {
        // Show without taking focus: the hotkey is pressed in-game and the game must keep it.
        win32::show_without_focus(&window);
        win32::bring_to_top(&window);
    } else {
        window.hide().map_err(|e| e.to_string())?;
    }
    tray::sync(app);
    app.emit(VISIBILITY_EVENT, visible).map_err(|e| e.to_string())
}

pub fn toggle_visible(app: &AppHandle) {
    let _ = set_visible(app, !is_visible(app));
}

/// True while a game runs in exclusive fullscreen, where Windows doesn't show other windows.
pub fn exclusive_fullscreen() -> bool {
    EXCLUSIVE_FULLSCREEN.load(Ordering::Relaxed)
}

/// Background watcher:
/// - game mode: borderless/windowed-fullscreen games often push other topmost windows behind
///   them, so we periodically restore our own window's topmost position;
/// - exclusive fullscreen: the chat can't be shown at all, so we tell the user how to fix it.
pub fn start_watch(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut timer = tokio::time::interval(WATCH_EVERY);
        loop {
            timer.tick().await;

            let fullscreen = win32::exclusive_fullscreen_running();
            if fullscreen != EXCLUSIVE_FULLSCREEN.swap(fullscreen, Ordering::Relaxed) {
                tray::set_fullscreen_warning(&app, fullscreen);
                let _ = app.emit(FULLSCREEN_EVENT, fullscreen);
            }

            if !settings::get(&app).game_mode {
                continue;
            }
            if let Ok(window) = window(&app) {
                if window.is_visible().unwrap_or(false) {
                    win32::bring_to_top(&window);
                }
            }
        }
    });
}

#[cfg(windows)]
mod win32 {
    use tauri::WebviewWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, ShowWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
        SWP_NOSIZE, SW_SHOWNOACTIVATE,
    };

    pub fn bring_to_top(window: &WebviewWindow) {
        if let Ok(hwnd) = window.hwnd() {
            // SAFETY: plain Win32 call on our own, still-alive window handle.
            unsafe {
                SetWindowPos(
                    hwnd.0 as _,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
        }
    }

    pub fn show_without_focus(window: &WebviewWindow) {
        if let Ok(hwnd) = window.hwnd() {
            // SAFETY: as above.
            unsafe {
                ShowWindow(hwnd.0 as _, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// Asks the shell (the same check Windows uses to hold back notifications) whether a
    /// Direct3D game is in exclusive fullscreen. Read-only, doesn't touch the game.
    pub fn exclusive_fullscreen_running() -> bool {
        use windows_sys::Win32::UI::Shell::{SHQueryUserNotificationState, QUNS_RUNNING_D3D_FULL_SCREEN};
        let mut state = 0;
        // SAFETY: writes one enum value into `state`.
        let hr = unsafe { SHQueryUserNotificationState(&mut state) };
        hr == 0 && state == QUNS_RUNNING_D3D_FULL_SCREEN
    }
}

#[cfg(not(windows))]
mod win32 {
    use tauri::WebviewWindow;

    pub fn bring_to_top(window: &WebviewWindow) {
        let _ = window.set_always_on_top(true);
    }

    pub fn show_without_focus(window: &WebviewWindow) {
        let _ = window.show();
    }

    pub fn exclusive_fullscreen_running() -> bool {
        false
    }
}
