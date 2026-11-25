# 安装奶鸟俱乐部客户端

把本页链接发给你要使用的项目 Agent，并告诉它项目目录。请在实际运行 MilkBird Tray 的 Mac 上安装。Agent 如果运行在远程容器中，先确认目标 Mac 使用 Apple 芯片还是 Intel，再把对应步骤交给本机终端执行。

目前处于开发预览期，[GitHub Releases](https://github.com/a2w-team/MilkBirdClient/releases) 尚无正式签名安装包。手头已有从本仓库构建并信任的应用时，可以走下方开发预览流程。

## 安装托盘与项目 Skill

在目标 Mac 上准备 Python 3。可以使用已有源码、从 GitHub 项目页下载 ZIP，或运行以下命令获取源码。

```sh
git clone https://github.com/a2w-team/MilkBirdClient.git
cd MilkBirdClient
```

在 **MilkBirdClient 源码目录**执行安装命令，将下面的路径换成已经存在的目标项目根目录。

```sh
python3 apps/tray/install/install_milkbird.py --project "/你的项目绝对路径"
```

脚本检查本机系统、芯片架构和已安装托盘。找到兼容应用后，从应用包复制完整 Skill 到目标项目的 `.agents/skills/a2w`，并核对安装结果。

如果本机缺少可用托盘，脚本会提示当前可用的安装方式。有适配的正式签名 DMG 时，可加上 `--download-dir ~/Downloads`，由脚本下载并校验安装包。打开校验通过的 DMG，将 **MilkBird Tray.app** 拖到「应用程序」，按 macOS 提示完成安装，然后重跑上面的命令。

远程 Agent 如需查询安装包，请先让用户在目标 Mac 的「关于本机」查看芯片和 macOS 版本，再使用确认过的值查询。安装和连通性检查仍在目标 Mac 上运行。

```sh
python3 apps/tray/install/milkbird_release.py --select-only --arch arm64 --macos-version 15.0
```

上例使用 Apple 芯片和 macOS 15.0。Intel Mac 对应 `x86_64`。

### 开发预览

按 [README 的本地开发与构建步骤](README.md#本地开发与构建)构建 macOS `.app`。把命令中的两个路径换成实际的项目和应用包绝对路径。

```sh
python3 apps/tray/install/install_milkbird.py --project "/你的项目绝对路径" \
  --app "/应用包所在目录/MilkBird Tray.app" --preview
```

开发预览用本地已确认来源的构建产物。保留 macOS 安全设置，按系统提示打开应用。

## 验证并开始使用

打开 **MilkBird Tray**，使用校园邮箱在托盘中完成验证码登录。进入目标项目根目录，运行安装后的桥接脚本检查状态。

```sh
python3 .agents/skills/a2w/scripts/a2w_tray.py status
```

返回 `ok: true` 表示本机 Agent 能连接托盘，`loggedIn: true` 表示托盘已登录。连接失败时，先确认托盘已在当前 macOS 用户会话启动。返回未登录时，到托盘完成登录后重试。下载、复制或连通性检查中断后，重新运行同一命令即可。重复运行会复用内容相同的安装。

在该项目的 Codex 会话中，由你输入或选择 `$a2w`。支持斜杠激活的宿主使用 `/a2w`。随后描述你想找的饭搭子、运动搭子或其他校园需求，Agent 会整理需求卡，等你确认后提交。

## 卸载项目 Skill

在 MilkBirdClient 源码目录执行以下命令，卸载脚本在指定项目中安装的 Skill。每个项目独立安装和卸载，托盘应用由 macOS 管理。

```sh
python3 apps/tray/install/install_milkbird.py --project "/你的项目绝对路径" --uninstall
```
