//! The deliberately small, headless-first FNDR desktop host.
//!
//! T-901's first runnable cut proves the important ownership boundary: an
//! explicitly started real capture worker drains before app exit even if no
//! window is open. A small trust/status window makes lifecycle state
//! inspectable before it can request Screen Recording; closing it hides the
//! window while an active worker continues in the menu bar.

use fndr_shell::app::{CaptureLaunchOptions, ShellCaptureState, doctor};
use fndr_shell::search::ShellSearchModel;
use fndr_types::CaptureRuntimeState;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;

const QUIT_MENU_ID: &str = "quit";
const SHOW_WINDOW_MENU_ID: &str = "show-window";
const TOGGLE_PAUSE_MENU_ID: &str = "toggle-pause";
const MAIN_WINDOW_LABEL: &str = "main";
static EXITING: AtomicBool = AtomicBool::new(false);

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_window = MenuItem::with_id(app, SHOW_WINDOW_MENU_ID, "Show FNDR", true, None::<&str>)?;
    let toggle_pause = MenuItem::with_id(
        app,
        TOGGLE_PAUSE_MENU_ID,
        "Pause / Resume Capture",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, QUIT_MENU_ID, "Quit FNDR", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_window, &toggle_pause, &quit])?;
    let icon = app
        .default_window_icon()
        .expect("FNDR requires the bundled application icon")
        .clone();

    TrayIconBuilder::with_id("fndr")
        .icon(icon)
        .menu(&menu)
        .tooltip("FNDR capture host")
        .on_menu_event(|app, event| match event.id().as_ref() {
            SHOW_WINDOW_MENU_ID => {
                if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            TOGGLE_PAUSE_MENU_ID => {
                let state = app.state::<ShellCaptureState>();
                let paused = match state.status().state {
                    CaptureRuntimeState::Running => true,
                    CaptureRuntimeState::Paused => false,
                    _ => return,
                };
                let _ = state.set_paused(paused);
            }
            QUIT_MENU_ID => {
                EXITING.store(true, Ordering::Release);
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn main() {
    let options = CaptureLaunchOptions::parse(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("FNDR launch options: {error}");
        eprintln!(
            "usage: fndr-shell [--data-dir PATH] [--model PATH] [--doctor | --run-seconds N]"
        );
        std::process::exit(2);
    });
    if options.doctor {
        let report = doctor(&options);
        for line in report.lines() {
            println!("{line}");
        }
        std::process::exit(if report.ready_for_permission_rehearsal {
            0
        } else {
            3
        });
    }
    let exit_after = options.run_seconds;
    let options_for_autostart = options.clone();
    let commands = fndr_shell::specta_builder();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(ShellCaptureState::default())
        // Registered independently of capture: search is read-only, so a
        // person can query what FNDR already remembers without starting
        // capture first. The model worker inside stays unspawned until the
        // first search that needs the semantic route.
        .manage(ShellSearchModel::default())
        .manage(options)
        .invoke_handler(commands.invoke_handler())
        .setup(move |app| {
            commands.mount_events(app);
            install_tray(app)?;
            if let Some(seconds) = exit_after {
                app.state::<ShellCaptureState>()
                    .start(app.handle(), options_for_autostart.clone())?;
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(seconds));
                    if let Err(error) = handle.state::<ShellCaptureState>().shutdown() {
                        eprintln!("FNDR capture shutdown: {error}");
                    }
                    EXITING.store(true, Ordering::Release);
                    let exit_handle = handle.clone();
                    if let Err(error) = handle.run_on_main_thread(move || exit_handle.exit(0)) {
                        eprintln!("FNDR scheduled exit failed: {error}");
                    }
                });
            }
            Ok(())
        })
        .build(tauri::tauri_build_context!())
        .unwrap_or_else(|error| {
            eprintln!("FNDR could not start: {error}");
            std::process::exit(1);
        });

    app.run(|app, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == MAIN_WINDOW_LABEL && !EXITING.load(Ordering::Acquire) => {
            api.prevent_close();
            if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                let _ = window.hide();
            }
        }
        tauri::RunEvent::ExitRequested { .. } => {
            EXITING.store(true, Ordering::Release);
            if let Err(error) = app.state::<ShellCaptureState>().shutdown() {
                eprintln!("FNDR capture shutdown: {error}");
            }
        }
        _ => {}
    });
}
