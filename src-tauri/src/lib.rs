mod hotkeys;
mod overlay;
mod settings;
mod tray;
mod twitch;

use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_window_state::StateFlags;

use settings::Settings;

pub(crate) const SETTINGS_WINDOW: &str = "main";
/// Passed by the autostart entry: start in the tray without opening the settings window.
const MINIMIZED_ARG: &str = "--minimized";

#[tauri::command]
fn get_settings(app: AppHandle) -> Settings {
    settings::get(&app)
}

/// Saves settings coming from the settings window. Fields Rust manages itself
/// (current channel, click-through) are kept as they are.
#[tauri::command]
fn update_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    let current = settings::get(&app);
    if settings.hotkey_click_through != current.hotkey_click_through
        || settings.hotkey_toggle_overlay != current.hotkey_toggle_overlay
    {
        hotkeys::apply(&app, &settings.hotkey_click_through, &settings.hotkey_toggle_overlay)?;
    }
    Ok(settings::save(
        &app,
        Settings {
            channel: current.channel,
            click_through: current.click_through,
            ..settings
        },
    ))
}

#[tauri::command]
fn set_click_through(app: AppHandle, enabled: bool) -> Result<(), String> {
    overlay::set_click_through(&app, enabled)
}

#[tauri::command]
fn get_overlay_visible(app: AppHandle) -> bool {
    overlay::is_visible(&app)
}

#[tauri::command]
fn set_overlay_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    overlay::set_visible(&app, visible)
}

#[tauri::command]
fn get_exclusive_fullscreen() -> bool {
    overlay::exclusive_fullscreen()
}

#[tauri::command]
fn hotkey_status(state: State<hotkeys::Hotkeys>) -> Option<String> {
    state.startup_error.lock().unwrap().clone()
}

#[tauri::command]
fn pause_hotkeys(app: AppHandle) {
    hotkeys::pause(&app)
}

#[tauri::command]
fn resume_hotkeys(app: AppHandle) {
    hotkeys::resume(&app)
}

/// Autostart lives in the registry (HKCU\…\Run), so Windows itself is the source of truth.
#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    let result = if enabled { autolaunch.enable() } else { autolaunch.disable() };
    result.map_err(|e| format!("Не удалось изменить автозапуск: {e}"))
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

pub fn run() {
    let context = tauri::generate_context!();
    let settings_store = settings::init(&context.config().identifier);
    let start_minimized = std::env::args().any(|arg| arg == MINIMIZED_ARG);

    tauri::Builder::default()
        // Must come first: a second launch just brings the running copy's settings window forward,
        // instead of starting a second overlay that fights over the hotkeys.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_settings_window(app);
        }))
        .manage(settings_store)
        .manage(hotkeys::Hotkeys::default())
        .manage(twitch::Twitch::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![MINIMIZED_ARG]),
        ))
        // Remembers where the user put both windows (position and size only).
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION | StateFlags::SIZE)
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(hotkeys::handle)
                .build(),
        )
        .setup(move |app| {
            let handle = app.handle();
            let saved = settings::get(handle);

            if saved.click_through {
                let _ = overlay::set_click_through(handle, true);
            }
            // A busy hotkey must not stop the app: report it in the settings window instead.
            if let Err(err) = hotkeys::apply(handle, &saved.hotkey_click_through, &saved.hotkey_toggle_overlay) {
                *app.state::<hotkeys::Hotkeys>().startup_error.lock().unwrap() =
                    Some(format!("{err} Пока работают переключатели в окне настроек."));
            }
            tray::create(handle)?;
            // The settings window starts hidden (see tauri.conf.json) so autostart doesn't flash it.
            if !start_minimized {
                tray::show_settings_window(handle);
            }
            overlay::start_watch(handle);
            twitch::resume(handle);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != SETTINGS_WINDOW {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                if settings::get(app).close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                } else {
                    app.exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            update_settings,
            set_click_through,
            get_overlay_visible,
            set_overlay_visible,
            get_exclusive_fullscreen,
            hotkey_status,
            pause_hotkeys,
            resume_hotkeys,
            get_autostart,
            set_autostart,
            quit_app,
            twitch::chat_status,
            twitch::chat_connect,
            twitch::chat_disconnect,
            twitch::account_status,
            twitch::account_builtin_client_id,
            twitch::account_login,
            twitch::account_cancel,
            twitch::account_logout,
            twitch::account_open_verification
        ])
        .run(context)
        .expect("error while running OverChat");
}
