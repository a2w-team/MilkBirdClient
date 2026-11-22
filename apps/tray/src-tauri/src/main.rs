mod bridge;

use std::sync::Mutex;
#[cfg(not(target_os = "macos"))]
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
#[cfg(target_os = "macos")]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::webview::WebviewWindowBuilder;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow, WindowEvent,
    tray::TrayIconBuilder,
};

const TRAY_ID: &str = "a2w";

#[derive(Default)]
struct ResultState(Mutex<Option<bridge::LookupResult>>);

#[tauri::command]
fn current_result(state: State<'_, ResultState>) -> Option<bridge::LookupResult> {
    state.0.lock().expect("result state").clone()
}

fn place_under_tray(app: &AppHandle, window: &WebviewWindow) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let Some(anchor) = tray.rect()? else {
        return Ok(());
    };
    let scale = window.scale_factor()?;
    let position = anchor.position.to_physical::<f64>(scale);
    let size = anchor.size.to_physical::<f64>(scale);
    let center_x = position.x + size.width / 2.0;
    let Some(monitor) = window.monitor_from_point(center_x, position.y + size.height / 2.0)? else {
        return Ok(());
    };

    let monitor_scale = monitor.scale_factor();
    let panel = window
        .outer_size()?
        .to_logical::<f64>(scale)
        .to_physical::<f64>(monitor_scale);
    let work_area = monitor.work_area();
    let left = f64::from(work_area.position.x);
    let top = f64::from(work_area.position.y);
    let max_x = (left + f64::from(work_area.size.width) - panel.width).max(left);
    let max_y = (top + f64::from(work_area.size.height) - panel.height).max(top);
    let x = (center_x - panel.width / 2.0).clamp(left, max_x);
    let y = (position.y + size.height + 4.0 * monitor_scale).clamp(top, max_y);

    window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
}

fn show_result(app: &AppHandle) {
    let window: tauri::Result<_> = app.get_webview_window("main").map_or_else(
        || {
            let window =
                WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?.build()?;
            if let Err(err) = place_under_tray(app, &window) {
                eprintln!("unable to position the result window: {err}");
            }
            Ok(window)
        },
        Ok,
    );
    match window {
        Ok(window) => {
            let _ = window.show();
            let _ = window.set_focus();
        }
        Err(err) => eprintln!("unable to open the result window: {err}"),
    }
}

fn main() {
    tauri::Builder::default()
        .manage(ResultState::default())
        .invoke_handler(tauri::generate_handler![current_result])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Regular);
            let show = MenuItem::with_id(app, "show", "查看公告结果", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出奶鸟俱乐部", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let tray = TrayIconBuilder::with_id(TRAY_ID)
                .title("a2w")
                .tooltip("奶鸟俱乐部")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_result(app),
                    "quit" => app.exit(0),
                    _ => {}
                });
            #[cfg(not(target_os = "macos"))]
            let tray = tray.icon(Image::from_bytes(include_bytes!("../icons/icon.png"))?);
            #[cfg(target_os = "macos")]
            let tray = tray
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main")
                            && window.is_visible().unwrap_or(false)
                        {
                            let _ = window.hide();
                        } else {
                            show_result(app);
                        }
                    }
                });
            tray.build(app)?;
            let handle = app.handle().clone();
            bridge::start(move |result| {
                let handle = handle.clone();
                let _ = handle.clone().run_on_main_thread(move || {
                    *handle
                        .state::<ResultState>()
                        .0
                        .lock()
                        .expect("result state") = Some(result);
                    show_result(&handle);
                    let _ = handle.emit_to("main", "lookup-changed", ());
                });
            })?;
            if !std::env::args().any(|arg| arg == "--background") {
                show_result(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("unable to start MilkBird Tray")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            if matches!(_event, tauri::RunEvent::Reopen { .. }) {
                show_result(_app);
            }
        });
}
