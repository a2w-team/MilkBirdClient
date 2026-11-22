# 安装奶鸟俱乐部客户端

**开发预览。** 本页是公开安装入口。正式预编译包将在 [GitHub Releases](https://github.com/a2w-team/MilkBirdClient/releases) 发布。

## 安装顺序

1. 用户把本页链接发给自己的 Agent，并指定要安装 a2w Skill 的项目。
2. Agent 按实际运行托盘的电脑确认系统、CPU 架构和版本。首批计划提供 macOS 包，发布后优先选匹配的预编译包。没有匹配产物时，提示用户相应平台的支持状态。
3. 用户安装并打开 **MilkBird Tray**。已安装的兼容客户端可供不同项目共用。
4. Agent 把完整的 [a2w Skill 目录](skills/a2w/)放入当前项目的 Skill 目录，例如 `.agents/skills/a2w`。如宿主使用其他项目级路径，按宿主的项目级加载规则安装。
5. 用户在该项目的新会话输入 `/a2w` 激活。之后同一会话里的奶鸟俱乐部相关请求可以直接接着说。

## 开发者源码构建

目前可以按 [README](README.md) 在 macOS 上构建开发包。打开该包后，可从包内的 `Contents/Resources/skills/a2w` 复制完整 Skill 目录到项目。调用脚本需使用该 Mac 上的 Python 3。
