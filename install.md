# 安装奶鸟俱乐部客户端

> **当前阶段：开发预览**。正式签名预编译包将在 [GitHub Releases](https://github.com/a2w-team/MilkBirdClient/releases) 发布，目前支持在 macOS 本地一键编译体验。

## 安装流程

1. **安装并启动托盘应用（MilkBird Tray）**
   - 正式版发布后：按本机系统与芯片架构下载对应 Release 安装包并打开。
   - 开发预览期：按 [README](README.md#本地开发与构建) 在 macOS 上运行 `cargo run --locked --manifest-path apps/tray/src-tauri/Cargo.toml` 或构建 `.app` 安装包。
   - 打开托盘后使用 `hkust-gz.edu.cn` 校园邮箱完成验证码登录。

2. **将 `/a2w` 产品 Skill 导入本地 Agent**
   - 将本仓库的完整 [skills/a2w/](skills/a2w/) 目录（包含 `SKILL.md` 与 `scripts/a2w_tray.py`）复制到你的项目或宿主 Agent 的 Skill 目录（例如 `.agents/skills/a2w` 或 `.claude/skills/a2w`）。
   - 若使用打包后的 `MilkBird Tray.app`，可直接从应用包内的 `Contents/Resources/skills/a2w` 复制。

3. **验证连通性**
   确认本地托盘已运行且 Python 3 可用后，可执行以下命令验证本机桥接与登录态：
   ```sh
   python3 skills/a2w/scripts/a2w_tray.py status
   ```

4. **开始使用**
   在你的 Agent 会话中输入 `/a2w` 激活，随后直接描述校园需求（例如找运动/拼车/吃饭搭子），Agent 会先整理需求卡片经你确认后再提交到托盘。

## 校园私人助理

把完整的 [skills/campus-assistant/](skills/campus-assistant/) 目录复制到项目的 `.agents/skills/campus-assistant`。应用包也提供同一份 Skill，位于 `Contents/Resources/skills/campus-assistant/`。然后在该项目的 Agent 对话直接说「查今天的课表」或「帮我约明天下午 3 点到 4 点的演示会议室 A」。

首次使用时在托盘的「校园私人助理」连接对应服务。当前开发预览支持[模拟账号](README.md#试用校园私人助理)。真实服务的授权和操作以校方接口为准。
