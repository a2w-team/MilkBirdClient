use crate::api::{Api, ApiResult, Failure};
use crate::session::{Session, write_private_json};
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
struct PendingDemand {
    origin: String,
    member_id: String,
    key: String,
    body: Value,
    deadline: DateTime<Utc>,
    withdrawing: bool,
    failed: Option<String>,
}

impl PendingDemand {
    fn belongs_to(&self, api: &Api, session: &Session) -> bool {
        self.origin == api.origin() && self.member_id == session.member.id
    }

    fn receipt(&self) -> Value {
        let status = if self.withdrawing && self.failed.is_some() {
            "withdrawal_failed"
        } else if self.withdrawing {
            "withdrawing"
        } else if self.failed.is_some() {
            "failed"
        } else if self.deadline <= Utc::now() {
            "expired"
        } else {
            "pending"
        };
        json!({
            "idempotencyKey": self.key, "title": self.body["title"],
            "expiresAt": self.deadline, "status": status, "error": self.failed
        })
    }
}

pub struct Outbox {
    path: PathBuf,
    entries: Vec<PendingDemand>,
}

impl Outbox {
    pub fn open(dir: &Path) -> io::Result<Self> {
        let path = dir.join("demand-outbox.json");
        let entries = match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && meta.permissions().mode() & 0o777 == 0o600 => {
                serde_json::from_reader(File::open(&path)?)?
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "outbox must be a private regular file",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { path, entries })
    }

    fn edit(&mut self, change: impl FnOnce(&mut Vec<PendingDemand>)) -> ApiResult<()> {
        let mut entries = self.entries.clone();
        change(&mut entries);
        write_private_json(&self.path, &entries)
            .map_err(|_| Failure::new("outbox_io", "无法保存本机的待同步需求。"))?;
        self.entries = entries;
        Ok(())
    }

    pub fn list(&self, api: &Api, session: &Session) -> Value {
        json!({ "submissions": self.entries.iter().filter(|item| item.belongs_to(api, session))
            .map(PendingDemand::receipt).collect::<Vec<_>>() })
    }

    pub fn submit(&mut self, api: &Api, session: &Session, body: Value) -> ApiResult {
        let key = body["idempotencyKey"]
            .as_str()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(Failure::invalid_request)?;
        let index = match self
            .entries
            .iter()
            .position(|item| item.belongs_to(api, session) && item.key == key)
        {
            Some(index) => {
                let item = &self.entries[index];
                if item.withdrawing {
                    return Err(Failure::new(
                        "demand_submission_withdrawn",
                        "这次提交已申请撤回。",
                    ));
                }
                if item.body != body {
                    return Err(Failure::new(
                        "idempotency_conflict",
                        "重试必须使用原确认单。",
                    ));
                }
                if let Some(message) = &item.failed {
                    return Err(Failure::new("submission_failed", message));
                }
                index
            }
            None => {
                let deadline = demand_deadline(&body)?;
                if deadline <= Utc::now() {
                    return Err(expired());
                }
                let index = self.entries.len();
                let entry = PendingDemand {
                    origin: api.origin().into(),
                    member_id: session.member.id.clone(),
                    key: key.into(),
                    body,
                    deadline,
                    withdrawing: false,
                    failed: None,
                };
                self.edit(|entries| entries.push(entry))?;
                index
            }
        };
        match self.deliver(api, session, index, false) {
            Ok(Some(result)) => Ok(result),
            Ok(None) => Ok(json!({ "submission": self.entries[index].receipt() })),
            Err(error) => Err(error),
        }
    }

    pub fn withdraw(&mut self, api: &Api, session: &Session, key: &str) -> ApiResult {
        let index = self
            .entries
            .iter()
            .position(|item| item.belongs_to(api, session) && item.key == key);
        if let Some(index) = index
            && self.entries[index].failed.is_some()
            && !self.entries[index].withdrawing
        {
            self.edit(|entries| {
                entries.remove(index);
            })?;
            return Ok(json!({ "status": "discarded" }));
        }
        // The automatic retry may have completed after the user saw the pending entry.
        let index = index.unwrap_or(self.entries.len());
        self.edit(|entries| match entries.get_mut(index) {
            Some(item) => {
                item.withdrawing = true;
                item.failed = None;
            }
            None => entries.push(PendingDemand {
                origin: api.origin().into(),
                member_id: session.member.id.clone(),
                key: key.into(),
                body: Value::Null,
                deadline: Utc::now(),
                withdrawing: true,
                failed: None,
            }),
        })?;
        match self.deliver(api, session, index, false)? {
            Some(result) => Ok(result),
            None => Ok(json!({ "submission": self.entries[index].receipt() })),
        }
    }

    pub fn flush(&mut self, api: &Api, session: &Session) -> ApiResult<(bool, Vec<Value>)> {
        let mut changed = false;
        let mut delivered = Vec::new();
        for withdrawing in [true, false] {
            let mut index = 0;
            while index < self.entries.len() {
                let item = &self.entries[index];
                if !item.belongs_to(api, session)
                    || item.withdrawing != withdrawing
                    || item.failed.is_some()
                {
                    index += 1;
                    continue;
                }
                match self.deliver(api, session, index, true)? {
                    Some(result) => {
                        changed = true;
                        delivered.push(result);
                    }
                    None if self.entries[index].failed.is_some() => {
                        changed = true;
                        index += 1;
                    }
                    None => return Ok((changed, delivered)),
                }
            }
        }
        Ok((changed, delivered))
    }

    // A lost HTTP reply leaves the original queued confirmation available for idempotent delivery.
    fn deliver(
        &mut self,
        api: &Api,
        session: &Session,
        index: usize,
        background: bool,
    ) -> ApiResult<Option<Value>> {
        let item = &self.entries[index];
        let result = if item.withdrawing {
            api.withdraw_submission(&session.token, &item.key)
                .and_then(|result| {
                    let demand = result.get("demand").ok_or_else(Failure::invalid_response)?;
                    Ok(json!({ "demand": demand, "status": "withdrawn" }))
                })
        } else if item.deadline <= Utc::now() {
            Err(expired())
        } else {
            let mut body = item.body.clone();
            if body["expiresAt"].is_null() {
                body["expiresAt"] = json!(item.deadline);
            }
            api.submit_demand(&session.token, &body).and_then(|result| {
                let demand = result
                    .get("demand")
                    .filter(|demand| demand["id"].is_string())
                    .ok_or_else(Failure::invalid_response)?;
                let matches = result["matches"].as_array().cloned().unwrap_or_default();
                Ok(json!({ "demand": demand, "matches": matches }))
            })
        };
        match result {
            Ok(result) => {
                self.edit(|entries| {
                    entries.remove(index);
                })?;
                Ok(Some(result))
            }
            Err(error) if error.unauthorized => Err(error),
            Err(error) if error.retryable || error.code == "invalid_response" => Ok(None),
            Err(error) if background || item.withdrawing => {
                self.edit(|entries| entries[index].failed = Some(error.message))?;
                Ok(None)
            }
            Err(error) => {
                self.edit(|entries| {
                    entries.remove(index);
                })?;
                Err(error)
            }
        }
    }
}

fn demand_deadline(body: &Value) -> ApiResult<DateTime<Utc>> {
    let required = !body["matchAuthorization"].is_null();
    let timestamp = if required || !body["expiresAt"].is_null() {
        Some(&body["expiresAt"])
    } else {
        body["timeWindow"].get("end")
    };
    match timestamp {
        None => Ok(Utc::now() + TimeDelta::hours(6)),
        Some(value) => value
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|date| date.with_timezone(&Utc))
            .ok_or_else(Failure::invalid_request),
    }
}

fn expired() -> Failure {
    Failure::new(
        "demand_submission_expired",
        "确认单已过期，请重新确认后提交。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Member, tests::TestDir};

    #[test]
    fn restores_confirmed_work_and_withdrawal_for_the_same_account() {
        let dir = TestDir::new();
        let api = Api::new("http://127.0.0.1:1").unwrap();
        let session = |id: &str| Session {
            token: "test-session".into(),
            member: Member {
                id: id.into(),
                email: "test@example.test".into(),
                display_name: "Test".into(),
            },
        };
        let owner = session("owner");
        let body = json!({ "idempotencyKey": "confirmed", "title": "Exam", "expiresAt": Utc::now() + TimeDelta::hours(1) });
        let mut queue = Outbox::open(dir.path()).unwrap();
        assert_eq!(
            queue.submit(&api, &owner, body.clone()).unwrap()["submission"]["status"],
            "pending"
        );
        drop(queue);

        let mut queue = Outbox::open(dir.path()).unwrap();
        assert_eq!(
            queue.list(&api, &session("other"))["submissions"],
            json!([])
        );
        assert_eq!(
            queue.list(&api, &owner)["submissions"][0]["idempotencyKey"],
            "confirmed"
        );
        assert_eq!(
            queue.withdraw(&api, &owner, "confirmed").unwrap()["submission"]["status"],
            "withdrawing"
        );
        drop(queue);

        let mut queue = Outbox::open(dir.path()).unwrap();
        assert_eq!(
            queue.submit(&api, &owner, body).unwrap_err().code,
            "demand_submission_withdrawn"
        );
        let expired_body =
            json!({ "idempotencyKey": "expired", "expiresAt": Utc::now() - TimeDelta::hours(1) });
        assert_eq!(
            queue.submit(&api, &owner, expired_body).unwrap_err().code,
            "demand_submission_expired"
        );
    }
}
