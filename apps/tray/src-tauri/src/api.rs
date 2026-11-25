use crate::session::Session;
use reqwest::blocking::{Client, RequestBuilder};
use reqwest::{Method, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::error::Error;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
static CLIENT_MSG_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_client_msg_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = CLIENT_MSG_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("tray-{nanos}-{seq}")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Failure {
    pub code: String,
    pub message: String,
    #[serde(skip)]
    pub unauthorized: bool,
    #[serde(skip)]
    pub retryable: bool,
}

impl Failure {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            unauthorized: false,
            retryable: false,
        }
    }

    fn from_http(status: StatusCode, bytes: &[u8]) -> Self {
        let body = serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null);
        let error = body.get("error").unwrap_or(&body);
        let (code, message) = match status {
            StatusCode::UNAUTHORIZED => ("unauthorized", "登录凭据无效或已过期。"),
            StatusCode::FORBIDDEN => ("forbidden", "当前账号无法执行此操作。"),
            StatusCode::NOT_FOUND => ("not_found", "找不到这条记录。"),
            StatusCode::CONFLICT => ("conflict", "记录状态已变化，请刷新后重试。"),
            StatusCode::TOO_MANY_REQUESTS => ("rate_limited", "操作过于频繁，请稍后重试。"),
            _ if status.is_client_error() => ("invalid_request", "俱乐部拒绝了这次请求。"),
            _ => (
                "server_unavailable",
                "俱乐部暂时无法处理此请求，请稍后重试。",
            ),
        };
        let code = error
            .as_str()
            .or_else(|| error.get("code").and_then(Value::as_str))
            .or_else(|| body.get("code").and_then(Value::as_str))
            .unwrap_or(code);
        if status == StatusCode::CONFLICT && code == "demand_not_editable" {
            return Failure::new(code, "当前需求状态已不可修改。");
        }
        let message = match (status, code) {
            (StatusCode::CONFLICT, "active_demand_limit_reached") => {
                "最多可同时挂起 3 个需求，请先完成或撤回一个需求。"
            }
            (StatusCode::CONFLICT, "room_expired") => "这个临时群已到期。",
            (StatusCode::CONFLICT, "demand_not_open") => "这条需求已结束，无法继续接收回应。",
            (StatusCode::CONFLICT, "demand_submission_withdrawn") => {
                "这次提交已撤回，请重新确认后发起新需求。"
            }
            (StatusCode::CONFLICT, "match_profile_changed") => {
                "画像已更新，请重新确认本次共享字段。"
            }
            (StatusCode::CONFLICT, "profile_conflict") => {
                "资料已更新，请比较当前值和你的编辑后选择。"
            }
            (StatusCode::CONFLICT, "idempotency_conflict") => {
                "这次提交与原确认单不同，请重新确认后提交。"
            }
            _ => message,
        };
        let mut failure = Failure::new(
            code,
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| body.get("message").and_then(Value::as_str))
                .unwrap_or(message),
        );
        failure.unauthorized = status == StatusCode::UNAUTHORIZED;
        failure.retryable = status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS;
        failure
    }

    pub fn invalid_request() -> Self {
        Self::new("invalid_request", "请求格式有误。")
    }

    pub fn invalid_response() -> Self {
        Self::new("invalid_response", "俱乐部返回了无法读取的结果。")
    }

    pub fn login_required() -> Self {
        Self::new(
            "not_logged_in",
            "请先在 MilkBird Tray 用 hkust-gz.edu.cn 校园邮箱登录。",
        )
    }

    pub fn confirmation_required() -> Self {
        Self::new("confirmation_required", "请先确认本次提交。")
    }

    pub fn session_io() -> Self {
        Self::new("session_io", "无法更新本机登录记录。")
    }
}

pub type ApiResult<T = Value> = Result<T, Failure>;

pub struct Api {
    base: Url,
    short: Client,
    long: Client,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Event {
    pub seq: u64,
    #[serde(rename = "type", alias = "kind")]
    pub kind: String,
    #[serde(default, alias = "payload")]
    pub data: Value,
}

#[derive(Deserialize)]
pub struct EventBatch {
    pub events: Vec<Event>,
    #[serde(rename = "lastSeq")]
    pub last_seq: u64,
}

impl Api {
    pub fn from_env() -> Result<Self, Box<dyn Error>> {
        let origin =
            std::env::var("MILKBIRD_API_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
        Self::new(&origin)
    }

    pub fn new(origin: &str) -> Result<Self, Box<dyn Error>> {
        let base = Url::parse(origin)?;
        let loopback = matches!(base.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
        if !(base.scheme() == "https" || base.scheme() == "http" && loopback)
            || base.path() != "/"
            || base.query().is_some()
            || base.fragment().is_some()
            || !base.username().is_empty()
            || base.password().is_some()
        {
            return Err(
                "MILKBIRD_API_ORIGIN must be an HTTPS origin or loopback HTTP origin".into(),
            );
        }
        let client = |seconds| {
            Client::builder()
                .timeout(Duration::from_secs(seconds))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
        };
        Ok(Self {
            base,
            short: client(5)?,
            long: client(35)?,
        })
    }

    pub fn origin(&self) -> &str {
        self.base.as_str().trim_end_matches('/')
    }

    fn url(&self, path: &[&str]) -> ApiResult<Url> {
        if path.iter().any(|part| matches!(*part, "" | "." | "..")) {
            return Err(Failure::invalid_request());
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("HTTP origin")
            .extend(["api", "v1"])
            .extend(path.iter().copied());
        Ok(url)
    }

    fn send(request: RequestBuilder, token: Option<&str>) -> ApiResult {
        let request = if let Some(token) = token {
            request.bearer_auth(token)
        } else {
            request
        };
        let response = request.send().map_err(|_| {
            let mut error = Failure::new("server_unavailable", "暂时无法连接俱乐部，请稍后重试。");
            error.retryable = true;
            error
        })?;
        let status = response.status();
        let mut bytes = Vec::new();
        let read = response.take(MAX_RESPONSE + 1).read_to_end(&mut bytes);
        if read.is_err() || bytes.len() as u64 > MAX_RESPONSE {
            if status.is_success() {
                return Err(Failure::invalid_response());
            }
            bytes.clear();
        }
        if status.is_success() {
            return if status == StatusCode::NO_CONTENT || bytes.is_empty() {
                Ok(json!({}))
            } else {
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| Failure::invalid_response())?;
                if !value.is_object() {
                    return Err(Failure::invalid_response());
                }
                Ok(value)
            };
        }
        Err(Failure::from_http(status, &bytes))
    }

    fn get(&self, path: &[&str], query: &[(&str, &str)], token: &str) -> ApiResult {
        Self::send(self.short.get(self.url(path)?).query(query), Some(token))
    }

    fn write(
        &self,
        method: Method,
        path: &[&str],
        body: Option<&Value>,
        token: Option<&str>,
    ) -> ApiResult {
        let mut request = self.short.request(method, self.url(path)?);
        if let Some(body) = body {
            request = request.json(body);
        }
        Self::send(request, token)
    }

    pub fn request_code(&self, email: &str) -> ApiResult {
        self.write(
            Method::POST,
            &["auth", "email", "code"],
            Some(&json!({ "email": email })),
            None,
        )
    }

    pub fn verify_code(
        &self,
        email: &str,
        code: &str,
        display_name: Option<&str>,
    ) -> ApiResult<Session> {
        let mut body = json!({ "email": email, "code": code });
        if let Some(display_name) = display_name {
            body["displayName"] = json!(display_name);
        }
        let result = self.write(
            Method::POST,
            &["auth", "email", "verify"],
            Some(&body),
            None,
        )?;
        let session: Session =
            serde_json::from_value(result).map_err(|_| Failure::invalid_response())?;
        if !session.is_valid() {
            return Err(Failure::invalid_response());
        }
        Ok(session)
    }

    pub fn me(&self, token: &str) -> ApiResult {
        self.get(&["me"], &[], token)
    }

    pub fn logout(&self, token: &str) -> ApiResult {
        self.write(Method::POST, &["auth", "logout"], None, Some(token))
    }

    pub fn submit_demand(&self, token: &str, body: &Value) -> ApiResult {
        self.write(Method::POST, &["demands"], Some(body), Some(token))
    }

    pub fn withdraw_submission(&self, token: &str, key: &str) -> ApiResult {
        self.write(
            Method::POST,
            &["demands", "withdraw-submission"],
            Some(&json!({ "idempotencyKey": key })),
            Some(token),
        )
    }

    pub fn update_demand(&self, token: &str, id: &str, body: &Value) -> ApiResult {
        self.write(Method::PATCH, &["demands", id], Some(body), Some(token))
    }

    pub fn list_demand_revisions(&self, token: &str, id: &str) -> ApiResult {
        self.get(&["demands", id, "revisions"], &[], token)
    }

    pub fn list_demands(&self, token: &str, status: Option<&str>) -> ApiResult {
        let query = status.map(|status| [("status", status)]);
        self.get(
            &["demands"],
            query.as_ref().map_or(&[], |query| query.as_slice()),
            token,
        )
    }

    pub fn get_demand(&self, token: &str, id: &str) -> ApiResult {
        self.get(&["demands", id], &[], token)
    }

    pub fn cancel_demand(&self, token: &str, id: &str) -> ApiResult {
        self.write(Method::POST, &["demands", id, "cancel"], None, Some(token))
    }

    pub fn complete_demand(&self, token: &str, id: &str) -> ApiResult {
        self.write(
            Method::POST,
            &["demands", id, "complete"],
            None,
            Some(token),
        )
    }

    pub fn list_demand_responses(&self, token: &str, demand_id: &str) -> ApiResult {
        self.get(&["demands", demand_id, "responses"], &[], token)
    }

    pub fn feedback_response(&self, token: &str, response_id: &str, feedback: &str) -> ApiResult {
        self.write(
            Method::POST,
            &["responses", response_id, "feedback"],
            Some(&json!({ "feedback": feedback })),
            Some(token),
        )
    }

    pub fn list_matches(&self, token: &str) -> ApiResult {
        self.get(&["matches"], &[], token)
    }

    pub fn respond_match(&self, token: &str, id: &str, decision: &str) -> ApiResult {
        self.write(
            Method::POST,
            &["matches", id, "respond"],
            Some(&json!({ "decision": decision })),
            Some(token),
        )
    }

    pub fn events(&self, token: &str, after: u64) -> ApiResult<EventBatch> {
        let url = self.url(&["events"])?;
        let request = self.long.get(url).query(&[("after", after), ("wait", 25)]);
        let result = Self::send(request, Some(token))?;
        serde_json::from_value(result).map_err(|_| Failure::invalid_response())
    }

    pub fn list_rooms(&self, token: &str) -> ApiResult {
        self.get(&["rooms"], &[], token)
    }

    pub fn list_room_messages(&self, token: &str, id: &str, after: u64) -> ApiResult {
        self.get(
            &["rooms", id, "messages"],
            &[("after", &after.to_string())],
            token,
        )
    }

    pub fn send_room_message(&self, token: &str, id: &str, text: &str) -> ApiResult {
        self.write(
            Method::POST,
            &["rooms", id, "messages"],
            Some(&json!({ "clientMsgId": next_client_msg_id(), "body": text })),
            Some(token),
        )
    }

    pub fn leave_room(&self, token: &str, id: &str) -> ApiResult {
        self.write(Method::POST, &["rooms", id, "leave"], None, Some(token))
    }

    pub fn vote_room_retention(&self, token: &str, room_id: &str, keep: bool) -> ApiResult {
        self.write(
            Method::POST,
            &["rooms", room_id, "retention-vote"],
            Some(&json!({ "keep": keep })),
            Some(token),
        )
    }

    pub fn sync_local_profile(&self, token: &str, body: &Value) -> ApiResult {
        self.write(Method::PUT, &["profile", "local"], Some(body), Some(token))
    }

    pub fn edit_profile_fields(&self, token: &str, body: &Value) -> ApiResult {
        self.write(Method::PATCH, &["profile", "local"], Some(body), Some(token))
    }

    pub fn get_profile(&self, token: &str) -> ApiResult {
        self.get(&["profile"], &[], token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_v2_conflict_codes_and_uses_specific_messages() {
        let limit = Failure::from_http(
            StatusCode::CONFLICT,
            br#"{"error":"active_demand_limit_reached"}"#,
        );
        assert_eq!(limit.code, "active_demand_limit_reached");
        assert!(limit.message.contains("3 个需求"));
        assert!(!limit.unauthorized);

        let expired = Failure::from_http(
            StatusCode::CONFLICT,
            br#"{"error":{"code":"room_expired"}}"#,
        );
        assert_eq!(expired.code, "room_expired");
        assert_eq!(expired.message, "这个临时群已到期。");
        assert!(!expired.unauthorized);
    }

    #[test]
    fn endpoint_keeps_ids_in_one_path_segment() {
        let api = Api::new("http://127.0.0.1:8080").unwrap();
        let url = api.url(&["rooms", "a/b?c", "messages"]).unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:8080/api/v1/rooms/a%2Fb%3Fc/messages"
        );
        assert!(api.url(&["rooms", "..", "messages"]).is_err());
    }

    #[test]
    fn event_batch_preserves_cursor_after_suppressed_deliveries() {
        let batch: EventBatch = serde_json::from_str(r#"{"events":[],"lastSeq":42}"#).unwrap();
        assert!(batch.events.is_empty());
        assert_eq!(batch.last_seq, 42);
    }

    #[test]
    fn demand_not_editable_preserves_code_and_uses_tray_message() {
        let failure = Failure::from_http(
            StatusCode::CONFLICT,
            br#"{"error":{"code":"demand_not_editable","message":"Demand is closed"}}"#,
        );
        assert_eq!(failure.code, "demand_not_editable");
        assert_eq!(failure.message, "当前需求状态已不可修改。");
        assert!(!failure.unauthorized);
    }
}
