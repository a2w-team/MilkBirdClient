"""Package a macOS arm64 app and prepare the public installer index."""

import argparse
import json
import os
import plistlib
import re
import subprocess
import tempfile
from pathlib import Path

from milkbird_release import (
    RELEASES_URL,
    ReleaseError,
    read_digest,
    select_artifact,
    verify_native_image,
    version_tuple,
)

MANIFEST_NAME = "install-manifest.json"
APP_IDENTIFIER = "cn.fordece.milkbird.tray"


def run(*command, timeout=120):
    try:
        result = subprocess.run(
            command, capture_output=True, text=True, check=True, timeout=timeout,
            env={**os.environ, "LC_ALL": "C"},
        )
    except subprocess.CalledProcessError as error:
        raise ReleaseError(f"{Path(command[0]).name} 校验或打包失败（退出状态 {error.returncode}）。") from error
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ReleaseError(f"无法完成 {Path(command[0]).name}。") from error
    return result.stdout, result.stderr


def app_metadata(app):
    with (app / "Contents" / "Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    if not isinstance(info, dict) or info.get("CFBundleIdentifier") != APP_IDENTIFIER:
        raise ReleaseError("应用包标识与 MilkBird Tray 不符。")
    version = info.get("CFBundleShortVersionString")
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ReleaseError("应用包缺少有效的三段版本号。")
    name = info.get("CFBundleExecutable")
    if not isinstance(name, str) or Path(name).name != name or not name:
        raise ReleaseError("应用包缺少可执行文件名。")
    binary = app / "Contents" / "MacOS" / name
    architecture, _ = run("/usr/bin/lipo", "-archs", str(binary))
    if architecture.strip() != "arm64":
        raise ReleaseError("当前发布流程只接受 macOS arm64 应用包。")
    build_info, _ = run("/usr/bin/otool", "-l", str(binary))
    targets = re.findall(r"^\s*minos\s+([0-9.]+)\s*$", build_info, re.MULTILINE)
    if not targets:
        raise ReleaseError("无法识别可执行文件的最低 macOS 版本。")
    minimum = max(info.get("LSMinimumSystemVersion", "11.0"), *targets, key=version_tuple)
    return version, minimum


def signing_credentials():
    names = ("APPLE_SIGNING_IDENTITY", "APPLE_API_KEY_PATH", "APPLE_API_KEY", "APPLE_API_ISSUER")
    missing = [name for name in names if not os.environ.get(name)]
    if missing:
        raise ReleaseError("缺少签名或公证配置 " + "、".join(missing))
    identity, key_path, key_id, issuer = (os.environ[name] for name in names)
    if not identity.startswith("Developer ID Application:") or not Path(key_path).is_file():
        raise ReleaseError("请配置有效的 Developer ID Application 身份与公证密钥文件。")
    return identity, key_path, key_id, issuer


def verify_release_app(app, identity):
    run("/usr/bin/codesign", "--verify", "--deep", "--strict", str(app))
    stdout, stderr = run("/usr/bin/codesign", "--display", "--verbose=4", str(app))
    if f"Authority={identity}" not in (stdout + stderr).splitlines():
        raise ReleaseError("应用包签名与本次发行配置的 Developer ID 不符。")
    stdout, stderr = run("/usr/sbin/spctl", "--assess", "--type", "execute", "--verbose=2", str(app))
    if "source=Notarized Developer ID" not in (stdout + stderr).splitlines():
        raise ReleaseError("待打包应用未通过 Developer ID 公证验证。")


def notarize_image(image, credentials):
    identity, key_path, key_id, issuer = credentials
    run("/usr/bin/codesign", "--sign", identity, "--timestamp", str(image))
    result, _ = run(
        "/usr/bin/xcrun", "notarytool", "submit", str(image),
        "--key", key_path, "--key-id", key_id, "--issuer", issuer,
        "--wait", "--output-format", "json", timeout=1800,
    )
    if json.loads(result).get("status") != "Accepted":
        raise ReleaseError("Apple 公证服务尚未接受磁盘映像。")
    run("/usr/bin/xcrun", "stapler", "staple", str(image))
    run("/usr/bin/xcrun", "stapler", "validate", str(image))
    verify_native_image(image)


def manifest_for(image, tag, minimum):
    metadata = {
        "schemaVersion": 1,
        "artifacts": [{
            "os": "macos",
            "arch": "arm64",
            "minimumOsVersion": minimum,
            "url": f"{RELEASES_URL}/download/{tag}/{image.name}",
            "sha256": read_digest(image),
        }],
    }
    select_artifact(metadata, "arm64", minimum)
    return metadata


def prepare(app, output, tag):
    app = app.resolve(strict=True)
    version, minimum = app_metadata(app)
    preview = tag is None
    if not preview and tag != f"v{version}":
        raise ReleaseError(f"发布标签须与应用版本一致，预期 v{version}。")
    credentials = None
    if not preview:
        credentials = signing_credentials()
        verify_release_app(app, credentials[0])
    output.mkdir(parents=True, exist_ok=True)
    name = f"MilkBird-Tray_{version}_arm64{'-preview' if preview else ''}.dmg"
    destination = output / name
    index = output / MANIFEST_NAME
    if destination.exists() or (not preview and index.exists()):
        raise ReleaseError("输出目录已有同名文件，请选用其他目录。")
    with tempfile.TemporaryDirectory(prefix=".milkbird-release-", dir=output) as scratch:
        staging = Path(scratch) / "stage"
        staging.mkdir()
        run("/usr/bin/ditto", str(app), str(staging / "MilkBird Tray.app"))
        (staging / "Applications").symlink_to("/Applications")
        image = Path(scratch) / name
        run("/usr/bin/hdiutil", "create", "-quiet", "-volname", "MilkBird Tray",
            "-srcfolder", str(staging), "-format", "UDZO", str(image), timeout=300)
        if preview:
            run("/usr/bin/hdiutil", "verify", str(image))
            os.link(image, destination)
            return destination
        notarize_image(image, credentials)
        staged_index = Path(scratch) / MANIFEST_NAME
        staged_index.write_text(
            json.dumps(manifest_for(image, tag, minimum), ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        os.link(image, destination)
        try:
            os.link(staged_index, index)
        except OSError:
            destination.unlink()
            raise
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--tag", help="为同版本的 GitHub Release 准备正式安装包与索引")
    mode.add_argument("--preview", action="store_true", help="只检查本机开发预览磁盘映像")
    args = parser.parse_args()
    try:
        image = prepare(args.app, args.out.resolve(), args.tag)
    except (ReleaseError, OSError, ValueError, plistlib.InvalidFileException) as error:
        parser.exit(1, f"{error}\n")
    print(image)
    if args.preview:
        print(f"开发预览 SHA-256 {read_digest(image)}")
    else:
        print(image.with_name(MANIFEST_NAME))


if __name__ == "__main__":
    main()
