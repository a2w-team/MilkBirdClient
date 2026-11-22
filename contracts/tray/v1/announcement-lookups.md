# a2w Skill → 托盘 · 校园公告查询 v1

用户首次安装并打开本机托盘，再将 a2w Skill 导入 Agent。确认查询后，Agent 运行 Skill 目录内的 `scripts/submit.py`。脚本连接托盘的 Unix socket，默认路径为 `~/.milkbird/tray.sock`。设置 `MILKBIRD_TRAY_DIR` 可让脚本和托盘使用同一自定义目录。托盘要求目录权限仅当前系统用户可访问，socket 文件权限为 0600。

每次连接发送一行 UTF-8 JSON，以 LF 结束，收到一行回复后关闭。每帧最多 8 KiB。Tauri 后台线程按请求使用同一个 HTTP 客户端，托盘启动时只建立原生菜单和 socket，收到结果或成员主动打开时再创建系统 WebView。主线程负责更新最近结果和窗口。

```json
{"version":1,"confirmed":true,"query":{"campus":"东校区","topic":"期末考试"}}
```

托盘仅接收这个版本且已经确认的请求，将 `query` 转发给[后台公告查询接口](../../club/v1/announcement-lookups.md)。成功时回复后台的结果，托盘窗口展示相同状态。错误以 `{"status":"error","code":"server_unavailable"}` 这一结构返回。

- 托盘错误码包括 `invalid_request`、`confirmation_required`、`server_unavailable`、`invalid_response`。
- 提交脚本错误码包括 `tray_unavailable`、`tray_connection_failed`、`invalid_response`。返回错误时 CLI 的退出码为 1。
- 首条联调使用 macOS 本地 Unix socket。托盘转发配置来自 `MILKBIRD_API_ORIGIN`，默认 `http://127.0.0.1:8080`。
