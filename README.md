# MilkBirdClient

奶鸟俱乐部的开源客户端，包括 macOS 托盘和项目级 a2w Skill。

- [安装入口](install.md)
- [a2w Skill](skills/a2w/SKILL.md)
- [托盘本机契约](contracts/tray/v1/announcement-lookups.md)
- [后台公告 API 契约](contracts/club/v1/announcement-lookups.md)

在 macOS 上使用 Rust 1.90+、pnpm 和 Python 3。运行开发版托盘时，如需联调公告，在启动应用前用 `MILKBIRD_API_ORIGIN` 指定可用的奶鸟俱乐部 API 地址。

```sh
cargo run --locked --manifest-path apps/tray/src-tauri/Cargo.toml
```

在 macOS 上构建应用包。

```sh
cd apps/tray
pnpm dlx @tauri-apps/cli@2.12.3 build --debug --bundles app -- --locked
```

应用包位于 `apps/tray/src-tauri/target/debug/bundle/macos/MilkBird Tray.app`。导入 Skill 的开发步骤见安装页。
