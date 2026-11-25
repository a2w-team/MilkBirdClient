# a2w Skill → 本地托盘 · Agent 工具协议 v1

本契约定义本地 Agent（通过 `skills/a2w/scripts/a2w_tray.py`）与常驻托盘（`MilkBirdClient`）之间的 Unix Domain Socket 协议。

## 1. 传输约定

- Socket 路径：`${MILKBIRD_TRAY_DIR:-$HOME/.milkbird}/tray.sock`
- 目录权限 `0700`、socket 权限 `0600`、本地凭据文件 `${MILKBIRD_TRAY_DIR:-$HOME/.milkbird}/session.json` 权限 `0600`。
- 每次短连接发送一行 UTF-8 JSON + `\n`，收到一行 JSON 回复 + `\n` 后关闭。请求与响应上限均为 **32 KiB**，包含换行符。
- 请求必须是仅含 `version`、`action` 和 `payload` 的 JSON 对象，`payload` 必须是对象。帧格式有误时返回 `invalid_request`。

## 2. 统一请求与响应格式

请求：
```json
{"version": 1, "action": "<action>", "payload": {}}
```

成功响应：
```json
{"ok": true, "action": "<action>", "data": {}}
```

错误响应：
```json
{"ok": false, "code": "not_logged_in", "message": "请先在 MilkBird Tray 用 hkust-gz.edu.cn 校园邮箱登录。"}
```
常见错误码包括 `invalid_request`、`unsupported_version`、`unknown_action`、`response_too_large`、`confirmation_required`、`not_logged_in` 和 `server_unavailable`。托盘未启动时，脚本会返回 `tray_unavailable`。
Hub 返回的 `active_demand_limit_reached`、`room_expired` 和 `demand_not_editable` 会保留原错误码。`demand_not_editable` 的托盘提示为「当前需求状态已不可修改。」

## 3. Agent 可调用的工具动作（Action 清单）

| Action | Payload 关键字段 | 行为与返回 `data` |
|---|---|---|
| `status` | `{}` | 返回 `{loggedIn, member, apiOrigin, unreadCount, unreadByDemand, unreadByRoom}`；不需要登录 |
| `submit_demand` | `{confirmed: true, idempotencyKey, category, title, summary?, campus, location?, timeWindow?, partySize?, preferences?, expiresAt?, matchAuthorization?}` | 登录并确认后保存原提交。后台确认时返回 `{demand, matches}`。临时不可达时返回 `{submission}`，托盘重连后用原键重发 |
| `list_submissions` | `{}` | 返回当前账号在本机的 `{submissions}`，离线可用 |
| `withdraw_submission` | `{idempotencyKey}` | 停止本机补发并用原键撤回。后台返回回执时返回 `{status: "withdrawn", demand}`。离线时返回 `{submission}`，可在重连后继续撤回。移除已经被拒绝的提交记录时返回 `{status: "discarded"}` |
| `update_demand` | `{confirmed: true, demandId, title?, summary?, campus?, location?, timeWindow?, partySize?, preferences?, expiresAt?, changeNote?}` | 校验确认和登录；将变更字段转发至 `PATCH /api/v1/demands/:id`，返回 `{demand, matches}`。成功后刷新列表，有匹配时触发托盘提醒 |
| `list_demands` | `{status?}` | 转发 `GET /api/v1/demands`，返回 `{demands}` |
| `get_demand` | `{demandId}` | 转发 `GET /api/v1/demands/:id`，返回 `{demand}` 及其授权状态 |
| `cancel_demand` | `{demandId}` | 转发 `POST /api/v1/demands/:id/cancel`，返回 `{demand}` |
| `complete_demand` | `{demandId}` | 转发 `POST /api/v1/demands/:id/complete`，返回 `{demand}`，释放活跃名额 |
| `list_demand_revisions` | `{demandId}` | 转发 `GET /api/v1/demands/:id/revisions`，返回 `{revisions}` |
| `list_demand_responses` | `{demandId}` | 转发 `GET /api/v1/demands/:id/responses`，返回 `{responses}` |
| `list_matches` | `{}` | 转发 `GET /api/v1/matches`，返回 `{matches}` |
| `respond_match` | `{matchId, decision: "accept" 或 "decline"}` | 转发 `POST /api/v1/matches/:id/respond`；若 `room` 创建成功，自动在托盘打开该群聊窗口；返回 `{match, room}` |
| `list_rooms` | `{}` | 转发 `GET /api/v1/rooms`，返回 `{rooms}` |
| `open_room` | `{roomId, demandId?}` | 打开指定群聊并清除该群的消息未读，返回 `{roomId}` |
| `vote_room_retention` | `{roomId, keep: boolean}` | 转发 `POST /api/v1/rooms/:id/retention-vote`，返回 Hub 的投票结果 |
| `notify` | `{title, body, demandId?, matchId?, roomId?}` | 显示轻通知。点击后优先打开 `roomId`，其次打开 `demandId` 详情，其余情况打开需求列表。未读数由 Hub 事件维护。返回 `{}` |
| `sync_local_profile` | `{campus?, preferences?, bio?}` | 转发 `PUT /api/v1/profile/local`，返回 `{localProfile}` |
| `edit_profile_fields` | `{confirmed: true, mutationId, changes}` | 保存已确认的字段编辑。后台确认后返回 `{localProfile, fieldChanges}`。等待同步时，返回 `{profileEdit}` |
| `get_profile` | `{}` | 读取后台资料并更新本机缓存。临时不可达时，已有的缓存附带 `profileSource: "cache"` |
| `set_modeling_consent` | `{confirmed: true, source, enabled, baseRevision}` | 转发 `PATCH /api/v1/profile/modeling`，返回 `{consent, profile, revokedDemandIds?}` 并更新本机确认缓存 |
| `profile_sync_status` | `{}` | 返回当前账号本机的 `{confirmedProfileCached, draft, edits}`，离线可用 |
| `save_profile_draft` | `{changes}` | 保存未确认草稿，返回 `{status: "draft"}` |
| `discard_profile_draft` | `{}` | 清除当前账号的草稿，返回 `{status: "discarded"}` |
| `discard_failed_profile_edit` | `{mutationId}` | 移除已被后台拒绝的本机编辑记录，返回 `{status: "discarded"}` |

`unreadByDemand` 以需求 ID 为键，值为 `{responseCount, roomMessageCount}`。`unreadByRoom` 以群聊 ID 为键，值为群消息未读数。`unreadCount` 等于所有 `responseCount` 加上所有群消息未读数。

脚本新增 `complete-demand --id …`、`list-responses --demand-id …` 和 `vote-retention --room-id … --keep|--no-keep`。`notify` 和 `open-room` 接受可选的 `--demand-id`。

使用 `update-demand --confirmed --id …` 修改现有需求，可用 `--change-note` 记录说明。`--start` 和 `--end` 成对传入并打包为 `timeWindow`。`--party-min` 和 `--party-max` 打包为 `partySize`，沿用 `submit-demand` 的缺省规则。使用 `list-revisions --demand-id …` 查询版本历史。

用 `submit-demand --single-match --expires-at … --idempotency-key …` 提交已确认的单次匹配授权。明确允许长期画像字段时，每项加 `--profile-field`，例如 `--profile-field preferences.food=2`，版本取自确认时 `get-profile` 的 `localProfile.fieldVersions[字段名].version`。明确允许推断标签时，每项加 `--inferred-tag`，并带上该次读取的 `cloudProfile.updatedAt` 作为 `--cloud-version`。用 `get-demand --id …` 查询原需求及其授权状态。

长期建模用 `set-modeling-consent --confirmed --source demands|interactions --enable|--disable --base-revision …`。先读取该来源的 `modeling[来源].revision`，按成员明确确认的设置提交。单次匹配回执不会改变长期授权。可选标签取自 `get-profile` 的 `cloudProfile.modelingCategories`。

关闭来源的回执可能包含 `revokedDemandIds`，即依赖该来源并被停止的在途需求。其他设备收到 `profile.modeling.revoked` 后按来源版本清除本机的相关推断，并从 `demand.updated` 更新需求状态。旧版本事件保持当前缓存的较新版本。

`submission` 包含 `idempotencyKey`、`title`、`expiresAt`、`status` 和可选错误消息 `error`。状态为 `pending`、`withdrawing`、`expired`、`failed` 或 `withdrawal_failed`。离线队列随托盘重启保留，按俱乐部地址和账号隔离。过期确认单停止提交。`list-submissions` 查看状态，`withdraw-submission --idempotency-key …` 撤回仍未收到需求 ID 的提交。

`sync-profile` 只提交本次确认保存的字段。`get-profile` 回执的 `localProfile.campus` 和 `campusVersion` 是后台的常驻校区及字段版本。

多设备编辑使用 `sync-profile-fields --confirmed --mutation-id … --changes-json …`。`changes` 按字段名填写 `{baseRevision, value}`。从 `localProfile.fieldVersions` 读取当前版本，版本缺省时用 `0`，删除时用 `value: null`。遇到 `profile_conflict` 时重新读取资料，将当前值和拟修改值交给成员选择。提交超时保留原编号和原字段编辑重试。

`save-profile-draft --changes-json …` 和 `discard-profile-draft` 只读写本机。确认提交与草稿内容一致的编辑后，草稿转入待同步队列。`profile-sync-status` 中的 `edits` 包含原 `mutationId`、`changes`、`status` 和 `error`，状态为 `pending`、`conflict` 或 `failed`。重连后先处理撤回和删除，再提交其余编辑。只有后台确认的值才会进入缓存。可以用 `discard-failed-profile-edit --mutation-id …` 移除失败记录。

## 4. 程序驱动 vs LLM 驱动配合规则

- **托盘后台事件驱动（无需 Agent 在场）**：登录后托盘自动长轮询 `GET /api/v1/events?after=<seq>&wait=25`。`demand.response`、`match.proposed` 和 `room.ready` 计入对应需求的回应未读，其他成员的 `room.message` 计入群聊消息未读。点击通知会打开对应页面并清除该目标的未读。
- **Agent 按需调用（显式 `/a2w` 会话内）**：Agent 通过 `skills/a2w/scripts/a2w_tray.py <subcommand>` 调用上述本地工具，只向用户展示极简人话回执。
