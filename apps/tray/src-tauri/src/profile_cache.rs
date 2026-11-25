use crate::api::{Api, ApiResult, Failure};
use crate::session::{Session, read_private_json, write_private_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Default, Deserialize, Serialize)]
struct AccountProfile {
    confirmed: Option<Value>,
    draft: Option<Value>,
    edits: Vec<QueuedProfileEdit>,
}

#[derive(Clone, Deserialize, Serialize)]
struct QueuedProfileEdit {
    body: Value,
    error: Option<Failure>,
}

impl QueuedProfileEdit {
    fn key(&self) -> &str {
        self.body["mutationId"]
            .as_str()
            .expect("validated mutation ID")
    }

    fn deletes_field(&self) -> bool {
        let changes = self.body["changes"].as_object().expect("validated changes");
        changes.iter().any(|(field, change)| {
            let value = &change["value"];
            value.is_null()
                || value.as_str().is_some_and(|text| {
                    (field == "campus" && text.trim().is_empty())
                        || (field == "bio" && text.is_empty())
                })
        })
    }

    fn receipt(&self) -> Value {
        let status = match self.error.as_ref().map(|error| error.code.as_str()) {
            Some("profile_conflict") => "conflict",
            Some(_) => "failed",
            None => "pending",
        };
        json!({ "mutationId": self.key(), "status": status, "changes": self.body["changes"], "error": self.error })
    }
}

pub struct ProfileCache {
    path: PathBuf,
    accounts: BTreeMap<String, AccountProfile>,
}

impl ProfileCache {
    pub fn open(dir: &Path) -> io::Result<Self> {
        let path = dir.join("profiles.json");
        let accounts = read_private_json(&path)?.unwrap_or_default();
        Ok(Self { path, accounts })
    }

    fn edit<T>(
        &mut self,
        account: &str,
        change: impl FnOnce(&mut AccountProfile) -> T,
    ) -> ApiResult<T> {
        let mut accounts = self.accounts.clone();
        let result = change(accounts.entry(account.into()).or_default());
        write_private_json(&self.path, &accounts)
            .map_err(|_| Failure::new("profile_cache_io", "无法保存本机资料与同步状态。"))?;
        self.accounts = accounts;
        Ok(result)
    }

    pub fn get_profile(&mut self, api: &Api, session: &Session) -> ApiResult {
        let key = account_key(api, session);
        let cached = self
            .accounts
            .get(&key)
            .and_then(|account| account.confirmed.as_ref());
        match api.get_profile(&session.token) {
            Ok(profile) => {
                if profile.get("localProfile").is_none() || profile.get("cloudProfile").is_none() {
                    return Err(Failure::invalid_response());
                }
                if cached != Some(&profile) {
                    self.edit(&key, |account| account.confirmed = Some(profile.clone()))?;
                }
                Ok(profile)
            }
            Err(error) if error.retryable => {
                let mut profile = cached.cloned().ok_or(error)?;
                profile["profileSource"] = json!("cache");
                Ok(profile)
            }
            Err(error) => Err(error),
        }
    }

    pub fn legacy_sync(&mut self, api: &Api, session: &Session, body: &Value) -> ApiResult {
        let result = api.sync_local_profile(&session.token, body)?;
        let local = result
            .get("localProfile")
            .ok_or_else(Failure::invalid_response)?;
        self.edit(&account_key(api, session), |account| {
            observe_local(account, local)
        })?;
        Ok(result)
    }

    pub fn status(&self, api: &Api, session: &Session) -> Value {
        let key = account_key(api, session);
        let account = self.accounts.get(&key);
        let edits: Vec<Value> = account
            .into_iter()
            .flat_map(|account| &account.edits)
            .map(QueuedProfileEdit::receipt)
            .collect();
        json!({
            "confirmedProfileCached": account.is_some_and(|account| account.confirmed.is_some()),
            "draft": account.and_then(|account| account.draft.as_ref()), "edits": edits
        })
    }

    pub fn save_draft(&mut self, api: &Api, session: &Session, changes: Value) -> ApiResult {
        validate_changes(&changes)?;
        self.edit(&account_key(api, session), |account| {
            account.draft = Some(changes)
        })?;
        Ok(json!({ "status": "draft" }))
    }

    pub fn discard_draft(&mut self, api: &Api, session: &Session) -> ApiResult {
        let key = account_key(api, session);
        if self
            .accounts
            .get(&key)
            .is_some_and(|account| account.draft.is_some())
        {
            self.edit(&key, |account| account.draft = None)?;
        }
        Ok(json!({ "status": "discarded" }))
    }

    pub fn discard_failed_edit(&mut self, api: &Api, session: &Session, id: &str) -> ApiResult {
        let key = account_key(api, session);
        let index = self
            .accounts
            .get(&key)
            .and_then(|account| {
                account
                    .edits
                    .iter()
                    .position(|edit| edit.key() == id && edit.error.is_some())
            })
            .ok_or_else(|| {
                Failure::new("failed_edit_not_found", "找不到这条同步失败的资料改动。")
            })?;
        self.edit(&key, |account| account.edits.remove(index))?;
        Ok(json!({ "status": "discarded" }))
    }

    pub fn has_pending_delete(&self, api: &Api, session: &Session) -> bool {
        self.accounts
            .get(&account_key(api, session))
            .is_some_and(|account| {
                account
                    .edits
                    .iter()
                    .any(|item| item.error.is_none() && item.deletes_field())
            })
    }

    pub fn submit(
        &mut self,
        api: &Api,
        session: &Session,
        mut body: Value,
        wait_for_withdrawal: bool,
    ) -> ApiResult {
        let id = body["mutationId"]
            .as_str()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(Failure::invalid_request)?
            .to_owned();
        if body.as_object().is_none_or(|object| object.len() != 2) {
            return Err(Failure::invalid_request());
        }
        validate_changes(&body["changes"])?;
        body["mutationId"] = json!(id);
        let key = account_key(api, session);
        let index = match self
            .accounts
            .get(&key)
            .and_then(|account| account.edits.iter().position(|item| item.key() == id))
        {
            Some(index) => {
                let queued = &self.accounts[&key].edits[index];
                if queued.body != body {
                    return Err(Failure::new(
                        "idempotency_conflict",
                        "重试必须使用原确认的资料改动。",
                    ));
                }
                if let Some(error) = &queued.error {
                    return Err(error.clone());
                }
                index
            }
            None => self.edit(&key, move |account| {
                if account.draft.as_ref() == body.get("changes") {
                    account.draft = None;
                }
                let index = account.edits.len();
                account.edits.push(QueuedProfileEdit { body, error: None });
                index
            })?,
        };
        let edits = &self.accounts[&key].edits;
        let deleting = edits[index].deletes_field();
        let earlier_edit = edits.iter().enumerate().any(|(i, item)| {
            item.error.is_none()
                && ((i < index && item.deletes_field() == deleting)
                    || (item.deletes_field() && !deleting))
        });
        if !wait_for_withdrawal && !earlier_edit {
            if let Some(result) = self.deliver(api, session, &key, index, false)? {
                return Ok(result);
            }
        }
        Ok(json!({ "profileEdit": self.accounts[&key].edits[index].receipt() }))
    }

    pub fn flush(
        &mut self,
        api: &Api,
        session: &Session,
        deleting: bool,
    ) -> ApiResult<(bool, bool)> {
        let key = account_key(api, session);
        let mut changed = false;
        let mut index = 0;
        while let Some(item) = self
            .accounts
            .get(&key)
            .and_then(|account| account.edits.get(index))
        {
            if item.error.is_some() || item.deletes_field() != deleting {
                index += 1;
                continue;
            }
            if self.deliver(api, session, &key, index, true)?.is_some() {
                changed = true;
            } else if self.accounts[&key].edits[index].error.is_some() {
                changed = true;
                index += 1;
            } else {
                return Ok((changed, true));
            }
        }
        Ok((changed, false))
    }

    fn deliver(
        &mut self,
        api: &Api,
        session: &Session,
        account: &str,
        index: usize,
        background: bool,
    ) -> ApiResult<Option<Value>> {
        let item = &self.accounts[account].edits[index];
        match api.edit_profile_fields(&session.token, &item.body) {
            Ok(result) => {
                let Some(local) = result.get("localProfile").filter(|local| local.is_object())
                else {
                    return Ok(None);
                };
                self.edit(account, |account| {
                    observe_local(account, local);
                    account.edits.remove(index);
                })?;
                Ok(Some(result))
            }
            Err(error) if error.unauthorized => Err(error),
            Err(error) if error.retryable || error.code == "invalid_response" => Ok(None),
            Err(error) if background => {
                self.edit(account, |account| account.edits[index].error = Some(error))?;
                Ok(None)
            }
            Err(error) => {
                self.edit(account, |account| account.edits.remove(index))?;
                Err(error)
            }
        }
    }
}

fn account_key(api: &Api, session: &Session) -> String {
    format!("{}/{}", api.origin(), session.member.id)
}

fn observe_local(account: &mut AccountProfile, local: &Value) {
    let profile = account
        .confirmed
        .get_or_insert_with(|| json!({ "cloudProfile": null }));
    profile["localProfile"] = local.clone();
}

fn validate_changes(changes: &Value) -> ApiResult<()> {
    let fields = changes
        .as_object()
        .filter(|fields| !fields.is_empty())
        .ok_or_else(Failure::invalid_request)?;
    for (field, change) in fields {
        let text_field = matches!(field.as_str(), "campus" | "bio");
        let valid_field = text_field
            || field
                .strip_prefix("preferences.")
                .is_some_and(|key| !key.is_empty());
        let valid_value = change
            .get("value")
            .is_some_and(|value| !text_field || value.is_string() || value.is_null());
        if !valid_field
            || !valid_value
            || change["baseRevision"].as_u64().is_none()
            || change.as_object().is_none_or(|change| change.len() != 2)
        {
            return Err(Failure::invalid_request());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Member, tests::TestDir};

    #[test]
    fn confirmed_edit_survives_restart_without_replacing_cached_profile_or_crossing_accounts() {
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
        let cached = json!({ "localProfile": { "campus": "East" }, "cloudProfile": null });
        let changes = json!({ "campus": { "baseRevision": 1, "value": "West" } });
        let body = json!({ "mutationId": "change-campus", "changes": changes });
        let mut profiles = ProfileCache::open(dir.path()).unwrap();
        profiles
            .edit(&account_key(&api, &owner), |account| {
                account.confirmed = Some(cached.clone())
            })
            .unwrap();
        profiles.save_draft(&api, &owner, changes.clone()).unwrap();
        assert_eq!(profiles.status(&api, &owner)["edits"], json!([]));
        assert_eq!(
            profiles.submit(&api, &owner, body, true).unwrap()["profileEdit"]["status"],
            "pending"
        );
        drop(profiles);

        let mut profiles = ProfileCache::open(dir.path()).unwrap();
        let status = profiles.status(&api, &owner);
        assert!(status["draft"].is_null());
        assert_eq!(status["edits"][0]["changes"], changes);
        let fallback = profiles.get_profile(&api, &owner).unwrap();
        assert_eq!(fallback["profileSource"], "cache");
        assert_eq!(fallback["localProfile"], cached["localProfile"]);
        assert_eq!(
            profiles.status(&api, &session("other")),
            json!({ "confirmedProfileCached": false, "draft": null, "edits": [] })
        );
    }
}
