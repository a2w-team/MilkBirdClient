use crate::api::{ApiResult, Failure};
use crate::hub::{Hub, Target};
use crate::session::ensure_private_dir;
use serde::Deserialize;
use serde_json::{Value, json};
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const MAX_FRAME: u64 = 32 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolFrame {
    version: u8,
    action: String,
    payload: serde_json::Map<String, Value>,
}

fn parse(line: &[u8]) -> ApiResult<ToolFrame> {
    if line.len() as u64 > MAX_FRAME
        || line.last() != Some(&b'\n')
        || line.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{')
    {
        return Err(Failure::invalid_request());
    }
    let frame: ToolFrame = serde_json::from_slice(line).map_err(|_| Failure::invalid_request())?;
    if frame.version != 1 {
        return Err(Failure::new(
            "unsupported_version",
            "工具协议版本不受支持。",
        ));
    }
    Ok(frame)
}

fn optional_string<'a>(payload: &'a Value, field: &str) -> ApiResult<Option<&'a str>> {
    match payload.get(field) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Some)
            .ok_or_else(Failure::invalid_request),
    }
}

fn required_string<'a>(payload: &'a Value, field: &str) -> ApiResult<&'a str> {
    optional_string(payload, field)?.ok_or_else(Failure::invalid_request)
}

fn dispatch(hub: &Hub, action: &str, payload: Value) -> ApiResult {
    match action {
        "status" => Ok(hub.status()),
        "submit_demand" => hub.submit_demand(payload),
        "update_demand" => {
            let demand_id = required_string(&payload, "demandId")?.to_owned();
            hub.update_demand(&demand_id, payload)
        }
        "list_demands" => {
            let status = optional_string(&payload, "status")?;
            hub.with_api(|api, token| api.list_demands(token, status))
        }
        "get_demand" => {
            let demand_id = required_string(&payload, "demandId")?;
            hub.with_api(|api, token| api.get_demand(token, demand_id))
        }
        "cancel_demand" => hub.cancel_demand(required_string(&payload, "demandId")?),
        "complete_demand" => hub.complete_demand(required_string(&payload, "demandId")?),
        "list_demand_revisions" => {
            let demand_id = required_string(&payload, "demandId")?;
            hub.with_api(|api, token| api.list_demand_revisions(token, demand_id))
        }
        "list_demand_responses" => {
            let demand_id = required_string(&payload, "demandId")?;
            hub.with_api(|api, token| api.list_demand_responses(token, demand_id))
        }
        "list_matches" => hub.with_api(|api, token| api.list_matches(token)),
        "respond_match" => hub.respond_match(
            required_string(&payload, "matchId")?,
            required_string(&payload, "decision")?,
        ),
        "list_rooms" => hub.with_api(|api, token| api.list_rooms(token)),
        "open_room" => hub.open_room(
            required_string(&payload, "roomId")?.into(),
            optional_string(&payload, "demandId")?.map(str::to_owned),
        ),
        "vote_room_retention" => hub.vote_room_retention(
            required_string(&payload, "roomId")?,
            payload["keep"]
                .as_bool()
                .ok_or_else(Failure::invalid_request)?,
        ),
        "notify" => hub.notify(
            required_string(&payload, "title")?,
            required_string(&payload, "body")?,
            Target {
                demand_id: optional_string(&payload, "demandId")?.map(str::to_owned),
                match_id: optional_string(&payload, "matchId")?.map(str::to_owned),
                room_id: optional_string(&payload, "roomId")?.map(str::to_owned),
            },
        ),
        "sync_local_profile" => {
            for (field, value) in payload.as_object().ok_or_else(Failure::invalid_request)? {
                let valid = match field.as_str() {
                    "campus" | "bio" => value.is_string(),
                    "preferences" => value.is_object(),
                    _ => false,
                };
                if !valid {
                    return Err(Failure::invalid_request());
                }
            }
            hub.with_api(|api, token| api.sync_local_profile(token, &payload))
        }
        "edit_profile_fields" => {
            if payload["confirmed"] != true {
                return Err(Failure::confirmation_required());
            }
            let mut body = payload;
            body.as_object_mut()
                .expect("confirmed field implies object")
                .remove("confirmed");
            hub.with_api(|api, token| api.edit_profile_fields(token, &body))
        }
        "get_profile" => hub.with_api(|api, token| api.get_profile(token)),
        _ => Err(Failure::new("unknown_action", "未知的托盘操作。")),
    }
}

fn tool_error(error: Failure) -> Value {
    json!({ "ok": false, "code": error.code, "message": error.message })
}

fn serve(stream: &mut UnixStream, hub: &Hub) -> io::Result<()> {
    let mut line = Vec::new();
    let mut reader = BufReader::new((&*stream).take(MAX_FRAME + 1));
    reader.read_until(b'\n', &mut line)?;

    let response = parse(&line).and_then(|frame| {
        let data = dispatch(hub, &frame.action, Value::Object(frame.payload))?;
        Ok(json!({ "ok": true, "action": frame.action, "data": data }))
    });
    let mut encoded = serde_json::to_vec(&response.unwrap_or_else(tool_error))?;
    if encoded.len() as u64 >= MAX_FRAME {
        encoded = serde_json::to_vec(&tool_error(Failure::new(
            "response_too_large",
            "结果超过托盘工具的大小上限。",
        )))?;
    }
    encoded.push(b'\n');
    stream.write_all(&encoded)
}

pub struct Bridge {
    listener: UnixListener,
    path: PathBuf,
    _lock: File,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Bridge {
    pub fn bind(dir: &Path) -> Result<Self, Box<dyn Error>> {
        ensure_private_dir(dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(dir.join("tray.lock"))?;
        lock.try_lock()?;
        let path = dir.join("tray.sock");
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_socket() => fs::remove_file(&path)?,
            Ok(_) => return Err("tray.sock exists and is not a socket".into()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        let listener = UnixListener::bind(&path)?;
        let bridge = Self {
            listener,
            path,
            _lock: lock,
        };
        fs::set_permissions(&bridge.path, fs::Permissions::from_mode(0o600))?;
        Ok(bridge)
    }

    pub fn start(self, hub: Arc<Hub>) -> io::Result<()> {
        std::thread::Builder::new()
            .name("tray-bridge".into())
            .spawn(move || {
                for stream in self.listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    if stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .is_ok()
                        && stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .is_ok()
                    {
                        let _ = serve(&mut stream, &hub);
                    }
                }
                // Keep the instance lock and socket cleanup for the lifetime of the listener.
                drop(self);
            })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Api;
    use crate::session::{SessionStore, tests::TestDir};
    use std::net::Shutdown;

    fn logged_out_hub(dir: &TestDir) -> Hub {
        let session = SessionStore::open(dir.path()).unwrap();
        let api = Api::new("http://127.0.0.1:1").unwrap();
        Hub::new(api, session, |_| panic!("request cannot update UI"))
    }

    fn reply(hub: &Hub, request: &[u8]) -> Value {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client.write_all(request).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        serve(&mut server, hub).unwrap();
        drop(server);
        let mut response = Vec::new();
        client.read_to_end(&mut response).unwrap();
        assert_eq!(response.last(), Some(&b'\n'));
        serde_json::from_slice(&response).unwrap()
    }

    #[test]
    fn invalid_and_legacy_frames_return_tool_errors() {
        let dir = TestDir::new();
        let hub = logged_out_hub(&dir);
        let invalid_frames: &[&[u8]] = &[
            b"{\"version\":1,\"confirmed\":true,\"query\":{\"campus\":\"East\",\"topic\":\"Exams\"}}\n",
            b"not JSON\n",
            b"[1,\"status\",{}]\n",
            b"{\"version\":1,\"action\":\"status\"}\n",
            b"{\"version\":1,\"action\":\"status\",\"payload\":[]}\n",
            b"{\"version\":1,\"action\":\"status\",\"payload\":{},\"extra\":true}\n",
            br#"{"version":1,"action":"status","payload":{}}"#,
        ];
        for frame in invalid_frames {
            assert_eq!(
                reply(&hub, frame),
                json!({ "ok": false, "code": "invalid_request", "message": "请求格式有误。" })
            );
        }
    }

    #[test]
    fn parses_tool_action_and_enforces_frame_limit() {
        let mut line =
            br#"{"version":1,"action":"notify","payload":{"title":"Title","body":"Body"}}"#
                .to_vec();
        line.resize(MAX_FRAME as usize - 1, b' ');
        line.push(b'\n');
        let frame = parse(&line).unwrap();
        assert_eq!(frame.action, "notify");
        assert_eq!(frame.payload["body"], "Body");
        line.pop();
        line.push(b' ');
        line.push(b'\n');
        assert_eq!(parse(&line).unwrap_err().code, "invalid_request");
    }

    #[test]
    fn submit_requires_confirmation_and_login() {
        let dir = TestDir::new();
        let hub = logged_out_hub(&dir);
        let submit = |confirmed| {
            let frame = json!({ "version": 1, "action": "submit_demand", "payload": { "confirmed": confirmed } });
            reply(&hub, format!("{frame}\n").as_bytes())
        };
        assert_eq!(submit(json!("true"))["code"], "confirmation_required");
        assert_eq!(submit(json!(true))["code"], "not_logged_in");
        let rooms = b"{\"version\":1,\"action\":\"list_rooms\",\"payload\":{}}\n";
        assert_eq!(reply(&hub, rooms)["code"], "not_logged_in");

        let status = b"{\"version\":1,\"action\":\"status\",\"payload\":{}}\n";
        let response = reply(&hub, status);
        assert_eq!(response["ok"], true);
        assert_eq!(response["action"], "status");
        assert_eq!(response["data"]["loggedIn"], false);
    }

    #[test]
    fn update_requires_explicit_confirmation_and_demand_id() {
        let dir = TestDir::new();
        let hub = logged_out_hub(&dir);
        let update = |payload| {
            let frame = json!({ "version": 1, "action": "update_demand", "payload": payload });
            reply(&hub, format!("{frame}\n").as_bytes())
        };
        for payload in [
            json!({"demandId": "d1"}),
            json!({"demandId": "d1", "confirmed": false}),
            json!({"demandId": "d1", "confirmed": "true"}),
        ] {
            assert_eq!(update(payload)["code"], "confirmation_required");
        }
        assert_eq!(update(json!({"confirmed": true}))["code"], "invalid_request");
        assert_eq!(
            update(json!({"demandId": "d1", "confirmed": true}))["code"],
            "not_logged_in"
        );
    }
}
