use crate::api::{Api, ApiResult, Event, EventBatch, Failure};
use crate::outbox::Outbox;
use crate::session::{Member, Session, SessionStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub demand_id: Option<String>,
    pub match_id: Option<String>,
    pub room_id: Option<String>,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    #[default]
    Demands,
    Detail,
    Room,
    Profile,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    id: u64,
    title: String,
    body: String,
    #[serde(flatten)]
    target: Target,
}

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DemandUnread {
    pub response_count: u64,
    pub room_message_count: u64,
}

#[derive(Default)]
struct Runtime {
    generation: u64,
    last_seq: u64,
    unread_by_demand: BTreeMap<String, DemandUnread>,
    unread_by_room: BTreeMap<String, u64>,
    room_to_demand: BTreeMap<String, String>,
    notice_id: u64,
    last_notice: Option<Notice>,
    active_view: View,
    target: Target,
    navigation_id: u64,
    seen_notices: HashSet<(String, String)>,
}

impl Runtime {
    fn reset(&mut self) {
        *self = Self {
            generation: self.generation + 1,
            notice_id: self.notice_id,
            navigation_id: self.navigation_id + 1,
            ..Self::default()
        };
    }

    fn total_unread(&self) -> u64 {
        self.unread_by_demand
            .values()
            .map(|demand| demand.response_count)
            .chain(self.unread_by_room.values().copied())
            .sum()
    }

    fn navigate(&mut self, target: Target) {
        self.active_view = if target.room_id.is_some() {
            View::Room
        } else if target.demand_id.is_some() {
            View::Detail
        } else {
            View::Demands
        };
        self.target = target;
        self.navigation_id += 1;
    }

    fn prune_demand_unread(&mut self, demand_id: &str) {
        if self
            .unread_by_demand
            .get(demand_id)
            .is_some_and(|unread| unread.response_count == 0 && unread.room_message_count == 0)
        {
            self.unread_by_demand.remove(demand_id);
        }
    }

    fn mark_demand_read(&mut self, demand_id: &str) -> bool {
        if let Some(unread) = self.unread_by_demand.get_mut(demand_id)
            && unread.response_count != 0
        {
            unread.response_count = 0;
            self.prune_demand_unread(demand_id);
            return true;
        }
        false
    }

    fn clear_withdrawn_demand(&mut self, demand_id: &str) {
        self.mark_demand_read(demand_id);
        if self.last_notice.as_ref().is_some_and(|notice| {
            notice.target.demand_id.as_deref() == Some(demand_id) && notice.target.room_id.is_none()
        }) {
            self.last_notice = None;
        }
    }

    fn remove_room_unread_from_demand(&mut self, demand_id: &str, count: u64) {
        if let Some(unread) = self.unread_by_demand.get_mut(demand_id) {
            unread.room_message_count -= count;
            self.prune_demand_unread(demand_id);
        }
    }

    fn cache_room_demand(&mut self, room_id: &str, demand_id: &str) {
        if self
            .room_to_demand
            .get(room_id)
            .is_some_and(|id| id == demand_id)
        {
            return;
        }
        let previous = self.room_to_demand.insert(room_id.into(), demand_id.into());
        if let Some(&count) = self.unread_by_room.get(room_id) {
            if let Some(previous) = previous {
                self.remove_room_unread_from_demand(&previous, count);
            }
            self.unread_by_demand
                .entry(demand_id.into())
                .or_default()
                .room_message_count += count;
        }
    }

    fn mark_room_read(&mut self, room_id: &str, demand_id: Option<&str>) -> bool {
        if let Some(demand_id) = demand_id {
            self.cache_room_demand(room_id, demand_id);
        }
        let Some(count) = self.unread_by_room.remove(room_id) else {
            return false;
        };
        if let Some(demand_id) = self.room_to_demand.get(room_id).cloned() {
            self.remove_room_unread_from_demand(&demand_id, count);
        }
        true
    }

    fn open_room(&mut self, room_id: String, demand_id: Option<String>) {
        self.mark_room_read(&room_id, demand_id.as_deref());
        self.navigate(Target {
            demand_id: demand_id.or_else(|| self.room_to_demand.get(&room_id).cloned()),
            room_id: Some(room_id),
            match_id: None,
        });
    }

    fn open_notification(&mut self, notice_id: u64) -> bool {
        let Some(notice) = &self.last_notice else {
            return false;
        };
        if notice.id != notice_id {
            return false;
        }
        let mut target = notice.target.clone();
        if let Some(room_id) = &target.room_id {
            self.mark_room_read(room_id, target.demand_id.as_deref());
            if target.demand_id.is_none() {
                target.demand_id = self.room_to_demand.get(room_id).cloned();
            }
        } else if let Some(demand_id) = &target.demand_id {
            self.mark_demand_read(demand_id);
        }
        self.navigate(target);
        true
    }

    fn notify(&mut self, title: &str, body: &str, target: Target) {
        self.notice_id += 1;
        self.last_notice = Some(Notice {
            id: self.notice_id,
            title: title.into(),
            body: body.into(),
            target,
        });
    }

    fn notify_response(&mut self, title: &str, body: &str, target: Target) {
        if let Some(id) = &target.demand_id {
            self.unread_by_demand
                .entry(id.clone())
                .or_default()
                .response_count += 1;
        }
        self.notify(title, body, target);
    }

    fn notify_match(&mut self, item: &Value, demand_id: Option<String>, member_id: &str) -> bool {
        let match_id = string_field(item, "id").or_else(|| string_field(item, "matchId"));
        if let Some(id) = &match_id
            && !self
                .seen_notices
                .insert(("match.proposed".into(), id.clone()))
        {
            return false;
        }
        let demand_id = demand_id.or_else(|| match_demand_id(item, member_id));
        let mut others = 0;
        let mut other_title = None;
        for participant in
            participants(item).filter(|p| p["memberId"].as_str().is_some_and(|id| id != member_id))
        {
            others += 1;
            other_title = other_title.or_else(|| string_field(participant, "demandTitle"));
        }
        let title = other_title.map_or_else(
            || "找到合适的搭子了".into(),
            |title| format!("{others} 位同学想一起：{title}"),
        );
        self.notify_response(
            &title,
            "同校区相容 · 全部同意后开 24 小时临时群",
            Target {
                demand_id,
                match_id,
                room_id: None,
            },
        );
        true
    }

    fn receive(&mut self, event: &Event, member_id: &str) -> bool {
        let data = &event.data;
        let demand_id = string_field(data, "demandId");
        match event.kind.as_str() {
            "demand.response" => {
                let Some(demand_id) = demand_id else {
                    return false;
                };
                let response = &data["response"];
                let title =
                    string_field(response, "title").unwrap_or_else(|| "需求收到新回应".into());
                let body = [
                    string_field(response, "source"),
                    string_field(response, "why").or_else(|| string_field(response, "summary")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                self.notify_response(
                    &title,
                    &body,
                    Target {
                        demand_id: Some(demand_id),
                        ..Target::default()
                    },
                );
            }
            "demand.updated" => {
                if data["demand"]["status"] == "cancelled"
                    && let Some(demand_id) = demand_id
                {
                    self.clear_withdrawn_demand(&demand_id);
                }
                return false;
            }
            "match.proposed" => {
                return self.notify_match(data.get("match").unwrap_or(data), demand_id, member_id);
            }
            "room.ready" => {
                let room = &data["room"];
                let Some(room_id) = event_room_id(data) else {
                    return false;
                };
                let demand_id = demand_id
                    .or_else(|| room_demand_id(room, &data["match"], member_id))
                    .or_else(|| self.room_to_demand.get(&room_id).cloned());
                if let Some(id) = &demand_id {
                    self.cache_room_demand(&room_id, id);
                }
                if !self
                    .seen_notices
                    .insert(("room.ready".into(), room_id.clone()))
                {
                    return false;
                }
                let room_title = string_field(room, "title").unwrap_or_else(|| "需求群聊".into());
                self.notify_response(
                    &format!("临时群已开好：{room_title}"),
                    "全员已同意，点击进入群聊（默认 24 小时）",
                    Target {
                        demand_id,
                        room_id: Some(room_id),
                        match_id: None,
                    },
                );
            }
            "room.message" => {
                let message = data.get("message").unwrap_or(data);
                let sender_id = message["senderId"]
                    .as_str()
                    .or_else(|| message["sender"]["id"].as_str())
                    .or_else(|| data["senderId"].as_str());
                if sender_id == Some(member_id) {
                    return false;
                }
                let Some(room_id) = event_room_id(data) else {
                    return false;
                };
                if let Some(id) = &demand_id {
                    self.cache_room_demand(&room_id, id);
                }
                let demand_id = demand_id.or_else(|| self.room_to_demand.get(&room_id).cloned());
                *self.unread_by_room.entry(room_id.clone()).or_default() += 1;
                if let Some(id) = &demand_id {
                    self.unread_by_demand
                        .entry(id.clone())
                        .or_default()
                        .room_message_count += 1;
                }
                let title = string_field(message, "senderName")
                    .or_else(|| string_field(data, "senderName"))
                    .map_or_else(
                        || "群聊有新回复".into(),
                        |name| format!("{name} 在群里说了句"),
                    );
                self.notify(
                    &title,
                    message["body"]
                        .as_str()
                        .or_else(|| message["text"].as_str())
                        .unwrap_or_default(),
                    Target {
                        demand_id,
                        room_id: Some(room_id),
                        match_id: None,
                    },
                );
            }
            "room.updated" => {
                if let Some(room_id) = event_room_id(data)
                    && let Some(id) = demand_id.or_else(|| string_field(&data["room"], "demandId"))
                {
                    self.cache_room_demand(&room_id, &id);
                }
                return false;
            }
            _ => return false,
        }
        true
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn participants(item: &Value) -> impl Iterator<Item = &Value> {
    item["participants"].as_array().into_iter().flatten()
}

fn match_demand_id(item: &Value, member_id: &str) -> Option<String> {
    string_field(item, "myDemandId").or_else(|| {
        participants(item)
            .find(|p| p["memberId"].as_str() == Some(member_id))
            .and_then(|p| string_field(p, "demandId"))
    })
}

fn room_demand_id(room: &Value, item: &Value, member_id: &str) -> Option<String> {
    string_field(room, "demandId").or_else(|| match_demand_id(item, member_id))
}

fn event_room_id(data: &Value) -> Option<String> {
    string_field(data, "roomId")
        .or_else(|| string_field(&data["room"], "id"))
        .or_else(|| string_field(&data["message"], "roomId"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    logged_in: bool,
    member: Option<Member>,
    api_origin: String,
    pub unread_count: u64,
    unread_by_demand: BTreeMap<String, DemandUnread>,
    unread_by_room: BTreeMap<String, u64>,
    active_view: View,
    #[serde(flatten)]
    target: Target,
    navigation_id: u64,
    pub notice: Option<Notice>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub events: Vec<Event>,
    pub new_notice: bool,
    pub refresh_lists: bool,
    #[serde(skip)]
    pub show_main: bool,
}

pub struct Hub {
    api: Api,
    session: SessionStore,
    outbox: Mutex<Outbox>,
    runtime: Mutex<Runtime>,
    changed: Condvar,
    publish: Box<dyn Fn(Update) + Send + Sync>,
}

impl Hub {
    pub fn new(
        api: Api,
        session: SessionStore,
        outbox: Outbox,
        publish: impl Fn(Update) + Send + Sync + 'static,
    ) -> Self {
        Self {
            api,
            session,
            outbox: Mutex::new(outbox),
            runtime: Mutex::default(),
            changed: Condvar::new(),
            publish: Box::new(publish),
        }
    }

    pub fn bootstrap(&self) -> Bootstrap {
        let runtime = self.runtime.lock().expect("hub state");
        let session = self.session.get();
        Bootstrap {
            logged_in: session.is_some(),
            member: session.map(|session| session.member),
            api_origin: self.api.origin().into(),
            unread_count: runtime.total_unread(),
            unread_by_demand: runtime.unread_by_demand.clone(),
            unread_by_room: runtime.unread_by_room.clone(),
            active_view: runtime.active_view,
            target: runtime.target.clone(),
            navigation_id: runtime.navigation_id,
            notice: runtime.last_notice.clone(),
        }
    }

    pub fn status(&self) -> Value {
        let runtime = self.runtime.lock().expect("hub state");
        let session = self.session.get();
        json!({
            "loggedIn": session.is_some(),
            "member": session.map(|session| session.member),
            "apiOrigin": self.api.origin(),
            "unreadCount": runtime.total_unread(),
            "unreadByDemand": runtime.unread_by_demand,
            "unreadByRoom": runtime.unread_by_room,
        })
    }

    pub fn request_code(&self, email: &str) -> ApiResult {
        self.api.request_code(email.trim())
    }

    pub fn verify_code(
        &self,
        email: &str,
        code: &str,
        display_name: Option<&str>,
    ) -> ApiResult<Member> {
        let generation = self.runtime.lock().expect("hub state").generation;
        let session = self.api.verify_code(
            email.trim(),
            code.trim(),
            display_name.map(str::trim).filter(|name| !name.is_empty()),
        )?;
        let mut runtime = self.runtime.lock().expect("hub state");
        if runtime.generation != generation || self.session.get().is_some() {
            return Err(Failure::new("session_changed", "登录状态已变化。"));
        }
        let member = session.member.clone();
        self.session
            .set(session)
            .map_err(|_| Failure::session_io())?;
        runtime.reset();
        self.changed.notify_all();
        (self.publish)(Update::default());
        Ok(member)
    }

    pub fn logout(&self) -> ApiResult {
        let session = {
            let mut runtime = self.runtime.lock().expect("hub state");
            let session = self.session.get();
            self.clear_session(&mut runtime)?;
            session
        };
        if let Some(session) = session
            && let Err(error) = self.api.logout(&session.token)
            && !error.unauthorized
        {
            return Err(error);
        }
        Ok(json!({}))
    }

    fn clear_session(&self, runtime: &mut Runtime) -> ApiResult<()> {
        self.session.clear().map_err(|_| Failure::session_io())?;
        runtime.reset();
        self.changed.notify_all();
        (self.publish)(Update::default());
        Ok(())
    }

    fn expire(&self, generation: u64) -> ApiResult<()> {
        let mut runtime = self.runtime.lock().expect("hub state");
        if runtime.generation == generation {
            self.clear_session(&mut runtime)?;
        }
        Ok(())
    }

    pub fn with_api(&self, call: impl FnOnce(&Api, &str) -> ApiResult) -> ApiResult {
        self.authenticated(call, |_, _, _| None)
    }

    fn authenticated<T>(
        &self,
        call: impl FnOnce(&Api, &str) -> ApiResult<T>,
        effect: impl FnOnce(&mut Runtime, &T, &Member) -> Option<Update>,
    ) -> ApiResult<T> {
        self.authenticated_session(|api, session| call(api, &session.token), effect)
    }

    fn authenticated_session<T>(
        &self,
        call: impl FnOnce(&Api, &Session) -> ApiResult<T>,
        effect: impl FnOnce(&mut Runtime, &T, &Member) -> Option<Update>,
    ) -> ApiResult<T> {
        let (generation, session) = {
            let runtime = self.runtime.lock().expect("hub state");
            let session = self.session.get().ok_or_else(Failure::login_required)?;
            (runtime.generation, session)
        };
        let result = call(&self.api, &session);
        let mut runtime = self.runtime.lock().expect("hub state");
        if runtime.generation != generation {
            return Err(Failure::new("session_changed", "登录状态已变化。"));
        }
        if result.as_ref().is_err_and(|error| error.unauthorized) {
            self.clear_session(&mut runtime)?;
            return Err(Failure::login_required());
        }
        let value = result?;
        if let Some(update) = effect(&mut runtime, &value, &session.member) {
            (self.publish)(update);
        }
        Ok(value)
    }

    pub fn submit_demand(&self, mut payload: Value) -> ApiResult {
        if payload["confirmed"] != true {
            return Err(Failure::confirmation_required());
        }
        payload
            .as_object_mut()
            .expect("confirmed field implies object")
            .remove("confirmed");
        let mut outbox = self.outbox.lock().expect("demand outbox");
        self.authenticated_session(
            |api, session| outbox.submit(api, session, payload),
            |runtime, value, member| Some(Self::submission_update(runtime, value, member)),
        )
    }

    pub fn list_submissions(&self) -> ApiResult {
        let outbox = self.outbox.lock().expect("demand outbox");
        self.authenticated_session(|api, session| Ok(outbox.list(api, session)), |_, _, _| None)
    }

    pub fn withdraw_submission(&self, key: &str) -> ApiResult {
        let mut outbox = self.outbox.lock().expect("demand outbox");
        self.authenticated_session(
            |api, session| outbox.withdraw(api, session, key),
            |runtime, value, member| Some(Self::submission_update(runtime, value, member)),
        )
    }

    fn profile_action(
        &self,
        notify: bool,
        call: impl FnOnce(&mut Outbox, &Api, &Session) -> ApiResult,
    ) -> ApiResult {
        let mut outbox = self.outbox.lock().expect("demand outbox");
        self.authenticated_session(
            |api, session| call(&mut outbox, api, session),
            |_, _, _| notify.then(Update::default),
        )
    }

    pub fn get_profile(&self) -> ApiResult {
        self.profile_action(false, |outbox, api, session| {
            outbox.profiles.get_profile(api, session)
        })
    }

    pub fn set_modeling_consent(&self, body: &Value) -> ApiResult {
        self.profile_action(true, |outbox, api, session| {
            outbox.profiles.set_modeling_consent(api, session, body)
        })
    }

    pub fn set_inference_control(&self, body: &Value) -> ApiResult {
        self.profile_action(true, |outbox, api, session| {
            outbox.profiles.set_inference_control(api, session, body)
        })
    }

    pub fn profile_sync_status(&self) -> ApiResult {
        self.profile_action(false, |outbox, api, session| {
            Ok(outbox.profiles.status(api, session))
        })
    }

    pub fn save_profile_draft(&self, changes: Value) -> ApiResult {
        self.profile_action(true, |outbox, api, session| {
            outbox.profiles.save_draft(api, session, changes)
        })
    }

    pub fn discard_profile_draft(&self) -> ApiResult {
        self.profile_action(true, |outbox, api, session| {
            outbox.profiles.discard_draft(api, session)
        })
    }

    pub fn discard_failed_profile_edit(&self, mutation_id: &str) -> ApiResult {
        self.profile_action(true, |outbox, api, session| {
            outbox.profiles.discard_failed_edit(api, session, mutation_id)
        })
    }

    pub fn edit_profile_fields(&self, body: Value) -> ApiResult {
        self.profile_action(true, |outbox, api, session| {
            let wait_for_withdrawal = outbox.has_pending_withdrawal(api, session);
            outbox.profiles.submit(api, session, body, wait_for_withdrawal)
        })
    }

    pub fn sync_local_profile(&self, body: &Value) -> ApiResult {
        self.profile_action(false, |outbox, api, session| {
            if outbox.has_pending_withdrawal(api, session)
                || outbox.profiles.has_pending_delete(api, session)
            {
                return Err(Failure::new(
                    "profile_sync_pending",
                    "请等待撤回与资料删除同步完成。",
                ));
            }
            outbox.profiles.legacy_sync(api, session, body)
        })
    }

    fn flush_submissions(&self) {
        let mut outbox = self.outbox.lock().expect("demand outbox");
        let _ = self.authenticated_session(
            |api, session| outbox.flush(api, session),
            |runtime, (changed, results), member| {
                if !changed {
                    return None;
                }
                let mut update = Update::default();
                for value in results {
                    let step = Self::submission_update(runtime, value, member);
                    update.new_notice |= step.new_notice;
                    update.refresh_lists |= step.refresh_lists;
                }
                Some(update)
            },
        );
    }

    fn submission_update(runtime: &mut Runtime, value: &Value, member: &Member) -> Update {
        let Some(id) = string_field(&value["demand"], "id") else {
            return Update::default();
        };
        if value["demand"]["status"] == "cancelled" {
            runtime.clear_withdrawn_demand(&id);
        }
        Self::demand_saved(runtime, value, Some(id), member)
    }

    pub fn update_demand(&self, id: &str, mut payload: Value) -> ApiResult {
        if payload["confirmed"] != true {
            return Err(Failure::confirmation_required());
        }
        let fields = payload
            .as_object_mut()
            .expect("confirmed field implies object");
        fields.remove("confirmed");
        fields.remove("demandId");
        self.authenticated(
            |api, token| api.update_demand(token, id, &payload),
            |runtime, value, member| Some(Self::demand_saved(runtime, value, Some(id.into()), member)),
        )
    }

    fn demand_saved(
        runtime: &mut Runtime,
        value: &Value,
        demand_id: Option<String>,
        member: &Member,
    ) -> Update {
        let mut new_notice = false;
        if let Some(matches) = value["matches"].as_array() {
            for item in matches {
                new_notice |= runtime.notify_match(item, demand_id.clone(), &member.id);
            }
        }
        Update {
            new_notice,
            refresh_lists: true,
            ..Update::default()
        }
    }

    pub fn respond_match(&self, id: &str, decision: &str) -> ApiResult {
        if !matches!(decision, "accept" | "decline") {
            return Err(Failure::invalid_request());
        }
        self.authenticated(
            |api, token| api.respond_match(token, id, decision),
            |runtime, result, member| {
                let room = &result["room"];
                let room_id = string_field(room, "id").or_else(|| string_field(room, "roomId"));
                let show_main = room_id.is_some();
                if let Some(room_id) = room_id {
                    let demand_id = string_field(result, "demandId")
                        .or_else(|| room_demand_id(room, &result["match"], &member.id));
                    runtime.open_room(room_id, demand_id);
                }
                Some(Update {
                    show_main,
                    refresh_lists: true,
                    ..Update::default()
                })
            },
        )
    }

    fn refresh_after(&self, call: impl FnOnce(&Api, &str) -> ApiResult) -> ApiResult {
        self.authenticated(call, |_, _, _| {
            Some(Update {
                refresh_lists: true,
                ..Update::default()
            })
        })
    }

    pub fn cancel_demand(&self, id: &str) -> ApiResult {
        self.authenticated(
            |api, token| api.cancel_demand(token, id),
            |runtime, value, _| {
                if value["demand"]["status"] == "cancelled" {
                    runtime.clear_withdrawn_demand(id);
                }
                Some(Update {
                    refresh_lists: true,
                    ..Update::default()
                })
            },
        )
    }

    pub fn complete_demand(&self, id: &str) -> ApiResult {
        self.refresh_after(|api, token| api.complete_demand(token, id))
    }

    pub fn feedback_response(
        &self,
        response_id: &str,
        feedback: &str,
        expected_updated_at: Option<&str>,
    ) -> ApiResult {
        self.refresh_after(|api, token| {
            api.feedback_response(token, response_id, feedback, expected_updated_at)
        })
    }

    pub fn vote_room_retention(&self, room_id: &str, keep: bool) -> ApiResult {
        self.refresh_after(|api, token| api.vote_room_retention(token, room_id, keep))
    }

    pub fn open_room(&self, room_id: String, demand_id: Option<String>) -> ApiResult {
        let mut runtime = self.runtime.lock().expect("hub state");
        if self.session.get().is_none() {
            return Err(Failure::login_required());
        }
        runtime.open_room(room_id.clone(), demand_id);
        (self.publish)(Update {
            show_main: true,
            ..Update::default()
        });
        Ok(json!({ "roomId": room_id }))
    }

    pub fn notify(&self, title: &str, body: &str, target: Target) -> ApiResult {
        let mut runtime = self.runtime.lock().expect("hub state");
        if self.session.get().is_none() {
            return Err(Failure::login_required());
        }
        runtime.notify(title, body, target);
        (self.publish)(Update {
            new_notice: true,
            ..Update::default()
        });
        Ok(json!({}))
    }

    pub fn open_notification(&self, notice_id: u64) {
        let mut runtime = self.runtime.lock().expect("hub state");
        if runtime.open_notification(notice_id) {
            (self.publish)(Update {
                show_main: true,
                ..Update::default()
            });
        }
    }

    pub fn mark_demand_read(&self, demand_id: &str) {
        let mut runtime = self.runtime.lock().expect("hub state");
        if runtime.mark_demand_read(demand_id) {
            (self.publish)(Update::default());
        }
    }

    pub fn mark_room_read(&self, room_id: &str, demand_id: Option<&str>) {
        let mut runtime = self.runtime.lock().expect("hub state");
        if self.session.get().is_some() && runtime.mark_room_read(room_id, demand_id) {
            (self.publish)(Update::default());
        }
    }

    pub fn clear_unread(&self) {
        let mut runtime = self.runtime.lock().expect("hub state");
        if runtime.total_unread() != 0 {
            runtime.unread_by_demand.clear();
            runtime.unread_by_room.clear();
            (self.publish)(Update::default());
        }
    }

    pub fn remember_view(
        &self,
        navigation_id: u64,
        view: View,
        demand_id: Option<String>,
        room_id: Option<String>,
    ) {
        let mut runtime = self.runtime.lock().expect("hub state");
        if runtime.navigation_id == navigation_id && self.session.get().is_some() {
            let (demand_id, room_id) = match view {
                View::Demands | View::Profile => (None, None),
                View::Detail => (demand_id, None),
                View::Room => {
                    let demand_id = demand_id.or_else(|| {
                        room_id
                            .as_ref()
                            .and_then(|id| runtime.room_to_demand.get(id).cloned())
                    });
                    (demand_id, room_id)
                }
            };
            runtime.active_view = view;
            runtime.target = Target {
                demand_id,
                match_id: None,
                room_id,
            };
        }
    }

    pub fn start_events(self: Arc<Self>) -> io::Result<()> {
        std::thread::Builder::new()
            .name("tray-events".into())
            .spawn(move || self.poll_events())?;
        Ok(())
    }

    fn poll_events(&self) {
        let mut validate_restored = true;
        loop {
            let (generation, after, session) = {
                let mut runtime = self.runtime.lock().expect("hub state");
                let session = loop {
                    if let Some(session) = self.session.get() {
                        break session;
                    }
                    runtime = self.changed.wait(runtime).expect("hub state");
                };
                (runtime.generation, runtime.last_seq, session)
            };
            if std::mem::take(&mut validate_restored) && generation == 0 {
                if let Err(error) = self.api.me(&session.token)
                    && error.unauthorized
                {
                    self.invalidate_polled_session(generation);
                    continue;
                }
                if self.runtime.lock().expect("hub state").generation != generation {
                    continue;
                }
            }
            self.flush_submissions();
            let started = Instant::now();
            match self.api.events(&session.token, after) {
                Ok(EventBatch {
                    mut events,
                    last_seq,
                }) => {
                    events.sort_by_key(|event| event.seq);
                    let mut revocations = events.iter()
                        .filter(|event| matches!(event.kind.as_str(), "profile.modeling.revoked" | "profile.inference.changed"))
                        .map(|event| (event.kind.as_str(), &event.data))
                        .peekable();
                    if revocations.peek().is_some() {
                        let updated = self.outbox.lock().expect("demand outbox")
                            .profiles.observe_profile_changes(&self.api, &session, revocations);
                        if updated.is_err() {
                            self.wait_to_retry(generation, Duration::from_secs(2));
                            continue;
                        }
                    }
                    let mut runtime = self.runtime.lock().expect("hub state");
                    if runtime.generation != generation {
                        continue;
                    }
                    let mut new_notice = false;
                    events.retain(|event| {
                        if event.seq <= runtime.last_seq {
                            return false;
                        }
                        runtime.last_seq = event.seq;
                        new_notice |= runtime.receive(event, &session.member.id);
                        true
                    });
                    runtime.last_seq = runtime.last_seq.max(last_seq);
                    if !events.is_empty() {
                        let refresh_lists = events.iter().any(|event| {
                            matches!(event.kind.as_str(), "room.updated" | "match.updated" | "demand.updated")
                        });
                        (self.publish)(Update {
                            events,
                            new_notice,
                            refresh_lists,
                            ..Update::default()
                        });
                        continue;
                    }
                    drop(runtime);
                    // Avoid spinning if a development server returns empty batches immediately.
                    self.wait_to_retry(
                        generation,
                        Duration::from_secs(1).saturating_sub(started.elapsed()),
                    );
                }
                Err(error) if error.unauthorized => self.invalidate_polled_session(generation),
                Err(_) => self.wait_to_retry(generation, Duration::from_secs(2)),
            }
        }
    }

    fn invalidate_polled_session(&self, generation: u64) {
        if self.expire(generation).is_err() {
            self.wait_to_retry(generation, Duration::from_secs(2));
        }
    }

    fn wait_to_retry(&self, generation: u64, timeout: Duration) {
        let runtime = self.runtime.lock().expect("hub state");
        drop(
            self.changed
                .wait_timeout_while(runtime, timeout, |runtime| runtime.generation == generation)
                .expect("hub state"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receive(runtime: &mut Runtime, kind: &str, data: Value) -> bool {
        runtime.receive(
            &Event {
                seq: 1,
                kind: kind.into(),
                data,
            },
            "me",
        )
    }

    fn response(runtime: &mut Runtime, demand_id: &str) {
        assert!(receive(
            runtime,
            "demand.response",
            json!({
                "demandId": demand_id,
                "response": {"title": "球场有空位", "source": "校园活动", "why": "时间合适"}
            }),
        ));
    }

    fn message(runtime: &mut Runtime, room_id: &str, demand_id: Option<&str>) {
        assert!(receive(
            runtime,
            "room.message",
            json!({
                "demandId": demand_id,
                "message": {"roomId": room_id, "senderId": "other", "senderName": "小林", "body": "南门见"}
            }),
        ));
    }

    fn unread(runtime: &Runtime) -> Value {
        json!({
            "total": runtime.total_unread(),
            "demands": runtime.unread_by_demand,
            "rooms": runtime.unread_by_room,
        })
    }

    #[test]
    fn room_message_only_adds_chat_unread_and_ignores_own_messages() {
        let mut runtime = Runtime::default();
        assert!(!receive(
            &mut runtime,
            "room.message",
            json!({"roomId": "r1", "demandId": "d1", "message": {"senderId": "me"}}),
        ));
        assert_eq!(runtime.total_unread(), 0);
        message(&mut runtime, "r1", Some("d1"));
        message(&mut runtime, "r1", None);
        assert_eq!(
            unread(&runtime),
            json!({"total": 2, "demands": {"d1": {"responseCount": 0, "roomMessageCount": 2}},
                   "rooms": {"r1": 2}})
        );
        let notice = runtime.last_notice.unwrap();
        assert_eq!(notice.title, "小林 在群里说了句");
        assert_eq!(notice.body, "南门见");
        assert_eq!(notice.target.demand_id.as_deref(), Some("d1"));
        assert_eq!(notice.target.room_id.as_deref(), Some("r1"));
    }

    #[test]
    fn demand_response_only_adds_response_unread_for_its_demand() {
        let mut runtime = Runtime::default();
        response(&mut runtime, "d1");
        assert_eq!(
            unread(&runtime),
            json!({"total": 1, "demands": {"d1": {"responseCount": 1, "roomMessageCount": 0}},
                   "rooms": {}})
        );
        let notice = runtime.last_notice.unwrap();
        assert_eq!(notice.title, "球场有空位");
        assert_eq!(notice.body, "校园活动 · 时间合适");
        assert_eq!(notice.target.demand_id.as_deref(), Some("d1"));
        assert!(notice.target.room_id.is_none());
    }

    #[test]
    fn withdrawal_update_clears_only_the_revoked_demands_response_unread() {
        let mut runtime = Runtime::default();
        response(&mut runtime, "d2");
        message(&mut runtime, "r1", Some("d1"));
        response(&mut runtime, "d1");

        assert!(!receive(
            &mut runtime,
            "demand.updated",
            json!({"demandId": "d1", "demand": {"status": "cancelled"}}),
        ));
        assert_eq!(
            unread(&runtime),
            json!({"total": 2, "demands": {
                "d1": {"responseCount": 0, "roomMessageCount": 1},
                "d2": {"responseCount": 1, "roomMessageCount": 0}
            }, "rooms": {"r1": 1}})
        );
        assert!(runtime.last_notice.is_none());
    }

    #[test]
    fn read_operations_preserve_other_unread_and_prune_empty_demands() {
        let mut runtime = Runtime::default();
        response(&mut runtime, "d1");
        response(&mut runtime, "d2");
        message(&mut runtime, "r1", Some("d1"));
        message(&mut runtime, "r2", Some("d1"));
        assert!(runtime.mark_room_read("r1", None));
        assert!(!runtime.mark_room_read("r1", Some("d1")));
        assert_eq!(
            unread(&runtime),
            json!({"total": 3, "demands": {"d1": {"responseCount": 1, "roomMessageCount": 1},
                                           "d2": {"responseCount": 1, "roomMessageCount": 0}},
                   "rooms": {"r2": 1}})
        );
        assert!(runtime.mark_demand_read("d1"));
        assert_eq!(
            unread(&runtime),
            json!({"total": 2, "demands": {"d1": {"responseCount": 0, "roomMessageCount": 1},
                                           "d2": {"responseCount": 1, "roomMessageCount": 0}},
                   "rooms": {"r2": 1}})
        );
        assert!(runtime.mark_room_read("r2", Some("d1")));
        assert!(!runtime.unread_by_demand.contains_key("d1"));
        assert_eq!(runtime.total_unread(), 1);
    }

    #[test]
    fn notification_opens_detail_or_room_and_only_clears_that_target() {
        let mut runtime = Runtime::default();
        assert!(matches!(runtime.active_view, View::Demands));
        response(&mut runtime, "d2");
        message(&mut runtime, "r2", Some("d2"));
        message(&mut runtime, "r1", Some("d1"));
        response(&mut runtime, "d1");

        assert!(!runtime.open_notification(runtime.notice_id - 1));
        assert!(runtime.open_notification(runtime.notice_id));
        assert!(matches!(runtime.active_view, View::Detail));
        assert_eq!(runtime.target.demand_id.as_deref(), Some("d1"));
        assert_eq!(runtime.navigation_id, 1);
        assert_eq!(runtime.total_unread(), 3);
        assert_eq!(runtime.unread_by_room["r1"], 1);
        assert_eq!(runtime.unread_by_demand["d2"].response_count, 1);

        response(&mut runtime, "d1");
        message(&mut runtime, "r1", None);
        assert!(runtime.open_notification(runtime.notice_id));
        assert!(matches!(runtime.active_view, View::Room));
        assert_eq!(runtime.target.room_id.as_deref(), Some("r1"));
        assert_eq!(runtime.target.demand_id.as_deref(), Some("d1"));
        assert_eq!(runtime.navigation_id, 2);
        assert_eq!(
            unread(&runtime),
            json!({"total": 3, "demands": {"d1": {"responseCount": 1, "roomMessageCount": 0},
                                           "d2": {"responseCount": 1, "roomMessageCount": 1}},
                   "rooms": {"r2": 1}})
        );
    }

    #[test]
    fn room_mapping_can_arrive_after_messages_and_open_room_clears_only_chat() {
        let mut runtime = Runtime::default();
        message(&mut runtime, "r1", None);
        assert!(runtime.unread_by_demand.is_empty());
        assert_eq!(runtime.total_unread(), 1);
        let ready = json!({"room": {"id": "r1", "demandId": "d1", "title": "晚饭"}});
        assert!(receive(&mut runtime, "room.ready", ready.clone()));
        assert!(!receive(&mut runtime, "room.ready", ready));
        assert_eq!(
            runtime.last_notice.as_ref().unwrap().title,
            "临时群已开好：晚饭"
        );
        assert_eq!(
            unread(&runtime),
            json!({"total": 2, "demands": {"d1": {"responseCount": 1, "roomMessageCount": 1}},
                   "rooms": {"r1": 1}})
        );

        runtime.open_room("r1".into(), None);
        assert!(matches!(runtime.active_view, View::Room));
        assert_eq!(runtime.target.demand_id.as_deref(), Some("d1"));
        assert_eq!(
            unread(&runtime),
            json!({"total": 1, "demands": {"d1": {"responseCount": 1, "roomMessageCount": 0}},
                   "rooms": {}})
        );
    }

    #[test]
    fn match_notice_counts_other_participants_and_deduplicates_submit_and_event() {
        let mut runtime = Runtime::default();
        let item = json!({
            "id": "m1", "myDemandId": "d1",
            "participants": [
                {"memberId": "me", "demandTitle": "自己的需求"},
                {"memberId": "other", "demandTitle": "南门晚饭"},
                {"memberId": "third"}
            ]
        });
        assert!(runtime.notify_match(&item, Some("d1".into()), "me"));
        assert!(!receive(
            &mut runtime,
            "match.proposed",
            json!({"match": item})
        ));
        assert_eq!(runtime.total_unread(), 1);
        let notice = runtime.last_notice.unwrap();
        assert_eq!(notice.title, "2 位同学想一起：南门晚饭");
        assert_eq!(notice.target.demand_id.as_deref(), Some("d1"));
        assert_eq!(notice.target.match_id.as_deref(), Some("m1"));
    }
}
