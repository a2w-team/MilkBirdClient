# MilkBirdClient

奶鸟俱乐部的开源客户端，包含 **Tauri 2 桌面托盘应用** 与 **项目级 `/a2w` Agent Skill**。

## 核心能力

- **以需求为核心的三级界面**：首页展示当前最多 **3 个活跃需求** 与各自未读数；点击进入需求详情页查看所有系统回应与关联群聊；支持查看单条需求的完整**修改版本历史（v1/v2/...）**。
- **异构回应与安全渲染**：统一展示链接卡片、富文本回应与沙箱 `iframe` HTML 卡片，支持点赞/不感兴趣反馈；多人匹配全员同意后进入 **24 小时临时群**，支持到期前过半投票留存。
- **右下角互斥悬浮提醒**：主窗口未打开时，在当前屏幕右下角弹出可关闭的轻量通知浮窗；主窗口打开后只展示应用内未读红点，不重复打扰。
- **本地 Agent 协同（`/a2w` Skill）**：通过当前用户私有的 Unix socket 与 `skills/a2w/scripts/a2w_tray.py`（16 个子命令）联通本地 Agent，支持需求创建/修改/历史查询、匹配同意与群聊交互。

> 注意：本仓库的 `/a2w` 是面向校园用户的**产品端 Skill**，与研发协作看板 CoDev 的 CLI `a2w` 不同。

## 文档与契约

- [安装入口](install.md)
- [a2w 产品 Skill](skills/a2w/SKILL.md)
- [托盘本机工具契约](contracts/tray/v1/tools.md)
- [后台登录契约](contracts/club/v1/auth.md)
- [后台需求、匹配与群聊契约](contracts/club/v1/demands-and-rooms.md)

## 本地开发与构建

在 macOS 上需准备 **Rust 1.90+**、**pnpm** 和 **Python 3**。登录时填写校园邮箱 `@` 前的部分，再从下拉框选后缀。托盘默认提供 `hkust-gz.edu.cn` 和 `connect.hkust-gz.edu.cn`。连接自定义 Hub 时，可在启动托盘前用 `MILKBIRD_ALLOWED_EMAIL_DOMAINS` 设置逗号分隔的下拉选项，Hub 负责最终校验。

启动开发版托盘前，可用 `MILKBIRD_API_ORIGIN` 指定奶鸟俱乐部后端地址（默认 `http://127.0.0.1:8080`）。

```sh
cargo run --locked --manifest-path apps/tray/src-tauri/Cargo.toml
```

构建 macOS 应用包（产出位于 `apps/tray/src-tauri/target/debug/bundle/macos/MilkBird Tray.app`）：

```sh
cd apps/tray
pnpm dlx @tauri-apps/cli@2.12.3 build --debug --bundles app -- --locked
```

### 一键启动本地 Mock

在仓库根目录运行，需要 **Python 3.10+** 和 Rust。

```sh
python3 apps/tray/dev/start_mock.py
```

首次运行会编译托盘。窗口打开后，填写任意邮箱名（如 `demo`），任选一个后缀，点击「获取验证码」并填入 `123456`。菜单栏中的 `a2w Mock` 是演示实例。

- 打开「今晚南门约火锅」查看三种回应卡片并提交反馈，点击版本号查看修改历史。
- 打开「周末双打缺一位搭档」试用匹配邀请，或进入「找同学一起复习概率论」的群聊发消息、投票保留。

Mock 使用临时回环端口和独立的本机会话目录。按终端的 `Ctrl+C` 或从托盘菜单退出后会清理演示环境，下次启动恢复示例数据。

## 验证命令

```sh
cargo test --locked --manifest-path apps/tray/src-tauri/Cargo.toml
python3 -m py_compile skills/a2w/scripts/a2w_tray.py
node --check apps/tray/ui/app.js && node --check apps/tray/ui/toast.js
```

## 双托盘演示

- `MILKBIRD_TRAY_TITLE`：设置菜单栏标题（默认 `a2w`，有未读时显示 `<title> · N`）。
- `MILKBIRD_WINDOW_OFFSET_X`：以整数逻辑像素水平偏移主窗口（默认 `0`，负数向左），便于同屏并排演示两个账号。
- `MILKBIRD_TRAY_DIR`：为每个实例指定独立数据目录（存放 Unix socket 与登录会话）；调用 `a2w_tray.py` 时传入相同的 `MILKBIRD_TRAY_DIR` 即可驱动对应账号。

```sh
cargo build --locked --manifest-path apps/tray/src-tauri/Cargo.toml
demo_dir=$(mktemp -d /tmp/milkbird-demo.XXXXXX)
tray=apps/tray/src-tauri/target/debug/milkbird-tray
MILKBIRD_TRAY_DIR="$demo_dir/alice" MILKBIRD_TRAY_TITLE=Alice MILKBIRD_WINDOW_OFFSET_X=-600 "$tray" &
MILKBIRD_TRAY_DIR="$demo_dir/bob" MILKBIRD_TRAY_TITLE=Bob MILKBIRD_WINDOW_OFFSET_X=0 "$tray" &
```
