//! Non-blocking dispatcher that wakes `MilkBird Tray` or jumps directly into a chat room
//! via `tray.sock` `open_room` — with zero modifications to `apps/tray/`.

use crate::bridge::socket::{BridgeError, TraySocketClient};
use crate::bubble::BubbleAction;
use std::path::{Path, PathBuf};
use std::process::Command;

/// macOS bundle identifier of `MilkBird Tray` (`apps/tray/src-tauri/tauri.conf.json`).
pub const MILK_BIRD_TRAY_BUNDLE_ID: &str = "cn.fordece.milkbird.tray";

/// Outcome of a synchronous launch/dispatch call (useful for tests and diagnostics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOutcome {
    /// Opened target chat room directly via `tray.sock` `open_room`.
    OpenedRoom(String),
    /// Triggered OS activation of `MilkBird Tray` (`Reopen -> show_main`).
    ActivatedTray,
    /// No external navigation needed.
    NoOp,
}

/// Dispatch a [`BubbleAction`] synchronously (suitable for tests or background worker threads).
pub fn dispatch_action_sync(
    socket_path: &Path,
    action: &BubbleAction,
    activate_os_app: bool,
) -> Result<LaunchOutcome, BridgeError> {
    match action {
        BubbleAction::Dismiss => Ok(LaunchOutcome::NoOp),
        BubbleAction::OpenTray => {
            if activate_os_app {
                activate_milkbird_tray();
            }
            Ok(LaunchOutcome::ActivatedTray)
        }
        BubbleAction::OpenRoom(room_id) => {
            let client = TraySocketClient::new(socket_path.to_path_buf());
            let res = client.open_room(room_id, None);
            if activate_os_app {
                activate_milkbird_tray();
            }
            match res {
                Ok(()) => Ok(LaunchOutcome::OpenedRoom(room_id.clone())),
                Err(BridgeError::TrayUnavailable(_)) if activate_os_app => {
                    // Tray wasn't running yet; OS activation was still attempted gracefully.
                    Ok(LaunchOutcome::ActivatedTray)
                }
                Err(err) => Err(err),
            }
        }
    }
}

/// Fire-and-forget dispatch on a tiny detached thread so the UI/AppKit main thread never blocks.
pub fn dispatch_action_async(socket_path: PathBuf, action: BubbleAction) {
    if matches!(action, BubbleAction::Dismiss) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("milkbird-pet-open".to_string())
        .spawn(move || {
            let _ = dispatch_action_sync(&socket_path, &action, true);
        });
}

/// Activate or launch `MilkBird Tray` using macOS `open -b` (triggers `RunEvent::Reopen -> show_main`).
pub fn activate_milkbird_tray() {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open")
            .args(["-b", MILK_BIRD_TRAY_BUNDLE_ID])
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = MILK_BIRD_TRAY_BUNDLE_ID;
    }
}
