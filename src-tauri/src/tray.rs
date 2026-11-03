//! Tray icon: lets the app live without the settings window, and gives quick toggles in-game.

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::{overlay, settings, SETTINGS_WINDOW};

const TOOLTIP: &str = "OverChat — чат Twitch поверх игры";
const TOOLTIP_FULLSCREEN: &str =
    "OverChat: игра в эксклюзивном полноэкранном режиме — чат не виден. Включите в игре режим «Без рамки».";

/// Check items whose state can also change elsewhere (hotkeys, settings window).
pub struct Tray {
    icon: TrayIcon,
    overlay_visible: CheckMenuItem<Wry>,
    click_through: CheckMenuItem<Wry>,
}

pub fn show_settings_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let open_settings = MenuItem::with_id(app, "settings", "Настройки", true, None::<&str>)?;
    let overlay_visible = CheckMenuItem::with_id(
        app,
        "overlay",
        "Показывать чат",
        true,
        overlay::is_visible(app),
        None::<&str>,
    )?;
    let click_through = CheckMenuItem::with_id(
        app,
        "click-through",
        "Сквозные клики",
        true,
        settings::get(app).click_through,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_settings,
            &PredefinedMenuItem::separator(app)?,
            &overlay_visible,
            &click_through,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => show_settings_window(app),
            "overlay" => overlay::toggle_visible(app),
            "click-through" => overlay::toggle_click_through(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_settings_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let icon = builder.build(app)?;

    app.manage(Tray { icon, overlay_visible, click_through });
    Ok(())
}

/// Brings the menu check marks in line with the real state (a menu click also flips them itself).
pub fn sync(app: &AppHandle) {
    if let Some(tray) = app.try_state::<Tray>() {
        let _ = tray.overlay_visible.set_checked(overlay::is_visible(app));
        let _ = tray.click_through.set_checked(settings::get(app).click_through);
    }
}

pub fn set_fullscreen_warning(app: &AppHandle, exclusive_fullscreen: bool) {
    if let Some(tray) = app.try_state::<Tray>() {
        let _ = tray
            .icon
            .set_tooltip(Some(if exclusive_fullscreen { TOOLTIP_FULLSCREEN } else { TOOLTIP }));
    }
}
