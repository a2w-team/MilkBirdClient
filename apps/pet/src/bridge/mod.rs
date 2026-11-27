//! Tray bridge: smart exponential-backoff `tray.sock` poller, unread/match state diffing,
//! quiet offline companion fallback, and zero-blocking main-thread integration.

pub mod launcher;
pub mod socket;

use crate::bridge::socket::{BridgeError, TraySocketClient, TrayStatusData, default_socket_path};
use crate::bubble::BubbleAction;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Polling cadence controlled by the desktop pet's current activity & power state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollPace {
    /// Active desktop & awake pet: poll lightweight `status` every `4 s`.
    Active,
    /// Idle or sleeping pet: back off to `15 s`.
    Relaxed,
    /// Fullscreen / screen-locked / stealth suspended: pause polling completely (`0% CPU`).
    Paused,
}

/// High-level tray connectivity state surfaced in the native right-click menu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TrayConnection {
    /// Tray process is not running; pet silently operates in standalone companion mode.
    #[default]
    Offline,
    /// Tray is running, waiting for student campus-email login.
    LoggedOut,
    /// Tray is logged in and syncing normally.
    LoggedIn {
        member_label: String,
        active_demands: u32,
        unread_count: u32,
    },
}

impl TrayConnection {
    /// Concise Chinese status label shown at the top of the native right-click menu.
    pub fn menu_summary(&self) -> String {
        match self {
            Self::Offline => "离线伴侣模式（未检测到托盘）".to_string(),
            Self::LoggedOut => "奶鸟托盘：未登录（去托盘登录校园邮箱）".to_string(),
            Self::LoggedIn {
                member_label,
                active_demands,
                unread_count,
            } => {
                if *unread_count > 0 {
                    format!(
                        "奶鸟托盘：{member_label} · {active_demands} 活跃需求 / {unread_count} 未读"
                    )
                } else if *active_demands > 0 {
                    format!("奶鸟托盘：{member_label} · 盯着 {active_demands} 条活跃需求")
                } else {
                    format!("奶鸟托盘：{member_label} · 已连线")
                }
            }
        }
    }
}

/// Notification hint computed when diffing consecutive [`TrayStatusData`] snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayNotification {
    pub text: String,
    pub action: BubbleAction,
}

/// Event emitted from the background bridge thread to the pet UI session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayBridgeUpdate {
    pub connection: TrayConnection,
    pub unread_count: u32,
    pub active_demands: u32,
    pub urgent_pulse: bool,
    pub notification: Option<TrayNotification>,
    /// Primary room to jump to if user clicks MilkBird while an unread badge is showing.
    pub preferred_action: Option<BubbleAction>,
}

/// Stateful differ comparing consecutive `status` snapshots without noisy repeat alerts.
#[derive(Debug, Default)]
pub struct TrayStateDiffer {
    last_status: Option<TrayStatusData>,
    was_online: bool,
    announced_logged_out: bool,
}

impl TrayStateDiffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a successful `status` + optional `active_demands` poll.
    pub fn on_status(
        &mut self,
        status: TrayStatusData,
        active_demands: u32,
    ) -> TrayBridgeUpdate {
        self.was_online = true;

        if !status.logged_in {
            self.last_status = Some(status);
            let notification = if !self.announced_logged_out {
                self.announced_logged_out = true;
                Some(TrayNotification {
                    text: "去托盘登录校园邮箱，我就能帮你盯搭子消息啦～".to_string(),
                    action: BubbleAction::OpenTray,
                })
            } else {
                None
            };
            return TrayBridgeUpdate {
                connection: TrayConnection::LoggedOut,
                unread_count: 0,
                active_demands: 0,
                urgent_pulse: false,
                notification,
                preferred_action: Some(BubbleAction::OpenTray),
            };
        }

        self.announced_logged_out = false;
        let member_label = status
            .member
            .as_ref()
            .map_or_else(|| "校园成员".to_string(), |m| m.label());

        let prev = self.last_status.as_ref();
        let prev_unread = prev.map_or(0, |p| p.unread_count);
        let unread_delta = status.unread_count.saturating_sub(prev_unread);

        let mut notification = None;
        let mut urgent_pulse = false;

        if unread_delta > 0 {
            urgent_pulse = true;
            // Prefer jumping directly to a chat room whose unread count increased.
            let newly_unread_room = status
                .unread_by_room
                .iter()
                .filter(|(_, count)| **count > 0)
                .find_map(|(room_id, count)| {
                    let prev_c = prev
                        .and_then(|p| p.unread_by_room.get(room_id).copied())
                        .unwrap_or(0);
                    (*count > prev_c).then_some((room_id.clone(), *count - prev_c))
                });

            if let Some((room_id, added)) = newly_unread_room {
                notification = Some(TrayNotification {
                    text: format!("叮咚！群聊有 {added} 条新消息，点我直达聊天室～"),
                    action: BubbleAction::OpenRoom(room_id),
                });
            } else {
                notification = Some(TrayNotification {
                    text: format!(
                        "叮咚！收到 {unread_delta} 条新匹配/回应提醒，点我打开托盘看看～"
                    ),
                    action: BubbleAction::OpenTray,
                });
            }
        }

        // Determine default click target whenever unread messages exist.
        let preferred_action = if status.unread_count > 0 {
            status
                .unread_by_room
                .iter()
                .find(|(_, count)| **count > 0)
                .map(|(room_id, _)| BubbleAction::OpenRoom(room_id.clone()))
                .or(Some(BubbleAction::OpenTray))
        } else {
            None
        };

        let update = TrayBridgeUpdate {
            connection: TrayConnection::LoggedIn {
                member_label,
                active_demands,
                unread_count: status.unread_count,
            },
            unread_count: status.unread_count,
            active_demands,
            urgent_pulse,
            notification,
            preferred_action,
        };
        self.last_status = Some(status);
        update
    }

    /// Process a `tray_unavailable` error; emits an update only when transitioning from online.
    pub fn on_offline(&mut self) -> Option<TrayBridgeUpdate> {
        if !self.was_online && self.last_status.is_none() {
            return None;
        }
        self.was_online = false;
        self.last_status = None;
        self.announced_logged_out = false;
        Some(TrayBridgeUpdate {
            connection: TrayConnection::Offline,
            unread_count: 0,
            active_demands: 0,
            urgent_pulse: false,
            notification: None,
            preferred_action: None,
        })
    }
}

/// Compute next poll delay with smart exponential backoff when tray is offline.
pub fn next_poll_delay(pace: PollPace, offline_streak: u32) -> Option<Duration> {
    match pace {
        PollPace::Paused => None,
        PollPace::Active if offline_streak == 0 => Some(Duration::from_secs(4)),
        PollPace::Relaxed if offline_streak == 0 => Some(Duration::from_secs(15)),
        _ => {
            // Tray offline: back off 15s -> 22s -> 30s (capped) to stay virtually invisible.
            let secs = match offline_streak {
                0 | 1 => 15,
                2 => 22,
                _ => 30,
            };
            Some(Duration::from_secs(secs))
        }
    }
}

struct BridgeShared {
    pace: PollPace,
    stop: bool,
    force_poll: bool,
    queue: VecDeque<TrayBridgeUpdate>,
}

/// Non-blocking background `tray.sock` synchronization worker.
pub struct TrayBridge {
    socket_path: PathBuf,
    shared: Arc<(Mutex<BridgeShared>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl TrayBridge {
    /// Start background polling against `socket_path` (defaults to `${MILKBIRD_TRAY_DIR:-$HOME/.milkbird}/tray.sock`).
    pub fn start(socket_path: Option<PathBuf>) -> Self {
        let path = socket_path.unwrap_or_else(default_socket_path);
        let shared = Arc::new((
            Mutex::new(BridgeShared {
                pace: PollPace::Active,
                stop: false,
                force_poll: true,
                queue: VecDeque::new(),
            }),
            Condvar::new(),
        ));

        let worker_shared = Arc::clone(&shared);
        let worker_path = path.clone();
        let worker = thread::Builder::new()
            .name("milkbird-pet-bridge".to_string())
            .spawn(move || run_worker(worker_path, worker_shared))
            .ok();

        Self {
            socket_path: path,
            shared,
            worker,
        }
    }

    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Update polling cadence (`Active` / `Relaxed` / `Paused` for 0% CPU suspend).
    pub fn set_pace(&self, pace: PollPace) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock()
            && guard.pace != pace
        {
            let was_paused = guard.pace == PollPace::Paused;
            guard.pace = pace;
            if was_paused && pace != PollPace::Paused {
                guard.force_poll = true;
            }
            cvar.notify_all();
        }
    }

    /// Trigger an immediate out-of-band status refresh (e.g., after clicking a bubble).
    pub fn request_refresh(&self) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock() {
            guard.force_poll = true;
            cvar.notify_all();
        }
    }

    /// Non-blocking drain of the next pending tray status update (safe to call on UI thread).
    pub fn try_recv(&self) -> Option<TrayBridgeUpdate> {
        let (lock, _) = &*self.shared;
        lock.lock().ok()?.queue.pop_front()
    }
}

impl Drop for TrayBridge {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut guard) = lock.lock() {
            guard.stop = true;
            cvar.notify_all();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

fn run_worker(socket_path: PathBuf, shared: Arc<(Mutex<BridgeShared>, Condvar)>) {
    let client = TraySocketClient::new(socket_path);
    let mut differ = TrayStateDiffer::new();
    let mut offline_streak: u32 = 0;
    let mut cached_active_demands: u32 = 0;
    let mut polls_since_demands: u32 = 0;

    loop {
        let pace = {
            let (lock, cvar) = &*shared;
            let mut guard = match lock.lock() {
                Ok(g) => g,
                Err(_) => return,
            };

            loop {
                if guard.stop {
                    return;
                }
                if guard.pace == PollPace::Paused {
                    guard = match cvar.wait(guard) {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    continue;
                }
                if guard.force_poll {
                    guard.force_poll = false;
                    break guard.pace;
                }
                let Some(delay) = next_poll_delay(guard.pace, offline_streak) else {
                    continue;
                };
                let (next_guard, wait_res) = match cvar.wait_timeout(guard, delay) {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                guard = next_guard;
                if guard.stop {
                    return;
                }
                if wait_res.timed_out() || guard.force_poll {
                    guard.force_poll = false;
                    if guard.pace != PollPace::Paused {
                        break guard.pace;
                    }
                }
            }
        };

        let _ = pace;
        match client.fetch_status() {
            Ok(status) => {
                let became_online = offline_streak > 0 || polls_since_demands == 0;
                offline_streak = 0;

                // Refresh active demand count lightly every 3rd poll or when unread/online changes.
                if status.logged_in {
                    let unread_changed = differ
                        .last_status
                        .as_ref()
                        .is_none_or(|p| p.unread_count != status.unread_count);
                    if became_online || unread_changed || polls_since_demands >= 3 {
                        if let Ok(cnt) = client.fetch_active_demand_count() {
                            cached_active_demands = cnt;
                        }
                        polls_since_demands = 1;
                    } else {
                        polls_since_demands += 1;
                    }
                } else {
                    cached_active_demands = 0;
                    polls_since_demands = 0;
                }

                let update = differ.on_status(status, cached_active_demands);
                push_update(&shared, update);
            }
            Err(BridgeError::TrayUnavailable(_) | BridgeError::Timeout) => {
                offline_streak = offline_streak.saturating_add(1);
                cached_active_demands = 0;
                polls_since_demands = 0;
                if let Some(update) = differ.on_offline() {
                    push_update(&shared, update);
                }
            }
            Err(_) => {
                // Transient server or protocol issue: back off mildly without alarming user.
                offline_streak = offline_streak.saturating_add(1);
            }
        }
    }
}

fn push_update(shared: &Arc<(Mutex<BridgeShared>, Condvar)>, update: TrayBridgeUpdate) {
    let (lock, _) = &**shared;
    if let Ok(mut guard) = lock.lock() {
        // Keep queue bounded even if UI is momentarily busy.
        while guard.queue.len() >= 8 {
            guard.queue.pop_front();
        }
        guard.queue.push_back(update);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::bridge::launcher::{LaunchOutcome, dispatch_action_sync};
    use crate::bridge::socket::{MAX_FRAME_BYTES, TraySocketClient};
    use crate::bubble::ScreenRect;
    use crate::platform::{LaunchOptions, PetSession};
    use crate::state::PetState;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn unique_sock_path(tag: &str) -> PathBuf {
        static SEQ: AtomicUsize = AtomicUsize::new(1);
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("mb-pet-{tag}-{}-{id}.sock", std::process::id()))
    }

    fn spawn_mock_server<F>(path: PathBuf, max_conns: usize, handler: F) -> JoinHandle<()>
    where
        F: Fn(String, Value) -> String + Send + 'static,
    {
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind mock tray.sock");
        thread::spawn(move || {
            for stream in listener.incoming().take(max_conns) {
                let Ok(mut stream) = stream else { break };
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                if reader.read_line(&mut line).is_ok() && !line.trim().is_empty() {
                    let req: Value = serde_json::from_str(line.trim()).unwrap_or(Value::Null);
                    let action = req
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let payload = req.get("payload").cloned().unwrap_or(json!({}));
                    let reply = handler(action, payload);
                    let _ = stream.write_all(reply.as_bytes());
                    let _ = stream.flush();
                }
            }
            let _ = std::fs::remove_file(&path);
        })
    }

    #[test]
    fn mock_socket_handles_status_unread_diff_read_clear_and_open_room() {
        let sock = unique_sock_path("flow");
        let opened_room = Arc::new(Mutex::new(None::<String>));
        let opened_room_srv = Arc::clone(&opened_room);
        let call_idx = Arc::new(AtomicUsize::new(0));
        let call_idx_srv = Arc::clone(&call_idx);

        let srv = spawn_mock_server(sock.clone(), 5, move |action, payload| match action.as_str() {
            "status" => {
                let n = call_idx_srv.fetch_add(1, Ordering::Relaxed);
                match n {
                    0 => format!(
                        "{}\n",
                        json!({
                            "ok": true,
                            "action": "status",
                            "data": {
                                "loggedIn": true,
                                "member": { "id": 7, "displayName": "yyb" },
                                "unreadCount": 0,
                                "unreadByDemand": {},
                                "unreadByRoom": {}
                            }
                        })
                    ),
                    1 => format!(
                        "{}\n",
                        json!({
                            "ok": true,
                            "action": "status",
                            "data": {
                                "loggedIn": true,
                                "member": { "id": 7, "displayName": "yyb" },
                                "unreadCount": 2,
                                "unreadByDemand": { "d-10": { "responseCount": 0, "roomMessageCount": 2 } },
                                "unreadByRoom": { "room-42": 2 }
                            }
                        })
                    ),
                    _ => format!(
                        "{}\n",
                        json!({
                            "ok": true,
                            "action": "status",
                            "data": {
                                "loggedIn": true,
                                "member": { "id": 7, "displayName": "yyb" },
                                "unreadCount": 0,
                                "unreadByDemand": {},
                                "unreadByRoom": {}
                            }
                        })
                    ),
                }
            }
            "open_room" => {
                let rid = payload
                    .get("roomId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                *opened_room_srv.lock().unwrap() = Some(rid.clone());
                format!(
                    "{}\n",
                    json!({ "ok": true, "action": "open_room", "data": { "roomId": rid } })
                )
            }
            _ => format!(
                "{}\n",
                json!({ "ok": false, "code": "unknown_action", "message": "unknown" })
            ),
        });

        let client = TraySocketClient::new(sock.clone());
        let mut differ = TrayStateDiffer::new();

        // 1. Initial calm status.
        let s0 = client.fetch_status().expect("status 0");
        let u0 = differ.on_status(s0, 1);
        assert_eq!(u0.unread_count, 0);
        assert!(!u0.urgent_pulse);
        assert!(u0.notification.is_none());

        // 2. Unread increases in room-42 -> triggers AlertMatch + OpenRoom bubble!
        let s1 = client.fetch_status().expect("status 1");
        let u1 = differ.on_status(s1, 1);
        assert_eq!(u1.unread_count, 2);
        assert!(u1.urgent_pulse);
        let notif = u1.notification.expect("room notification");
        assert_eq!(notif.action, BubbleAction::OpenRoom("room-42".to_string()));
        assert!(notif.text.contains("2"));

        // 3. Clicking bubble calls open_room("room-42") over tray.sock.
        let outcome = dispatch_action_sync(&sock, &notif.action, false).expect("open_room ok");
        assert_eq!(outcome, LaunchOutcome::OpenedRoom("room-42".to_string()));
        assert_eq!(opened_room.lock().unwrap().as_deref(), Some("room-42"));

        // 4. Unread cleared -> returns calmly to zero unread.
        let s2 = client.fetch_status().expect("status 2");
        let u2 = differ.on_status(s2, 1);
        assert_eq!(u2.unread_count, 0);
        assert!(!u2.urgent_pulse);
        assert!(u2.preferred_action.is_none());

        // Clean up unused 5th connection slot if any by calling status once more.
        let _ = client.fetch_status();
        srv.join().unwrap();
    }

    #[test]
    fn mock_socket_handles_not_logged_in_offline_oversized_and_timeout() {
        // 1. Offline (nonexistent socket).
        let missing = unique_sock_path("missing");
        let offline_err = TraySocketClient::new(missing).fetch_status().unwrap_err();
        assert!(matches!(offline_err, BridgeError::TrayUnavailable(_)));
        assert_eq!(next_poll_delay(PollPace::Active, 1), Some(Duration::from_secs(15)));
        assert_eq!(next_poll_delay(PollPace::Active, 2), Some(Duration::from_secs(22)));
        assert_eq!(next_poll_delay(PollPace::Active, 5), Some(Duration::from_secs(30)));
        assert_eq!(next_poll_delay(PollPace::Paused, 0), None);

        // 2. not_logged_in error code & status loggedIn=false.
        let sock_auth = unique_sock_path("auth");
        let srv_auth = spawn_mock_server(sock_auth.clone(), 2, |action, _| match action.as_str() {
            "status" => format!(
                "{}\n",
                json!({ "ok": true, "action": "status", "data": { "loggedIn": false, "unreadCount": 0 } })
            ),
            _ => format!(
                "{}\n",
                json!({ "ok": false, "code": "not_logged_in", "message": "请先登录校园邮箱" })
            ),
        });
        let auth_client = TraySocketClient::new(sock_auth);
        let st = auth_client.fetch_status().expect("status logged out");
        assert!(!st.logged_in);
        let mut differ = TrayStateDiffer::new();
        let u_out = differ.on_status(st, 0);
        assert_eq!(u_out.connection, TrayConnection::LoggedOut);
        assert!(u_out.notification.is_some());
        let err = auth_client.fetch_active_demand_count().unwrap_err();
        assert!(matches!(err, BridgeError::NotLoggedIn(_)));
        srv_auth.join().unwrap();

        // 3. Oversized (> 32 KiB) response rejected cleanly.
        let sock_big = unique_sock_path("big");
        let srv_big = spawn_mock_server(sock_big.clone(), 1, |_, _| {
            let huge = "x".repeat(MAX_FRAME_BYTES + 256);
            format!("{{\"ok\":true,\"data\":{{\"blob\":\"{huge}\"}}}}\n")
        });
        let big_err = TraySocketClient::new(sock_big).fetch_status().unwrap_err();
        assert_eq!(big_err, BridgeError::FrameTooLarge);
        srv_big.join().unwrap();

        // 4. Slow server triggers short socket timeout.
        let sock_slow = unique_sock_path("slow");
        let _ = std::fs::remove_file(&sock_slow);
        let listener = UnixListener::bind(&sock_slow).unwrap();
        let sock_slow_cleanup = sock_slow.clone();
        let srv_slow = thread::spawn(move || {
            if let Ok((_stream, _)) = listener.accept() {
                thread::sleep(Duration::from_millis(300));
            }
            let _ = std::fs::remove_file(&sock_slow_cleanup);
        });
        let slow_client =
            TraySocketClient::new(sock_slow).with_timeout(Duration::from_millis(80));
        let timeout_err = slow_client.fetch_status().unwrap_err();
        assert_eq!(timeout_err, BridgeError::Timeout);
        srv_slow.join().unwrap();
    }

    #[test]
    fn session_end_to_end_syncs_alert_match_from_live_mock_socket() {
        let sock = unique_sock_path("e2e");
        // Expect 2 RPCs on first sync (`status` + `list_demands`), then mock server exits cleanly.
        let srv = spawn_mock_server(sock.clone(), 2, |action, _| match action.as_str() {
            "status" => format!(
                "{}\n",
                json!({
                    "ok": true,
                    "action": "status",
                    "data": {
                        "loggedIn": true,
                        "member": { "id": 7, "displayName": "yyb" },
                        "unreadCount": 1,
                        "unreadByDemand": {},
                        "unreadByRoom": { "room-e2e": 1 }
                    }
                })
            ),
            "list_demands" => format!(
                "{}\n",
                json!({ "ok": true, "action": "list_demands", "data": { "demands": [{"id":"d1"}] } })
            ),
            _ => format!("{}\n", json!({ "ok": true, "data": {} })),
        });

        let screens = [ScreenRect::new(0.0, 0.0, 1440.0, 900.0)];
        let cfg = std::env::temp_dir().join(format!("mb-pet-e2e-{}.json", std::process::id()));
        let opts = LaunchOptions {
            config_path: Some(cfg.clone()),
            socket_path: Some(sock.clone()),
            offline: false,
            ..LaunchOptions::default()
        };
        let mut session = PetSession::new(2.0, &screens, &opts);

        // Wait up to 1.5s for background bridge thread to deliver the live mock status.
        let mut alerted = false;
        for _ in 0..30 {
            let out = session.step_with_dt(0.05, &screens);
            if session.state() == PetState::AlertMatch && out.bubble.is_some() {
                alerted = true;
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
        assert!(alerted, "PetSession should enter AlertMatch with bubble from live tray.sock");
        assert!(session.tray_status_label().contains("yyb"));
        assert!(session.tray_status_label().contains("1 未读"));

        drop(session);
        let _ = std::fs::remove_file(&cfg);
        let _ = srv.join();
    }
}
