//! `tray.sock` Unix Domain Socket v1 JSON-line protocol client.
//!
//! Strictly conforms to `contracts/tray/v1/tools.md`:
//! - Default path: `${MILKBIRD_TRAY_DIR:-$HOME/.milkbird}/tray.sock`
//! - Short-lived Unix socket connection per request: 1 UTF-8 JSON line + `\n`, 1 JSON line reply.
//! - Hard 32 KiB frame limit (`MAX_FRAME_BYTES`) on both requests and responses.
//! - Short socket timeouts (`1.0 s`) so background probes never hang.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

/// Protocol version defined in `contracts/tray/v1/tools.md`.
pub const TRAY_PROTOCOL_VERSION: u32 = 1;

/// Hard frame cap (32 KiB including trailing `\n`) per `contracts/tray/v1/tools.md`.
pub const MAX_FRAME_BYTES: usize = 32 * 1024;

/// Default read/write timeout for short-lived `tray.sock` RPC calls.
pub const DEFAULT_SOCKET_TIMEOUT: Duration = Duration::from_millis(1000);

/// Resolve default `tray.sock` path respecting `MILKBIRD_TRAY_DIR`.
pub fn default_socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("MILKBIRD_TRAY_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("tray.sock");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".milkbird").join("tray.sock");
    }
    PathBuf::from("tray.sock")
}

/// Member metadata returned inside `status.data.member` when logged in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayMember {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub campus: Option<String>,
}

impl TrayMember {
    pub fn label(&self) -> String {
        if let Some(ref name) = self.display_name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Some(ref email) = self.email {
            if let Some((prefix, _)) = email.split_once('@')
                && !prefix.trim().is_empty()
            {
                return prefix.trim().to_string();
            }
            let trimmed = email.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        "校园成员".to_string()
    }
}

/// Per-demand unread breakdown returned in `status.data.unreadByDemand`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemandUnread {
    #[serde(default)]
    pub response_count: u32,
    #[serde(default)]
    pub room_message_count: u32,
}

impl DemandUnread {
    pub const fn total(self) -> u32 {
        self.response_count.saturating_add(self.room_message_count)
    }
}

/// Structured response of `{"version":1,"action":"status","payload":{}}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayStatusData {
    #[serde(default)]
    pub logged_in: bool,
    #[serde(default)]
    pub member: Option<TrayMember>,
    #[serde(default)]
    pub api_origin: Option<String>,
    #[serde(default)]
    pub unread_count: u32,
    #[serde(default)]
    pub unread_by_demand: BTreeMap<String, DemandUnread>,
    #[serde(default)]
    pub unread_by_room: BTreeMap<String, u32>,
}

/// Errors produced by `tray.sock` v1 RPC calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    /// Socket missing, connection refused, or platform unsupported (quietly degrade to offline).
    TrayUnavailable(String),
    /// Tray is running, but user has not logged in yet.
    NotLoggedIn(String),
    /// Hub backend temporarily unreachable according to tray.
    ServerUnavailable(String),
    /// Request or response exceeded the 32 KiB protocol limit.
    FrameTooLarge,
    /// Socket timed out within [`DEFAULT_SOCKET_TIMEOUT`].
    Timeout,
    /// Any other structured error returned by tray (`ok: false`).
    ProtocolError { code: String, message: String },
    /// Malformed JSON or transport error.
    InvalidResponse(String),
}

#[derive(Debug, Deserialize)]
struct RawEnvelope {
    ok: bool,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

/// Low-level JSON-line `tray.sock` v1 RPC client.
#[derive(Debug, Clone)]
pub struct TraySocketClient {
    socket_path: PathBuf,
    timeout: Duration,
}

impl TraySocketClient {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            timeout: DEFAULT_SOCKET_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.max(Duration::from_millis(50));
        self
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Call `status` (does not require login).
    pub fn fetch_status(&self) -> Result<TrayStatusData, BridgeError> {
        let data = self.call("status", json!({}))?;
        serde_json::from_value::<TrayStatusData>(data)
            .map_err(|e| BridgeError::InvalidResponse(format!("status 解析失败: {e}")))
    }

    /// Call `list_demands` with `{"status":"open"}` to count active demands.
    pub fn fetch_active_demand_count(&self) -> Result<u32, BridgeError> {
        let data = self.call("list_demands", json!({ "status": "open" }))?;
        let count = data
            .get("demands")
            .and_then(Value::as_array)
            .map_or(0, |arr| arr.len() as u32);
        Ok(count)
    }

    /// Call `open_room` to open a specific chat room in MilkBird Tray and clear its unread badge.
    pub fn open_room(&self, room_id: &str, demand_id: Option<&str>) -> Result<(), BridgeError> {
        let mut payload = serde_json::Map::new();
        payload.insert("roomId".to_string(), Value::String(room_id.to_string()));
        if let Some(did) = demand_id
            && !did.trim().is_empty()
        {
            payload.insert("demandId".to_string(), Value::String(did.to_string()));
        }
        let _ = self.call("open_room", Value::Object(payload))?;
        Ok(())
    }

    /// Send one v1 JSON-line frame and parse the JSON-line envelope under strict 32 KiB bounds.
    pub fn call(&self, action: &str, payload: Value) -> Result<Value, BridgeError> {
        if !payload.is_object() {
            return Err(BridgeError::ProtocolError {
                code: "invalid_request".to_string(),
                message: "payload 必须是 JSON 对象".to_string(),
            });
        }
        let mut req_line = serde_json::to_string(&json!({
            "version": TRAY_PROTOCOL_VERSION,
            "action": action,
            "payload": payload,
        }))
        .map_err(|e| BridgeError::InvalidResponse(e.to_string()))?;
        req_line.push('\n');

        if req_line.len() > MAX_FRAME_BYTES {
            return Err(BridgeError::FrameTooLarge);
        }

        let resp_line = self.exchange_line(req_line.as_bytes())?;
        let env: RawEnvelope = serde_json::from_str(&resp_line)
            .map_err(|e| BridgeError::InvalidResponse(format!("JSON 响应非法: {e}")))?;

        if env.ok {
            Ok(env.data)
        } else {
            match env.code.as_str() {
                "not_logged_in" => Err(BridgeError::NotLoggedIn(env.message)),
                "server_unavailable" => Err(BridgeError::ServerUnavailable(env.message)),
                "response_too_large" => Err(BridgeError::FrameTooLarge),
                _ => Err(BridgeError::ProtocolError {
                    code: env.code,
                    message: env.message,
                }),
            }
        }
    }

    #[cfg(unix)]
    fn exchange_line(&self, request_bytes: &[u8]) -> Result<String, BridgeError> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|err| {
            use std::io::ErrorKind;
            match err.kind() {
                ErrorKind::NotFound
                | ErrorKind::ConnectionRefused
                | ErrorKind::ConnectionReset
                | ErrorKind::PermissionDenied => BridgeError::TrayUnavailable(err.to_string()),
                ErrorKind::TimedOut | ErrorKind::WouldBlock => BridgeError::Timeout,
                _ => BridgeError::TrayUnavailable(err.to_string()),
            }
        })?;

        let _ = stream.set_write_timeout(Some(self.timeout));
        let _ = stream.set_read_timeout(Some(self.timeout));

        stream
            .write_all(request_bytes)
            .and_then(|()| stream.flush())
            .map_err(map_io_err)?;

        // Cap reading at MAX_FRAME_BYTES + 1 so an oversized reply is caught cleanly without OOM.
        let mut limited = BufReader::new(stream.take((MAX_FRAME_BYTES + 1) as u64));
        let mut buf = Vec::with_capacity(512);
        let n = limited.read_until(b'\n', &mut buf).map_err(map_io_err)?;

        if n == 0 {
            return Err(BridgeError::InvalidResponse("托盘关闭连接前未返回响应".to_string()));
        }
        if buf.len() > MAX_FRAME_BYTES {
            return Err(BridgeError::FrameTooLarge);
        }
        if !buf.ends_with(b"\n") {
            return Err(BridgeError::InvalidResponse("响应缺少换行结束符".to_string()));
        }

        let line = String::from_utf8(buf)
            .map_err(|e| BridgeError::InvalidResponse(format!("响应非 UTF-8: {e}")))?;
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    #[cfg(not(unix))]
    fn exchange_line(&self, _request_bytes: &[u8]) -> Result<String, BridgeError> {
        Err(BridgeError::TrayUnavailable(
            "当前平台不支持 Unix Domain Socket".to_string(),
        ))
    }
}

#[cfg(unix)]
fn map_io_err(err: std::io::Error) -> BridgeError {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => BridgeError::Timeout,
        ErrorKind::ConnectionReset | ErrorKind::BrokenPipe | ErrorKind::ConnectionRefused => {
            BridgeError::TrayUnavailable(err.to_string())
        }
        _ => BridgeError::InvalidResponse(err.to_string()),
    }
}
