mod api;
mod bridge;
mod hub;
mod outbox;
mod profile_cache;
mod session;

use api::{Api, ApiResult, Failure};
use hub::{Bootstrap, Hub, Update, View};
use outbox::Outbox;
use session::{Member, SessionStore};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
#[cfg(not(target_os = "macos"))]
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
#[cfg(target_os = "macos")]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::webview::WebviewWindowBuilder;
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, State, WebviewWindow, WindowEvent,
    tray::TrayIconBuilder,
};

const TRAY_ID: &str = "a2w";
const DEFAULT_EMAIL_DOMAINS: &str = "hkust-gz.edu.cn,connect.hkust-gz.edu.cn";

struct TraySettings {
    title: String,
    window_offset_x: Option<i32>,
    path: PathBuf,
    do_not_disturb: Mutex<bool>,
    email_domains: Vec<String>,
}

impl TraySettings {
    fn from_env(dir: &Path) -> std::io::Result<Self> {
        let path = dir.join("do-not-disturb.json");
        let do_not_disturb = session::read_private_json(&path)?.unwrap_or(false);
        Ok(Self {
            title: std::env::var("MILKBIRD_TRAY_TITLE").unwrap_or_else(|_| TRAY_ID.into()),
            window_offset_x: std::env::var("MILKBIRD_WINDOW_OFFSET_X")
                .ok()
                .and_then(|offset| offset.trim().parse().ok()),
            path,
            do_not_disturb: Mutex::new(do_not_disturb),
            email_domains: login_domains(
                &std::env::var("MILKBIRD_ALLOWED_EMAIL_DOMAINS").unwrap_or_default(),
            ),
        })
    }

    fn is_quiet(&self) -> bool {
        *self.do_not_disturb.lock().expect("tray notification setting")
    }

    fn set_do_not_disturb(&self, enabled: bool) -> std::io::Result<()> {
        let mut setting = self.do_not_disturb.lock().expect("tray notification setting");
        session::write_private_json(&self.path, &enabled)?;
        *setting = enabled;
        Ok(())
    }
}

fn login_domains(config: &str) -> Vec<String> {
    let mut domains = Vec::new();
    for domain in config.split(',') {
        let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
        if domain.len() <= 253
            && domain.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
            && !domains.contains(&domain)
        {
            domains.push(domain);
        }
    }
    if domains.is_empty() {
        domains.extend(DEFAULT_EMAIL_DOMAINS.split(',').map(str::to_owned));
    }
    domains
}

async fn blocking<T: Send + 'static>(
    state: State<'_, Arc<Hub>>,
    work: impl FnOnce(&Hub) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    let hub = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || work(&hub))
        .await
        .map_err(|_| Failure::new("internal_error", "托盘暂时无法处理此请求。"))?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TrayBootstrap {
    #[serde(flatten)]
    hub: Bootstrap,
    do_not_disturb: bool,
}

#[tauri::command]
fn get_bootstrap(state: State<'_, Arc<Hub>>, settings: State<'_, TraySettings>) -> TrayBootstrap {
    TrayBootstrap {
        hub: state.bootstrap(),
        do_not_disturb: settings.is_quiet(),
    }
}

#[tauri::command]
fn set_do_not_disturb(
    app: AppHandle,
    settings: State<'_, TraySettings>,
    enabled: bool,
) -> ApiResult<()> {
    settings
        .set_do_not_disturb(enabled)
        .map_err(|_| Failure::new("notification_settings_io", "无法保存本机免打扰设置。"))?;
    if enabled && let Some(toast) = app.get_webview_window("toast") {
        let _ = toast.hide();
    }
    Ok(())
}

#[tauri::command]
fn get_login_domains(settings: State<'_, TraySettings>) -> Vec<String> {
    settings.email_domains.clone()
}

#[tauri::command]
async fn request_email_code(state: State<'_, Arc<Hub>>, email: String) -> ApiResult {
    blocking(state, move |hub| hub.request_code(&email)).await
}

#[tauri::command]
async fn verify_email_code(
    state: State<'_, Arc<Hub>>,
    email: String,
    code: String,
    display_name: Option<String>,
) -> ApiResult<Member> {
    blocking(state, move |hub| {
        hub.verify_code(&email, &code, display_name.as_deref())
    })
    .await
}

#[tauri::command]
async fn logout(state: State<'_, Arc<Hub>>) -> ApiResult {
    blocking(state, Hub::logout).await
}

#[tauri::command]
async fn list_demands(state: State<'_, Arc<Hub>>, status: Option<String>) -> ApiResult {
    blocking(state, move |hub| {
        hub.with_api(|api, token| api.list_demands(token, status.as_deref()))
    })
    .await
}

#[tauri::command]
async fn complete_demand(state: State<'_, Arc<Hub>>, demand_id: String) -> ApiResult {
    blocking(state, move |hub| hub.complete_demand(&demand_id)).await
}

#[tauri::command]
async fn cancel_demand(state: State<'_, Arc<Hub>>, demand_id: String) -> ApiResult {
    blocking(state, move |hub| hub.cancel_demand(&demand_id)).await
}

#[tauri::command]
async fn list_submissions(state: State<'_, Arc<Hub>>) -> ApiResult {
    blocking(state, Hub::list_submissions).await
}

#[tauri::command]
async fn withdraw_submission(state: State<'_, Arc<Hub>>, idempotency_key: String) -> ApiResult {
    blocking(state, move |hub| hub.withdraw_submission(&idempotency_key)).await
}

#[tauri::command]
async fn list_demand_revisions(state: State<'_, Arc<Hub>>, demand_id: String) -> ApiResult {
    blocking(state, move |hub| {
        hub.with_api(|api, token| api.list_demand_revisions(token, &demand_id))
    })
    .await
}

#[tauri::command]
async fn list_demand_responses(state: State<'_, Arc<Hub>>, demand_id: String) -> ApiResult {
    blocking(state, move |hub| {
        hub.with_api(|api, token| api.list_demand_responses(token, &demand_id))
    })
    .await
}

#[tauri::command]
async fn feedback_response(
    state: State<'_, Arc<Hub>>,
    response_id: String,
    feedback: String,
    expected_updated_at: Option<String>,
) -> ApiResult {
    blocking(state, move |hub| {
        hub.feedback_response(&response_id, &feedback, expected_updated_at.as_deref())
    })
    .await
}

#[tauri::command]
async fn list_matches(state: State<'_, Arc<Hub>>) -> ApiResult {
    blocking(state, |hub| hub.with_api(Api::list_matches)).await
}

#[tauri::command]
async fn respond_match(
    state: State<'_, Arc<Hub>>,
    match_id: String,
    decision: String,
) -> ApiResult {
    blocking(state, move |hub| hub.respond_match(&match_id, &decision)).await
}

#[tauri::command]
async fn list_rooms(state: State<'_, Arc<Hub>>) -> ApiResult {
    blocking(state, |hub| hub.with_api(Api::list_rooms)).await
}

#[tauri::command]
async fn open_room(
    state: State<'_, Arc<Hub>>,
    room_id: String,
    demand_id: Option<String>,
) -> ApiResult {
    blocking(state, move |hub| hub.open_room(room_id, demand_id)).await
}

#[tauri::command]
async fn list_room_messages(state: State<'_, Arc<Hub>>, room_id: String, after: u64) -> ApiResult {
    blocking(state, move |hub| {
        hub.with_api(|api, token| api.list_room_messages(token, &room_id, after))
    })
    .await
}

#[tauri::command]
async fn send_room_message(state: State<'_, Arc<Hub>>, room_id: String, text: String) -> ApiResult {
    blocking(state, move |hub| {
        hub.with_api(|api, token| api.send_room_message(token, &room_id, &text))
    })
    .await
}

#[tauri::command]
async fn vote_room_retention(state: State<'_, Arc<Hub>>, room_id: String, keep: bool) -> ApiResult {
    blocking(state, move |hub| hub.vote_room_retention(&room_id, keep)).await
}

#[tauri::command]
async fn leave_room(state: State<'_, Arc<Hub>>, room_id: String) -> ApiResult {
    blocking(state, move |hub| {
        hub.with_api(|api, token| api.leave_room(token, &room_id))
    })
    .await
}

#[tauri::command]
async fn get_profile(state: State<'_, Arc<Hub>>) -> ApiResult {
    blocking(state, Hub::get_profile).await
}

#[tauri::command]
async fn set_modeling_consent(
    state: State<'_, Arc<Hub>>,
    confirmed: bool,
    source: String,
    enabled: bool,
    base_revision: u64,
) -> ApiResult {
    if !confirmed {
        return Err(Failure::confirmation_required());
    }
    blocking(state, move |hub| {
        hub.set_modeling_consent(&serde_json::json!({
            "source": source, "enabled": enabled, "baseRevision": base_revision,
        }))
    })
    .await
}

#[tauri::command]
async fn set_inference_control(
    state: State<'_, Arc<Hub>>,
    confirmed: bool,
    tag: String,
    action: String,
    base_revision: u64,
) -> ApiResult {
    if !confirmed {
        return Err(Failure::confirmation_required());
    }
    blocking(state, move |hub| {
        hub.set_inference_control(&serde_json::json!({
            "tag": tag, "action": action, "baseRevision": base_revision,
        }))
    })
    .await
}

#[tauri::command]
async fn profile_sync_status(state: State<'_, Arc<Hub>>) -> ApiResult {
    blocking(state, Hub::profile_sync_status).await
}

#[tauri::command]
async fn discard_profile_draft(state: State<'_, Arc<Hub>>) -> ApiResult {
    blocking(state, Hub::discard_profile_draft).await
}

#[tauri::command]
async fn discard_failed_profile_edit(state: State<'_, Arc<Hub>>, mutation_id: String) -> ApiResult {
    blocking(state, move |hub| hub.discard_failed_profile_edit(&mutation_id)).await
}

#[tauri::command]
async fn mark_demand_read(state: State<'_, Arc<Hub>>, demand_id: String) -> ApiResult<()> {
    blocking(state, move |hub| {
        hub.mark_demand_read(&demand_id);
        Ok(())
    })
    .await
}

#[tauri::command]
async fn mark_room_read(
    state: State<'_, Arc<Hub>>,
    room_id: String,
    demand_id: Option<String>,
) -> ApiResult<()> {
    blocking(state, move |hub| {
        hub.mark_room_read(&room_id, demand_id.as_deref());
        Ok(())
    })
    .await
}

#[tauri::command]
async fn clear_unread(state: State<'_, Arc<Hub>>) -> ApiResult<()> {
    blocking(state, |hub| {
        hub.clear_unread();
        Ok(())
    })
    .await
}

#[tauri::command]
async fn open_notification(state: State<'_, Arc<Hub>>, notice_id: u64) -> ApiResult<()> {
    blocking(state, move |hub| {
        hub.open_notification(notice_id);
        Ok(())
    })
    .await
}

#[tauri::command]
fn remember_view(
    state: State<'_, Arc<Hub>>,
    navigation_id: u64,
    view: View,
    demand_id: Option<String>,
    room_id: Option<String>,
) {
    state.remember_view(navigation_id, view, demand_id, room_id);
}

fn main_window_x(
    anchor_x: Option<f64>,
    offset: Option<i32>,
    min_x: f64,
    max_x: f64,
) -> Option<f64> {
    let base_x = anchor_x.or_else(|| offset.map(|_| max_x))?;
    Some((base_x + f64::from(offset.unwrap_or_default())).clamp(min_x, max_x))
}

fn toast_window_pos(
    panel_height: f64,
    stack_above: bool,
    left: f64,
    top: f64,
    max_x: f64,
    max_y: f64,
) -> (f64, f64) {
    let margin = 16.0;
    let lift = if stack_above {
        panel_height + 12.0
    } else {
        0.0
    };
    let x = (max_x - margin).clamp(left, max_x);
    let y = (max_y - margin - lift).clamp(top, max_y);
    (x, y)
}

fn find_monitor(
    window: &WebviewWindow,
    anchor: Option<(f64, f64)>,
) -> tauri::Result<Option<tauri::Monitor>> {
    if let Some((x, y)) = anchor {
        if let Some(monitor) = window.monitor_from_point(x, y)? {
            return Ok(Some(monitor));
        }
        for monitor in window.available_monitors()? {
            let s = monitor.scale_factor();
            let pos = monitor.position().to_logical::<f64>(s);
            let size = monitor.size().to_logical::<f64>(s);
            if x >= pos.x && x < pos.x + size.width && y >= pos.y && y < pos.y + size.height {
                return Ok(Some(monitor));
            }
        }
    }
    if let Some(monitor) = window.current_monitor()? {
        return Ok(Some(monitor));
    }
    window.primary_monitor()
}

fn place_window(app: &AppHandle, window: &WebviewWindow) -> tauri::Result<()> {
    let settings = app.state::<TraySettings>();
    let is_toast = window.label() == "toast";
    let offset = if is_toast {
        None
    } else {
        settings.window_offset_x
    };
    let scale = window.scale_factor()?;
    let anchor = match app.tray_by_id(TRAY_ID) {
        Some(tray) => tray.rect()?,
        None => None,
    }
    .map(|rect| {
        let pos = rect.position.to_logical::<f64>(scale);
        let size = rect.size.to_logical::<f64>(scale);
        (pos.x + size.width / 2.0, pos.y + size.height)
    });

    let Some(monitor) = find_monitor(window, anchor)? else {
        return Ok(());
    };

    let monitor_scale = monitor.scale_factor();
    let panel = window.outer_size()?.to_logical::<f64>(scale);
    let work_area = monitor.work_area();
    let left = f64::from(work_area.position.x) / monitor_scale;
    let top = f64::from(work_area.position.y) / monitor_scale;
    let width = f64::from(work_area.size.width) / monitor_scale;
    let height = f64::from(work_area.size.height) / monitor_scale;
    let max_x = (left + width - panel.width).max(left);
    let max_y = (top + height - panel.height).max(top);

    let (x, y) = if is_toast {
        let stack_above = settings.window_offset_x.unwrap_or_default() != 0;
        toast_window_pos(panel.height, stack_above, left, top, max_x, max_y)
    } else {
        let anchor_x = anchor.map(|(x, _)| x - panel.width / 2.0);
        let Some(x) = main_window_x(anchor_x, offset, left, max_x) else {
            return Ok(());
        };
        let y = (anchor.map_or(top, |(_, y)| y).max(top) + 8.0).clamp(top, max_y);
        (x, y)
    };
    window.set_position(LogicalPosition::new(x.round(), y.round()))
}

fn show_window(app: &AppHandle, label: &str, focus: bool) {
    let result = (|| -> tauri::Result<()> {
        let window = if let Some(window) = app.get_webview_window(label) {
            window
        } else {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|config| config.label == label)
                .expect("window is configured");
            WebviewWindowBuilder::from_config(app, config)?.build()?
        };
        if label == "toast" || !window.is_visible()? {
            if let Err(error) = place_window(app, &window) {
                eprintln!("unable to position {label}: {error}");
            }
        }
        if !window.is_visible()? {
            window.show()?;
        }
        if focus {
            window.set_focus()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("unable to open {label}: {error}");
    }
}

fn show_main(app: &AppHandle) {
    if let Some(toast) = app.get_webview_window("toast") {
        let _ = toast.hide();
    }
    show_window(app, "main", true);
}

fn apply_update(app: &AppHandle, update: Update) {
    let state = app.state::<Arc<Hub>>().bootstrap();
    let settings = app.state::<TraySettings>();
    let quiet = settings.is_quiet();
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let title = &settings.title;
        let title = if state.unread_count == 0 {
            title.clone()
        } else {
            format!("{title} · {}", state.unread_count)
        };
        let _ = tray.set_title(Some(title));
    }
    let main_visible = app
        .get_webview_window("main")
        .is_some_and(|window| window.is_visible().unwrap_or(false));
    if update.show_main {
        show_main(app);
    } else if quiet || main_visible || state.notice.is_none() {
        if let Some(toast) = app.get_webview_window("toast") {
            let _ = toast.hide();
        }
    } else if update.new_notice {
        show_window(app, "toast", false);
    }
    let _ = app.emit_to("main", "hub-event", &update);
    if update.new_notice && !quiet {
        let _ = app.emit_to("toast", "hub-event", &update);
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            set_do_not_disturb,
            get_login_domains,
            request_email_code,
            verify_email_code,
            logout,
            list_demands,
            complete_demand,
            cancel_demand,
            list_submissions,
            withdraw_submission,
            list_demand_revisions,
            list_demand_responses,
            feedback_response,
            list_matches,
            respond_match,
            list_rooms,
            open_room,
            list_room_messages,
            send_room_message,
            vote_room_retention,
            leave_room,
            get_profile,
            set_modeling_consent,
            set_inference_control,
            profile_sync_status,
            discard_profile_draft,
            discard_failed_profile_edit,
            mark_demand_read,
            mark_room_read,
            clear_unread,
            open_notification,
            remember_view
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Regular);
            let dir = session::directory()?;
            let bridge = bridge::Bridge::bind(&dir)?;
            let settings = TraySettings::from_env(&dir)?;
            let show = MenuItem::with_id(app, "show", "打开奶鸟俱乐部", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出奶鸟俱乐部", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let tray = TrayIconBuilder::with_id(TRAY_ID)
                .title(&settings.title)
                .tooltip("奶鸟俱乐部")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_main(app),
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
                            show_main(app);
                        }
                    }
                });
            app.manage(settings);
            tray.build(app)?;

            let handle = app.handle().clone();
            let hub = Arc::new(Hub::new(
                Api::from_env()?,
                SessionStore::open(&dir)?,
                Outbox::open(&dir)?,
                move |update| {
                    let handle = handle.clone();
                    let _ = handle
                        .clone()
                        .run_on_main_thread(move || apply_update(&handle, update));
                },
            ));
            app.manage(Arc::clone(&hub));
            apply_update(app.handle(), Update::default());
            bridge.start(Arc::clone(&hub))?;
            hub.start_events()?;

            if !std::env::args().any(|arg| arg == "--background") {
                show_main(app.handle());
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
                show_main(_app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{TraySettings, login_domains, main_window_x, toast_window_pos};
    use crate::session::tests::TestDir;

    #[test]
    fn quiet_time_is_local_to_the_device_and_survives_reopen() {
        let dir = TestDir::new();
        let settings = TraySettings::from_env(dir.path()).unwrap();
        assert!(!settings.is_quiet());
        settings.set_do_not_disturb(true).unwrap();
        let reopened = TraySettings::from_env(dir.path()).unwrap();
        assert!(reopened.is_quiet());
        reopened.set_do_not_disturb(false).unwrap();
        assert!(!TraySettings::from_env(dir.path()).unwrap().is_quiet());
    }

    #[test]
    fn login_domains_normalize_validate_and_preserve_order() {
        assert_eq!(
            login_domains(" .EXAMPLE.EDU , campus.example.edu, example.edu, -, foo..edu, @foo.edu"),
            ["example.edu", "campus.example.edu"]
        );
        assert_eq!(
            login_domains(" , https://example.edu"),
            ["hkust-gz.edu.cn", "connect.hkust-gz.edu.cn"]
        );
    }

    #[test]
    fn main_window_offset_uses_work_area() {
        let position = |anchor, offset| main_window_x(anchor, offset, 50.0, 1000.0);
        assert_eq!(position(Some(800.0), None), Some(800.0));
        assert_eq!(position(Some(800.0), Some(-300)), Some(500.0));
        assert_eq!(position(Some(800.0), Some(300)), Some(1000.0));
        assert_eq!(position(None, None), None);
        assert_eq!(position(None, Some(0)), Some(1000.0));
        assert_eq!(position(None, Some(-600)), Some(400.0));
        assert_eq!(position(None, Some(-1200)), Some(50.0));
    }

    #[test]
    fn toast_window_places_bottom_right_and_stacks() {
        let (x0, y0) = toast_window_pos(100.0, false, 0.0, 25.0, 1168.0, 882.0);
        assert_eq!((x0, y0), (1152.0, 866.0));
        let (x1, y1) = toast_window_pos(100.0, true, 0.0, 25.0, 1168.0, 882.0);
        assert_eq!((x1, y1), (1152.0, 754.0));
    }
}
