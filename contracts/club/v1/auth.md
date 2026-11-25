# 校园邮箱验证码认证 · 后台 HTTP v1

本契约定义奶鸟俱乐部首版成员登录与设备会话绑定协议，供 `MilkBirdClient`（桌面托盘）与 `MilkBirdHub`（Go/Hertz 后台）共同遵守。

## 1. 校园邮箱校验规则

- 邮箱提交前统一 `trim` + 转小写。
- 后台通过 `-allowed-email-domains` 或环境变量 `MILKBIRD_ALLOWED_EMAIL_DOMAINS` 配置允许的域名后缀（逗号分隔，默认 `hkust-gz.edu.cn`）。
- 域名匹配规则：邮箱域名等于白名单项（去掉前导 `.`）或以 `.白名单项` 结尾。不满足时返回 `400 {"error":"invalid_campus_email"}`。

## 2. 发送验证码

`POST /api/v1/auth/email/code`

请求体（JSON，≤ 4 KiB，拒绝未知字段）：
```json
{"email": "alice@hkust-gz.edu.cn"}
```

行为：
- 生成 6 位数字验证码，有效期 600 秒（10 分钟），仅可使用一次；同一邮箱重新申请使旧验证码失效。
- 开发联调开关 `-dev-auth-code`（默认 `false`）：开启时服务端打印验证码日志并在响应中附加 `devCode`；关闭时不得返回 `devCode`。

响应 `200 OK`：
```json
{"sent": true, "expiresInSeconds": 600}
```
（`-dev-auth-code=true` 时额外包含 `"devCode": "123456"`）

## 3. 校验验证码并签发设备 Token

`POST /api/v1/auth/email/verify`

请求体（JSON，≤ 4 KiB，拒绝未知字段）：
```json
{
  "email": "alice@hkust-gz.edu.cn",
  "code": "123456",
  "deviceName": "MacBook Pro",
  "displayName": "Alice"
}
```
- `deviceName`、`displayName` 可选；首次注册且未提供 `displayName` 时默认取邮箱 `@` 前缀。
- 验证码错误、过期或已使用时返回 `401 {"error":"invalid_or_expired_code"}`。

响应 `200 OK`：
```json
{
  "token": "mbd_...",
  "member": {
    "id": "mem_...",
    "email": "alice@hkust-gz.edu.cn",
    "displayName": "Alice"
  }
}
```

## 4. 鉴权与身份查询

所有业务接口在请求头携带：`Authorization: Bearer <token>`。无效或已吊销 Token 返回 `401 {"error":"unauthorized"}`。

- `GET /api/v1/me` → `200 {"member": {"id":"...","email":"...","displayName":"..."}}`
- `POST /api/v1/auth/logout` → 吊销当前设备 Token，`200 {"ok": true}`
