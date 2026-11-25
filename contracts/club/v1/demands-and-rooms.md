# 需求挂起、匹配、通知、临时群聊与双画像 · 后台 HTTP v1

除另有说明外，以下全部接口要求 `Authorization: Bearer <token>`，JSON 请求体上限 32 KiB，时间统一使用 RFC 3339。

---

## 1. 双人物画像

### 1.1 同步本地画像（用户 Agent 维护 → 云端）
`PUT /api/v1/profile/local`

```json
{
  "campus": "东校区",
  "preferences": {"food": ["火锅", "轻食"], "activeHours": "晚上"},
  "bio": "大三计算机，喜欢约饭和打球",
  "updatedBy": "local_agent"
}
```
响应 `200 OK`：
```json
{
  "localProfile": {
    "campus": "东校区",
    "preferences": {"food": ["火锅", "轻食"], "activeHours": "晚上"},
    "bio": "大三计算机，喜欢约饭和打球",
    "updatedBy": "local_agent",
    "updatedAt": "2026-11-23T12:00:00Z"
  }
}
```

### 1.2 查询双画像（本地画像 + 平台云端画像）
`GET /api/v1/profile`

响应 `200 OK`：
```json
{
  "localProfile": { "...": "..." },
  "cloudProfile": {
    "demandCount": 3,
    "acceptedMatchCount": 2,
    "roomMessageCount": 14,
    "recentCategories": ["dining", "sports"],
    "summary": "近期在东校区活跃，常发起 dining 需求，参与临时群聊互动 14 条。",
    "updatedAt": "2026-11-23T12:05:00Z"
  }
}
```
> 平台在成员提交需求、响应匹配、在临时群发消息时自动增量更新 `cloudProfile`（首版规则归纳，后续可替换为 LLM 摘要）。

---

## 2. 需求卡片与挂起

### 2.1 提交已确认的需求卡片
`POST /api/v1/demands`

```json
{
  "idempotencyKey": "req-uuid-1",
  "category": "dining",
  "title": "今晚想找2-3人吃火锅",
  "summary": "19:00 左右东校区南门，人均 80 以内",
  "campus": "东校区",
  "location": "南门",
  "timeWindow": {
    "start": "2026-11-23T18:30:00Z",
    "end": "2026-11-23T21:00:00Z"
  },
  "partySize": {"min": 2, "max": 4},
  "preferences": {"food": "火锅", "budget": "80以内"},
  "expiresAt": "2026-11-23T21:00:00Z"
}
```
- 必填：`idempotencyKey`、`category`（非空，如 `dining` / `sports` / `meetup` / `study` / `other`）、`title`、`campus`。
- 幂等：同一成员使用相同 `idempotencyKey` 重复提交，返回原有 `demand` 与当前关联 `matches`。
- 每位成员最多同时挂起 3 个活跃需求。新增需求超过上限时返回 `409 active_demand_limit_reached`。完成或撤回活跃需求可释放名额。
- 新需求落库为 `status: "open"`，立即执行规则匹配：
  - 筛选**其他成员**的 `open` 且未过期需求；
  - `category` 相同（忽略大小写）且 `campus` 相同；
  - 若双方均提供 `timeWindow`，要求时间区间有交集（`a.start < b.end && b.start < a.end`）；
  - 匹配可包含多位参与者，人数约束由 `partySize` 表示；
  - 参与者之间当前不存在未结束（`pending` / `room_ready`）的重复匹配；
  - 命中后创建 `match`（`status: "pending"`），向参与者写入 `match.proposed` 实时事件。

响应 `200 OK`：
```json
{
  "demand": {
    "id": "dem_...",
    "memberId": "mem_...",
    "category": "dining",
    "title": "今晚想找2-3人吃火锅",
    "summary": "19:00 左右东校区南门，人均 80 以内",
    "campus": "东校区",
    "location": "南门",
    "timeWindow": {"start": "2026-11-23T18:30:00Z", "end": "2026-11-23T21:00:00Z"},
    "partySize": {"min": 2, "max": 4},
    "preferences": {"food": "火锅", "budget": "80以内"},
    "status": "open",
    "expiresAt": "2026-11-23T21:00:00Z",
    "createdAt": "2026-11-23T12:00:00Z"
  },
  "matches": []
}
```

### 2.2 查询、完成与撤回本人需求
- `GET /api/v1/demands`（可选 `?status=open`）→ `200 {"demands": [...]}`
- `GET /api/v1/demands/:id` → `200 {"demand": {...}}`，含单次匹配授权状态。仅本人可查询。
- `POST /api/v1/demands/:id/complete` → 完成自己的需求，释放活跃名额，返回 `200 {"demand": {...}}`。
- `POST /api/v1/demands/:id/cancel` → 仅本人可操作；将需求置为 `cancelled`，同时将关联 `pending` 匹配置为 `cancelled` 并通知各方；返回 `200 {"demand": {...}}`。
- `POST /api/v1/demands/withdraw-submission` 接受原提交的 `{idempotencyKey}`。已创建需求时按同样规则撤回并返回 `{demand}`。需求尚未创建时返回 `{demand: null}` 并记录撤回。当前账号随后使用该键的新建提交返回 `409 demand_submission_withdrawn`。

提交时带上 `matchAuthorization: {"purpose":"single_match"}` 和经确认的未来 `expiresAt`，签发绑定本次需求版本的授权。`profileFields` 将用户明确选中的长期字段映射到 `localProfile.fieldVersions[字段名].version`，例如 `{"preferences.food":2}`。用户明确选中的推断标签用 `inferredTags`，并带上 `cloudProfile.updatedAt` 作为 `cloudVersion`。两个来源均通过 `GET /api/v1/profile` 读取。授权回执 `demand.matchAuthorization` 含 `grantId`、`demandVersion`、`purpose`、`expiresAt` 和 `status`（`active`、`revoked`、`expired`、`closed`）。所选字段被改动或删除，或所选标签的来源版本变化时，返回 `409 match_profile_changed`。同一幂等键提交不同确认内容返回 `409 idempotency_conflict`。撤回后保持原需求 ID、原授权编号，回执显示 `revoked`。

### 2.3 修改需求与查询版本历史

- `PATCH /api/v1/demands/:id` → 更新本人的进行中需求。只提交需要变更的字段，可用字段为 `title`、`summary`、`campus`、`location`、`timeWindow`、`partySize`、`preferences` 和 `expiresAt`，可附 `changeNote` 记录变更说明。时间区间用 `{start, end}`，人数范围用 `{min, max}`。成功返回 `200 {"demand": {...}, "matches": [...]}`。需求状态已不可修改时返回 `409 demand_not_editable`。
- `GET /api/v1/demands/:id/revisions` → 查询指定需求的版本历史，返回 `200 {"revisions": [...]}`。

已经签发单次匹配授权的需求修改时返回 `409 demand_not_editable`。请先撤回，再用新确认单提交。

### 2.4 需求回应与反馈

- `GET /api/v1/demands/:id/responses` → `200 {"responses": [...]}`。托盘识别 `kind` 为 `link`、`richtext` 或 `html_card` 的回应，分别使用 `url`、`content` 或 `html` 展示内容。通知使用 `title`、`source` 和 `why`（缺省时读取 `summary`）。
- 已撤回或已过期的开放需求返回 `409 demand_not_open`，托盘清除该需求缓存的回应。事件流可跳过失效记录，托盘使用回执 `lastSeq` 推进游标。
- `POST /api/v1/responses/:id/feedback` → 提交对指定回应的反馈。托盘发送 `{"feedback": "useful"}` 标记有用、`{"feedback": "dismissed"}` 标记不合适，或发送 `{"feedback": ""}` 撤销标记。

---

## 3. 匹配回应与幂等建群

### 3.1 查询本人相关匹配
`GET /api/v1/matches` → `200 {"matches": [...]}`

单个 `match` 对象结构：
```json
{
  "id": "mat_...",
  "status": "pending",
  "reason": "同在东校区，都想 dining，时间区间重合",
  "roomId": "",
  "myDecision": "pending",
  "myDemandId": "dem_1",
  "participants": [
    {
      "memberId": "mem_1",
      "displayName": "Alice",
      "demandId": "dem_1",
      "demandTitle": "今晚想找2-3人吃火锅",
      "decision": "pending"
    },
    {
      "memberId": "mem_2",
      "displayName": "Bob",
      "demandId": "dem_2",
      "demandTitle": "东校区南门约晚饭",
      "decision": "pending"
    }
  ],
  "createdAt": "2026-11-23T12:01:00Z",
  "updatedAt": "2026-11-23T12:01:00Z"
}
```
`status` 取值：`pending`（等待各方同意）、`room_ready`（已建群）、`declined`（任一方拒绝）、`cancelled`（需求被撤回或过期）。

### 3.2 同意或跳过匹配（触发幂等建群）
`POST /api/v1/matches/:id/respond`

请求体：
```json
{"decision": "accept"}
```
（取值：`accept` 或 `decline`）

逻辑：
- 仅该 match 的参与成员可调用，否则 `403`。
- 若 `decision == "decline"`：将 match 置为 `declined`，向所有参与方发 `match.updated` 事件；不建群。
- 若 `decision == "accept"`：幂等记录当前成员同意；
  - 若尚有成员未同意：保持 `pending`，推送 `match.updated`；
  - 若**所有参与成员均已同意**：在同一数据库事务中以 `match_id` 唯一约束**幂等创建一个临时群**，将 match 置为 `room_ready` 并填 `roomId`，将相关需求置为 `matched`，向所有参与方推送 `room.ready` 事件！

响应 `200 OK`：
```json
{"match": {"...": "..."}, "room": {"...": "..."} }
```
（未建群时 `room` 为 `null`）

---

## 4. 实时事件流与断线补拉（长轮询）

`GET /api/v1/events?after=<seq>&wait=<0..25>`

- 返回当前成员 `seq > after` 的事件，按 `seq` 升序排列。
- 若暂无新事件且 `wait > 0`（服务端上限 25 秒），服务端阻塞等待直到有新事件写入或超时。
- 事件类型：
  - `demand.response`：payload 包含 `demandId` 与 `response`
  - `match.proposed`：payload 包含 `match`，可带 `demandId`，也可从 `match.myDemandId` 识别接收者的需求
  - `match.updated`：payload 包含 `match`
  - `room.ready`：payload 包含 `room`，需求归属取 `demandId` 或 `room.demandId`，可附带 `match`
  - `room.message`：payload 包含 `roomId` 与 `message`，可带 `demandId`；托盘也可根据既有群聊归属关联需求
  - `room.updated`：群聊状态变化，托盘刷新列表
  - `demand.updated`：payload 包含 `demandId` 和 `demand`。收到已撤回的状态时刷新需求，并清除该需求的回应未读。

响应 `200 OK`：
```json
{
  "events": [
    {
      "seq": 1,
      "type": "match.proposed",
      "createdAt": "2026-11-23T12:01:00Z",
      "payload": {"demandId": "dem_1", "match": {"...": "..."}}
    }
  ],
  "lastSeq": 1
}
```

---

## 5. 临时群聊

临时群默认有效 24 小时。群成员可以投票决定是否保留群聊，投票状态和保留结果以 Hub 返回为准。群聊到期后，受限的操作可返回 `409 room_expired`。

- `GET /api/v1/rooms`：列出当前成员参与的群聊 → `200 {"rooms": [...]}`
  - 单个 `room` 含 `id`、`matchId`、`title`、`status`（`active` 或 `closed`）、`members`（`[{memberId, displayName, joinedAt}]`）、`lastMessage`（`{...}` 或 `null`）、`expiresAt`、`createdAt`。托盘读取 `demandId` 关联当前成员的需求，并使用 `retentionStatus` 和 `retention` 展示保留状态与票数。
- `GET /api/v1/rooms/:id/messages?after=<seq>`：仅当前活跃群成员可读取（非成员或已退群返回 `403`）→ `200 {"messages": [...], "lastSeq": <int64>}`
- `POST /api/v1/rooms/:id/messages`
  - 请求：`{"clientMsgId": "uuid", "body": "今晚7点南门见！"}`（`body` 去空白后 1–2000 字符；同一 `(room_id, sender_id, client_msg_id)` 幂等返回同一消息）
  - 响应 `200 OK`：`{"message": {"id":"msg_...","seq":1,"roomId":"room_...","senderId":"mem_...","senderName":"Alice","clientMsgId":"uuid","body":"...","createdAt":"..."}}`
  - 写入后向群内所有活跃成员推送 `room.message` 事件，并增量更新发送者 `cloudProfile`。
- `POST /api/v1/rooms/:id/retention-vote`：提交保留投票，请求体为 `{"keep": true}` 或 `{"keep": false}`。
- `POST /api/v1/rooms/:id/leave`：退出群聊，撤销该成员后续读写权限；返回 `200 {"ok": true}`。
