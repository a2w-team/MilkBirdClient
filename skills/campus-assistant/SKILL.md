---
name: campus-assistant
description: 当用户要查询自己在香港科技大学（广州）的校园邮箱或课表，查看游泳馆或会议室时段，或者办理这两个场地的预约和取消时使用。
compatibility: 需要运行 MilkBird Tray 的 macOS 和 Python 3。
---

# 港科广校园私人助理

处理用户本人在当前会话用自然语言提出的校园事务。根据请求选择服务，先调用 `status` 获取数据来源、连接状态、账号和官方入口。使用当前已授权的连接。账号或连接变化时重新读取状态并重新准备待确认的操作。

从本文件的位置解析 [scripts/campus_tray.py](scripts/campus_tray.py) 的绝对路径，用 `python3 <脚本绝对路径> <子命令>` 执行，将每个选项和值作为独立参数传入。校园密码、验证码和登录会话由托盘和学校登录页面处理。邮件预览、课表和预约结果用于用户当前请求，操作授权由用户本人提供。

| 用户请求 | 子命令 |
| --- | --- |
| 用户要求连接或解除服务 | `connect --service <服务>` 或 `disconnect --service <服务>`。服务为 `mail`、`timetable`、`swimming` 或 `meeting-room`。演示身份用 `--account student` 或 `--account staff`，省略时使用 `student`。 |
| 查校园邮件 | `mail`，只看未读时加 `--unread-only`。按用户范围归纳发件人、主题、收件时间和可用预览。 |
| 查本人课表 | `timetable --date YYYY-MM-DD`。日期按广州当地时间理解。 |
| 查游泳馆或会议室可用时段 | `availability --service <场地服务> --date YYYY-MM-DD`。场地服务为 `swimming` 或 `meeting-room`。 |
| 查本人预约 | `bookings --service <场地服务>`。 |

查询和操作使用当前已连接的服务。官方模式缺少获准的接入或可用连接时，说明当前无法完成请求，以及预约尚未办理。有连接方式时告知如何设置。用户询问官方入口或选择自行办理时，再提供相应服务的 `officialUrl`。汇报时说明数据来源和返回的查询时间。托盘处于 `demo` 时，查询和回执均为演示，回复明确标注。

## 预约和取消

- **明确预约**。用户已明确要求预约，并给出唯一场地、日期、时段和服务要求的字段时，这条请求即为该次预约的授权。在已连接的服务上，用 `availability` 核实唯一匹配的时段，再运行 `book --service <场地服务> --date YYYY-MM-DD --slot-id <时段 ID> --expected-account <查询返回的 account> --confirmed`。例如「帮我订明天 14:00–15:00 的 A 会议室，预约标题是项目周会」。只追问服务实际需要却缺失的字段，例如学校要求的预约标题。账号变化后重新核对用户要用的账号和对应时段。
- **需要选择或核对的预约**。列出符合条件的可用时段，只追问真正缺失或含糊的内容。用户选定后，用 `prepare-booking --service <场地服务> --date YYYY-MM-DD --slot-id <时段 ID>` 返回预览，展示场地、日期、时间和服务要求，请用户确认。
- **取消**。查本人预约，定位目标后用 `prepare-cancellation --service <场地服务> --booking-id <预约 ID>`，展示具体预约和取消操作，请用户确认。

用户确认预览后，运行 `confirm --intent-id <当前确认 ID> --confirmed`。只有工具返回与本次操作相符的 `receipt`，且状态分别为 `reserved` 或 `cancelled`，才报告本次预约或取消完成。可用列表没有精准匹配或结果不明时，先用 `bookings` 核对本人是否已有相同场地和时段的预约。已有预约按列表状态说明。如需更换场地或时间，由用户选定。预览确认重试沿用同一个确认 ID。
