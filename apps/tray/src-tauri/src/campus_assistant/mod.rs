use crate::api::{ApiResult, Failure};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Service {
    Mail,
    Timetable,
    Swimming,
    MeetingRoom,
}

impl Service {
    const ALL: [Self; 4] = [Self::Mail, Self::Timetable, Self::Swimming, Self::MeetingRoom];

    fn parse(id: &str) -> ApiResult<Self> {
        match id {
            "mail" => Ok(Self::Mail),
            "timetable" => Ok(Self::Timetable),
            "swimming" => Ok(Self::Swimming),
            "meeting-room" => Ok(Self::MeetingRoom),
            _ => Err(invalid("请选择已支持的校园服务。")),
        }
    }

    fn info(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Mail => (
                "校园邮箱",
                "香港科技大学（广州）校园邮箱 · Microsoft Outlook",
                "https://outlook.office.com/mail/",
            ),
            Self::Timetable => (
                "个人课表",
                "香港科技大学（广州）教务系统 SIS",
                "https://sisn.hkust-gz.edu.cn/auth-sign",
            ),
            Self::Swimming => (
                "游泳馆预约",
                "香港科技大学（广州）科大 Go 场馆预约指引",
                "https://onestopservice.hkust-gz.edu.cn/zh/2023/07/27/gymnasium-reservation-guide/",
            ),
            Self::MeetingRoom => (
                "会议室预约",
                "香港科技大学（广州）Booking",
                "https://booking.hkust-gz.edu.cn/front_page",
            ),
        }
    }

    fn slots(self, date: &str) -> Vec<Slot> {
        let (id, title, location) = match self {
            Self::Swimming => ("swimming", "演示泳道时段", "演示游泳馆"),
            Self::MeetingRoom => ("meeting-room", "演示会议室时段", "演示会议室 A"),
            _ => return Vec::new(),
        };
        [("09:00", "10:00"), ("15:00", "16:00")]
            .into_iter()
            .enumerate()
            .map(|(index, (start, end))| Slot {
                id: format!("{id}-{date}-{}", index + 1),
                title,
                start: campus_time(date, start),
                end: campus_time(date, end),
                location,
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Account {
    Student,
    Staff,
}

impl Account {
    fn parse(payload: &Value) -> ApiResult<Self> {
        match payload.get("account") {
            None => Ok(Self::Student),
            Some(account) => match account.as_str() {
                Some("student") => Ok(Self::Student),
                Some("staff") => Ok(Self::Staff),
                _ => Err(invalid("请选择 student 或 staff 演示身份。")),
            },
        }
    }

    fn timetable_title(self) -> &'static str {
        match self {
            Self::Student => "演示课程 · 数据科学导论",
            Self::Staff => "演示课程 · 教学工作坊",
        }
    }
}

#[derive(Clone, Serialize)]
struct Slot {
    id: String,
    title: &'static str,
    start: String,
    end: String,
    location: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum BookingStatus {
    Reserved,
    Cancelled,
}

#[derive(Serialize)]
struct Booking {
    id: String,
    service: Service,
    slot: Slot,
    status: BookingStatus,
    #[serde(skip)]
    account: Account,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum Operation {
    Book,
    Cancel,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    id: String,
    operation: Operation,
    service: Service,
    slot: Slot,
    booking_id: String,
    status: BookingStatus,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Intent {
    id: String,
    operation: Operation,
    service: Service,
    slot: Slot,
    #[serde(skip_serializing_if = "Option::is_none")]
    booking_id: Option<String>,
    #[serde(skip)]
    account: Account,
    #[serde(skip)]
    receipt: Option<Receipt>,
}

#[derive(Default)]
struct DemoState {
    connections: HashMap<Service, Account>,
    bookings: Vec<Booking>,
    intents: HashMap<String, Intent>,
    next_id: u64,
}

impl DemoState {
    fn connected_account(&self, service: Service) -> ApiResult<Account> {
        self.connections.get(&service).copied().ok_or_else(authorization_required)
    }

    fn connect(&mut self, service: Service, account: Account) {
        if self.connections.insert(service, account) != Some(account) {
            self.intents.retain(|_, intent| intent.service != service);
        }
    }

    fn disconnect(&mut self, service: Service) {
        self.connections.remove(&service);
        self.intents.retain(|_, intent| intent.service != service);
    }

    fn next_id(&mut self, kind: &str) -> String {
        self.next_id += 1;
        format!("demo-{kind}-{}", self.next_id)
    }

    fn slot_available(&self, service: Service, slot_id: &str) -> bool {
        !self.bookings.iter().any(|booking| {
            booking.service == service
                && booking.slot.id == slot_id
                && booking.status == BookingStatus::Reserved
        })
    }

    fn owned_reservation(&self, service: Service, account: Account, id: &str) -> ApiResult<usize> {
        self.bookings.iter().position(|booking| {
            booking.id == id
                && booking.service == service
                && booking.account == account
                && booking.status == BookingStatus::Reserved
        }).ok_or_else(|| Failure::new("campus_booking_unavailable", "预约不存在或已取消。"))
    }

    fn prepare(
        &mut self,
        service: Service,
        account: Account,
        slot: Slot,
        booking_id: Option<String>,
    ) -> Value {
        let operation = if booking_id.is_some() { Operation::Cancel } else { Operation::Book };
        let intent = Intent {
            id: self.next_id("intent"),
            operation,
            service,
            slot,
            booking_id,
            account,
            receipt: None,
        };
        let response = json!({ "mode": "demo", "intent": intent });
        self.intents.insert(intent.id.clone(), intent);
        response
    }

    fn confirm(&mut self, id: &str) -> ApiResult<Receipt> {
        let intent = self.intents.get(id)
            .filter(|intent| self.connections.get(&intent.service) == Some(&intent.account))
            .ok_or_else(|| Failure::new("campus_intent_unavailable", "预约确认已失效，请重新查看。"))?
            .clone();
        if let Some(receipt) = intent.receipt {
            return Ok(receipt);
        }

        let (booking_id, status) = match &intent.booking_id {
            Some(booking_id) => {
                let index = self.owned_reservation(intent.service, intent.account, booking_id)?;
                self.bookings[index].status = BookingStatus::Cancelled;
                (booking_id.clone(), BookingStatus::Cancelled)
            }
            None => {
                if !self.slot_available(intent.service, &intent.slot.id) {
                    return Err(slot_conflict());
                }
                let booking_id = self.next_id("booking");
                self.bookings.push(Booking {
                    id: booking_id.clone(),
                    service: intent.service,
                    slot: intent.slot.clone(),
                    status: BookingStatus::Reserved,
                    account: intent.account,
                });
                (booking_id, BookingStatus::Reserved)
            }
        };
        let receipt = Receipt {
            id: intent.id,
            operation: intent.operation,
            service: intent.service,
            slot: intent.slot,
            booking_id,
            status,
        };
        self.intents.get_mut(id).expect("intent still exists").receipt = Some(receipt.clone());
        Ok(receipt)
    }
}

pub struct CampusAssistant {
    // Only the explicitly enabled demonstration can populate this local state.
    demo: bool,
    state: Mutex<DemoState>,
}

impl CampusAssistant {
    pub fn new(demo: bool) -> Self {
        Self { demo, state: Mutex::default() }
    }

    pub fn official_url(service: &str) -> ApiResult<&'static str> {
        Ok(Service::parse(service)?.info().2)
    }

    pub fn call(&self, action: &str, payload: &Value) -> ApiResult {
        let fields: &[&str] = match action {
            "status" => &[],
            "connect" => &["service", "account"],
            "disconnect" | "bookings" => &["service"],
            "mail" => &["unreadOnly"],
            "timetable" => &["date"],
            "availability" => &["service", "date"],
            "prepare-booking" => &["service", "date", "slotId"],
            "prepare-cancellation" => &["service", "bookingId"],
            "confirm" => &["intentId", "confirmed"],
            _ => return Err(invalid("未知的校园助理操作。")),
        };
        if !payload.as_object().is_some_and(|object| {
            object.keys().all(|key| fields.contains(&key.as_str()))
        }) {
            return Err(invalid("校园助理请求的参数格式无效。"));
        }

        let mut state = self.state.lock().expect("campus assistant state");
        match action {
            "status" => Ok(self.status(&state)),
            "connect" => {
                let service = Service::parse(required_text(payload, "service")?)?;
                let account = Account::parse(payload)?;
                if self.demo {
                    state.connect(service, account);
                }
                Ok(self.status(&state))
            }
            "disconnect" => {
                let service = Service::parse(required_text(payload, "service")?)?;
                state.disconnect(service);
                Ok(self.status(&state))
            }
            "mail" => {
                let unread_only = match payload.get("unreadOnly") {
                    Some(value) => value.as_bool().ok_or_else(|| invalid("请选择是否只看未读。"))?,
                    None => false,
                };
                let account = state.connected_account(Service::Mail)?;
                Ok(demo_mail(account, unread_only))
            }
            "timetable" => {
                let date = required_date(payload)?;
                let account = state.connected_account(Service::Timetable)?;
                Ok(json!({
                    "mode": "demo",
                    "observedAt": observed_at(),
                    "classes": [{
                        "id": format!("demo-class-{date}"),
                        "title": account.timetable_title(),
                        "start": campus_time(date, "10:00"),
                        "end": campus_time(date, "11:30"),
                        "location": "演示教学楼 101"
                    }]
                }))
            }
            "availability" | "bookings" | "prepare-booking" | "prepare-cancellation" => {
                let service = Service::parse(required_text(payload, "service")?)?;
                if !matches!(service, Service::Swimming | Service::MeetingRoom) {
                    return Err(invalid("请选择可预约场地的校园服务。"));
                }
                let account = state.connected_account(service)?;
                Self::booking_call(&mut state, action, payload, service, account)
            }
            "confirm" => {
                if payload["confirmed"] != true {
                    return Err(Failure::confirmation_required());
                }
                let id = required_text(payload, "intentId")?;
                if !self.demo {
                    return Err(authorization_required());
                }
                Ok(json!({ "mode": "demo", "receipt": state.confirm(id)? }))
            }
            _ => unreachable!("allowed action already checked"),
        }
    }

    fn status(&self, state: &DemoState) -> Value {
        let services: Vec<_> = Service::ALL.into_iter().map(|service| {
            let (name, official_source, official_url) = service.info();
            let source = if self.demo { "演示数据 · 未连接学校系统" } else { official_source };
            let account = state.connections.get(&service);
            let mut status = json!({
                "id": service,
                "name": name,
                "source": source,
                "officialUrl": official_url,
                "connected": account.is_some()
            });
            if let Some(account) = account {
                status["account"] = json!(account);
            }
            status
        }).collect();
        let mode = if self.demo { "demo" } else { "official" };
        json!({ "mode": mode, "services": services })
    }

    fn booking_call(
        state: &mut DemoState,
        action: &str,
        payload: &Value,
        service: Service,
        account: Account,
    ) -> ApiResult {
        match action {
            "availability" => {
                let date = required_date(payload)?;
                let slots: Vec<_> = service.slots(date).into_iter()
                    .filter(|slot| state.slot_available(service, &slot.id)).collect();
                Ok(json!({ "mode": "demo", "observedAt": observed_at(), "slots": slots }))
            }
            "bookings" => {
                let bookings: Vec<_> = state.bookings.iter()
                    .filter(|booking| booking.service == service && booking.account == account)
                    .collect();
                Ok(json!({ "mode": "demo", "bookings": bookings }))
            }
            "prepare-booking" => {
                let date = required_date(payload)?;
                let slot_id = required_text(payload, "slotId")?;
                let slot = service.slots(date).into_iter().find(|slot| slot.id == slot_id)
                    .ok_or_else(|| invalid("请选择当天的有效场地时段。"))?;
                if !state.slot_available(service, slot_id) {
                    return Err(slot_conflict());
                }
                Ok(state.prepare(service, account, slot, None))
            }
            "prepare-cancellation" => {
                let booking_id = required_text(payload, "bookingId")?;
                let index = state.owned_reservation(service, account, booking_id)?;
                let slot = state.bookings[index].slot.clone();
                Ok(state.prepare(service, account, slot, Some(booking_id.to_owned())))
            }
            _ => unreachable!("booking action already checked"),
        }
    }
}

fn demo_mail(account: Account, unread_only: bool) -> Value {
    let message = match account {
        Account::Student => (
            "demo-student-mail",
            "演示教务处 <student-affairs@example.test>",
            "演示 · 选课通知",
            "这是用来展示校园邮件列表的合成通知。",
        ),
        Account::Staff => (
            "demo-staff-mail",
            "演示教学办公室 <teaching-office@example.test>",
            "演示 · 教学安排",
            "这是用来展示教职员邮件列表的合成通知。",
        ),
    };
    let mut messages = vec![json!({
        "id": message.0, "from": message.1, "subject": message.2,
        "receivedAt": "2026-11-25T09:00:00+08:00", "unread": true, "preview": message.3
    })];
    if !unread_only {
        messages.push(json!({
            "id": "demo-read-mail", "from": "演示图书馆 <library@example.test>",
            "subject": "演示 · 图书馆开放时间", "receivedAt": "2026-11-24T14:00:00+08:00",
            "unread": false, "preview": "这是一封合成的已读邮件。"
        }));
    }
    json!({ "mode": "demo", "observedAt": observed_at(), "messages": messages })
}

fn campus_time(date: &str, hour_minute: &str) -> String {
    format!("{date}T{hour_minute}:00+08:00")
}

fn observed_at() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn required_text<'a>(payload: &'a Value, field: &str) -> ApiResult<&'a str> {
    payload[field].as_str().map(str::trim).filter(|text| !text.is_empty())
        .ok_or_else(|| invalid(&format!("请填写有效的 {field}。")))
}

fn required_date(payload: &Value) -> ApiResult<&str> {
    let date = required_text(payload, "date")?;
    let bytes = date.as_bytes();
    let invalid_date = || invalid("请输入有效日期，格式为 YYYY-MM-DD。");
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-'
        || !bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7) || byte.is_ascii_digit()
        })
    {
        return Err(invalid_date());
    }
    let year: u16 = date[..4].parse().map_err(|_| invalid_date())?;
    let month: u8 = date[5..7].parse().map_err(|_| invalid_date())?;
    let day: u8 = date[8..].parse().map_err(|_| invalid_date())?;
    let days = match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    if year == 0 || !(1..=days).contains(&day) {
        return Err(invalid_date());
    }
    Ok(date)
}

fn invalid(message: &str) -> Failure {
    Failure::new("campus_invalid_request", message)
}

fn authorization_required() -> Failure {
    Failure::new("campus_authorization_required", "当前未连接校园服务，请查看连接状态或打开学校官方入口。")
}

fn slot_conflict() -> Failure {
    Failure::new("campus_slot_conflict", "该演示时段已有预约，请改选其他时段。")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATE: &str = "2026-11-25";

    fn service<'a>(status: &'a Value, id: &str) -> &'a Value {
        status["services"].as_array().unwrap().iter()
            .find(|service| service["id"] == id).unwrap()
    }

    fn connect(assistant: &CampusAssistant, service: &str, account: &str) -> Value {
        assistant.call("connect", &json!({ "service": service, "account": account })).unwrap()
    }

    fn bookings(assistant: &CampusAssistant, service: &str) -> Value {
        assistant.call("bookings", &json!({ "service": service })).unwrap()
    }

    fn prepare_booking(assistant: &CampusAssistant, service: &str) -> Value {
        let available = assistant.call("availability", &json!({
            "service": service, "date": DATE
        })).unwrap();
        assistant.call("prepare-booking", &json!({
            "service": service, "date": DATE, "slotId": available["slots"][0]["id"]
        })).unwrap()["intent"].clone()
    }

    fn confirm(assistant: &CampusAssistant, intent: &Value) -> ApiResult {
        assistant.call("confirm", &json!({ "intentId": intent["id"], "confirmed": true }))
    }

    #[test]
    fn official_mode_exposes_only_verified_handoffs_and_never_receipts() {
        let assistant = CampusAssistant::new(false);
        let status = assistant.call("status", &json!({})).unwrap();
        assert_eq!(status["mode"], "official");
        for (id, url) in [
            ("mail", "https://outlook.office.com/mail/"),
            ("timetable", "https://sisn.hkust-gz.edu.cn/auth-sign"),
            ("swimming", "https://onestopservice.hkust-gz.edu.cn/zh/2023/07/27/gymnasium-reservation-guide/"),
            ("meeting-room", "https://booking.hkust-gz.edu.cn/front_page"),
        ] {
            assert_eq!(CampusAssistant::official_url(id).unwrap(), url);
            let entry = service(&status, id);
            assert_eq!(entry["officialUrl"], url);
            assert_eq!(entry["connected"], false);
            assert!(entry.get("account").is_none());
            assert_eq!(connect(&assistant, id, "student"), status);
        }
        for (action, payload) in [
            ("mail", json!({})),
            ("timetable", json!({ "date": DATE })),
            ("availability", json!({ "service": "swimming", "date": DATE })),
            ("bookings", json!({ "service": "meeting-room" })),
            ("prepare-booking", json!({ "service": "swimming", "date": DATE, "slotId": "one" })),
            ("prepare-cancellation", json!({ "service": "meeting-room", "bookingId": "one" })),
            ("confirm", json!({ "intentId": "one", "confirmed": true })),
        ] {
            assert_eq!(assistant.call(action, &payload).unwrap_err().code, "campus_authorization_required");
        }
    }

    #[test]
    fn each_demo_read_requires_its_own_connection_and_labels_the_result() {
        let assistant = CampusAssistant::new(true);
        let status = connect(&assistant, "mail", "staff");
        assert_eq!(status["mode"], "demo");
        assert_eq!(service(&status, "mail")["account"], "staff");
        assert_eq!(service(&status, "timetable")["connected"], false);
        assert_eq!(assistant.call("timetable", &json!({ "date": DATE })).unwrap_err().code,
                   "campus_authorization_required");
        let staff = assistant.call("mail", &json!({})).unwrap();
        assert_eq!(staff["mode"], "demo");
        assert!(staff["observedAt"].is_u64());
        assert_eq!(staff["messages"].as_array().unwrap().len(), 2);
        assistant.call("connect", &json!({ "service": "mail" })).unwrap();
        let student = assistant.call("mail", &json!({ "unreadOnly": true })).unwrap();
        assert_eq!(student["messages"].as_array().unwrap().len(), 1);
        assert_eq!(student["messages"][0]["unread"], true);
        assert_ne!(student["messages"][0]["id"], staff["messages"][0]["id"]);
        connect(&assistant, "timetable", "student");
        let schedule = assistant.call("timetable", &json!({ "date": DATE })).unwrap();
        assert_eq!(schedule["classes"][0]["start"], format!("{DATE}T10:00:00+08:00"));
        for id in ["swimming", "meeting-room"] {
            connect(&assistant, id, "student");
            let available = assistant.call("availability", &json!({ "service": id, "date": DATE })).unwrap();
            assert_eq!(available["mode"], "demo");
            assert_eq!(available["slots"][0]["id"], format!("{id}-{DATE}-1"));
            assert_eq!(available["slots"][0]["end"], format!("{DATE}T10:00:00+08:00"));
        }
    }

    #[test]
    fn booking_and_cancellation_require_confirmation_and_replay_exact_receipts() {
        let assistant = CampusAssistant::new(true);
        let id = "swimming";
        connect(&assistant, id, "student");
        let first = prepare_booking(&assistant, id);
        let competing = prepare_booking(&assistant, id);
        assert_eq!(first["slot"], competing["slot"]);
        assert_eq!(assistant.call("confirm", &json!({ "intentId": first["id"], "confirmed": false }))
            .unwrap_err().code, "confirmation_required");
        assert_eq!(bookings(&assistant, id)["bookings"], json!([]));

        let reserved = confirm(&assistant, &first).unwrap();
        assert_eq!(reserved["receipt"]["status"], "reserved");
        assert_eq!(confirm(&assistant, &first).unwrap(), reserved);
        assert_eq!(confirm(&assistant, &competing).unwrap_err().code, "campus_slot_conflict");
        assert_eq!(bookings(&assistant, id)["bookings"].as_array().unwrap().len(), 1);

        let cancellation = assistant.call("prepare-cancellation", &json!({
            "service": id, "bookingId": reserved["receipt"]["bookingId"]
        })).unwrap()["intent"].clone();
        assert_eq!(cancellation["operation"], "cancel");
        assert_eq!(bookings(&assistant, id)["bookings"][0]["status"], "reserved");
        let retired = confirm(&assistant, &cancellation).unwrap();
        assert_eq!(retired["receipt"]["status"], "cancelled");
        assert_eq!(confirm(&assistant, &cancellation).unwrap(), retired);
        assert_eq!(confirm(&assistant, &first).unwrap(), reserved);
        assert_eq!(bookings(&assistant, id)["bookings"][0]["status"], "cancelled");
        let replacement = prepare_booking(&assistant, id);
        assert_eq!(replacement["slot"], first["slot"]);
    }

    #[test]
    fn switching_or_disconnecting_retires_intents_and_keeps_history_account_scoped() {
        let assistant = CampusAssistant::new(true);
        let id = "meeting-room";
        connect(&assistant, "swimming", "staff");
        connect(&assistant, id, "student");
        let booked = confirm(&assistant, &prepare_booking(&assistant, id)).unwrap();
        let pending = prepare_booking(&assistant, id);
        let status = connect(&assistant, id, "staff");
        assert_eq!(service(&status, "swimming")["account"], "staff");
        assert_eq!(confirm(&assistant, &pending).unwrap_err().code, "campus_intent_unavailable");
        assert_eq!(bookings(&assistant, id)["bookings"], json!([]));
        assert_eq!(assistant.call("prepare-cancellation", &json!({
            "service": id, "bookingId": booked["receipt"]["bookingId"]
        })).unwrap_err().code, "campus_booking_unavailable");

        connect(&assistant, id, "student");
        assert_eq!(confirm(&assistant, &pending).unwrap_err().code, "campus_intent_unavailable");
        assert_eq!(bookings(&assistant, id)["bookings"][0]["status"], "reserved");
        let pending = prepare_booking(&assistant, id);
        let status = assistant.call("disconnect", &json!({ "service": id })).unwrap();
        assert_eq!(service(&status, id)["connected"], false);
        assert_eq!(service(&status, "swimming")["connected"], true);
        connect(&assistant, id, "student");
        assert_eq!(confirm(&assistant, &pending).unwrap_err().code, "campus_intent_unavailable");
    }

    #[test]
    fn unsupported_input_and_invalid_dates_return_meaningful_errors() {
        let assistant = CampusAssistant::new(true);
        connect(&assistant, "swimming", "student");
        for (action, payload) in [
            ("unknown", json!({})),
            ("connect", json!({ "service": "library" })),
            ("connect", json!({ "service": "mail", "account": "other" })),
            ("connect", json!({ "service": "mail", "password": "synthetic" })),
            ("mail", json!({ "unreadOnly": "true" })),
            ("availability", json!({ "service": "mail", "date": DATE })),
            ("availability", json!({ "service": "swimming", "date": "2026-02-29" })),
            ("availability", json!({ "service": "swimming", "date": "1900-02-29" })),
            ("availability", json!({ "service": "swimming", "date": "2026-11-31" })),
            ("prepare-booking", json!({ "service": "swimming", "date": DATE, "slotId": "missing" })),
        ] {
            let error = assistant.call(action, &payload).unwrap_err();
            assert_eq!(error.code, "campus_invalid_request");
            assert!(!error.message.is_empty());
        }
        assert!(assistant.call("availability", &json!({
            "service": "swimming", "date": "2000-02-29"
        })).is_ok());
    }
}
