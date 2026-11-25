# 发布 macOS 客户端

首批支持 Apple Silicon。发布流程以 `Cargo.lock`、仓库的 Tauri 配置和发布标签定位源码，读取实际可执行文件的架构及最低 macOS 版本，再根据最终公证并钉票的 DMG 生成安装索引。

## 开发预览

在 Apple Silicon Mac 构建调试应用包。以下命令只在本机生成预览磁盘映像。

```sh
cd apps/tray
pnpm dlx @tauri-apps/cli@2.12.3 build --debug --bundles app -- --locked
cd ../..
python3 apps/tray/install/prepare_macos_release.py \
  --app "apps/tray/src-tauri/target/debug/bundle/macos/MilkBird Tray.app" \
  --preview --out /tmp/milkbird-preview
```

预览命令校验 DMG 完整性并显示 SHA-256。使用 [安装指南](../install.md) 的开发预览模式验证项目 Skill 和托盘连通性。正式安装索引由签名发布流程生成。

## 正式版

在 GitHub 仓库的 `macos-release` 环境配置以下凭据。签名证书必须是团队的 Developer ID Application 证书。可以在已有发行 Mac 的钥匙串导出含私钥的 PKCS#12，再编码为 Base64。公证使用团队的 App Store Connect API 密钥。凭据存入 GitHub Actions Secrets。 [Tauri macOS 签名指南](https://v2.tauri.app/distribute/sign/macos/) · [Apple 公证指南](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)

| Secret | 内容 |
| --- | --- |
| `APPLE_SIGNING_IDENTITY` | Developer ID Application 证书的完整名称 |
| `APPLE_CERTIFICATE` | PKCS#12 证书的 Base64 |
| `APPLE_CERTIFICATE_PASSWORD` | PKCS#12 证书的导出密码 |
| `APPLE_API_ISSUER` | App Store Connect 团队 API Issuer ID |
| `APPLE_API_KEY` | App Store Connect API Key ID |
| `APPLE_API_KEY_P8_BASE64` | 公证 API 的 `.p8` 私钥 Base64 |

1. 在 Actions 手动运行 **Prepare macOS release**，选择 `main`。标签与 Tauri 的应用版本一致，例如 `v0.1.0`。
2. 工作流构建 arm64 应用并验证签名。它签名并公证 DMG，校验钉票和 Gatekeeper，再生成 `install-manifest.json`、将两项产物上传到同一个 Release 草稿。输入标签已存在时流程会停止。
3. 从草稿取得 DMG，在干净的 Mac 打开、安装并启动，按公开安装指南验证项目 Skill。核对草稿的 DMG 文件名、SHA-256、源码标签与索引。验收后发布并标为最新版本，再用公开链接复查无需私有仓库访问即可下载。 [GitHub 发布流程](https://cli.github.com/manual/gh_release_create)

索引由安装器从公开 Releases 读取，包含 `os`、`arch`、`minimumOsVersion`、固定版本的 `url` 与最终 DMG 的 `sha256`。清单中的最低系统版本取应用配置与 Mach-O 编译目标中较新的值。
