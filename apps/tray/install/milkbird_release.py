"""Select and verify a public MilkBird Tray macOS release asset."""

import argparse
import hashlib
import json
import os
import platform
import re
import subprocess
import tempfile
from http.client import HTTPException
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import urlopen

RELEASES_URL = "https://github.com/a2w-team/MilkBirdClient/releases"
MANIFEST_URL = RELEASES_URL + "/latest/download/install-manifest.json"
MANIFEST_LIMIT = 64 * 1024
CHUNK_SIZE = 1024 * 1024
ARCHITECTURES = ("arm64", "x86_64")


class ReleaseError(Exception):
    """The requested release cannot be selected or verified."""


def version_tuple(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]{1,4}(\.[0-9]{1,4}){0,2}", value):
        raise ReleaseError(f"无法识别 macOS 版本 {value!r}。")
    parts = tuple(map(int, value.split(".")))
    return parts + (0,) * (3 - len(parts))


def mac_host():
    if platform.system() != "Darwin":
        raise ReleaseError("请在实际运行托盘的 Mac 执行安装，或指定该 Mac 的芯片架构和系统版本查询选包。")

    arch = platform.machine()
    if arch == "x86_64":
        # A translated Python process can report x86_64 on an Apple Silicon Mac.
        try:
            arm64 = subprocess.run(
                ["/usr/sbin/sysctl", "-in", "hw.optional.arm64"],
                capture_output=True,
                text=True,
                timeout=5,
            )
            if arm64.returncode == 0 and arm64.stdout.strip() == "1":
                arch = "arm64"
        except (OSError, subprocess.TimeoutExpired):
            pass

    if arch not in ARCHITECTURES:
        raise ReleaseError(f"当前支持 Apple Silicon 与 Intel Mac，识别到的架构为 {arch!r}。")
    version = platform.mac_ver()[0]
    version_tuple(version)
    return arch, version


def get_manifest():
    try:
        with urlopen(MANIFEST_URL, timeout=20) as response:
            data = response.read(MANIFEST_LIMIT + 1)
    except HTTPError as error:
        if error.code == 404:
            raise ReleaseError(f"目前没有公开的安装包索引。可用版本见 {RELEASES_URL}，源码构建方法见 README。") from error
        raise ReleaseError(f"无法读取公开安装包索引（HTTP {error.code}）。") from error
    except (OSError, HTTPException) as error:
        raise ReleaseError(f"无法读取公开安装包索引。{error}") from error

    if len(data) > MANIFEST_LIMIT:
        raise ReleaseError("公开安装包索引超过允许的大小。")
    try:
        return json.loads(data)
    except (UnicodeError, ValueError) as error:
        raise ReleaseError("公开安装包索引不是有效的 JSON。") from error


def artifact_filename(url):
    if not isinstance(url, str):
        raise ReleaseError("安装包索引缺少下载地址。")
    try:
        parsed = urlsplit(url)
    except ValueError as error:
        raise ReleaseError("安装包索引的下载地址格式无效。") from error
    if (
        parsed.scheme != "https"
        or parsed.netloc != "github.com"
        or parsed.query
        or parsed.fragment
        or not re.fullmatch(
            r"/a2w-team/MilkBirdClient/releases/download/[A-Za-z0-9][A-Za-z0-9._-]*/"
            r"[A-Za-z0-9][A-Za-z0-9._-]*\.dmg",
            parsed.path,
        )
    ):
        raise ReleaseError("安装包地址必须是 MilkBirdClient 的公开、固定版本 DMG 资产。")
    return parsed.path.rsplit("/", 1)[-1]


def select_artifact(manifest, arch, os_version):
    if arch not in ARCHITECTURES:
        raise ReleaseError(f"安装包仅支持 Apple Silicon 与 Intel Mac，当前架构为 {arch!r}。")
    host_version = version_tuple(os_version)
    if (
        not isinstance(manifest, dict)
        or type(manifest.get("schemaVersion")) is not int
        or manifest["schemaVersion"] != 1
        or not isinstance(manifest.get("artifacts"), list)
        or not all(isinstance(item, dict) for item in manifest["artifacts"])
    ):
        raise ReleaseError("公开安装包索引格式无效。")

    matches = [item for item in manifest["artifacts"] if item.get("os") == "macos" and item.get("arch") == arch]
    if not matches:
        raise ReleaseError(f"公开版本目前没有 macOS {arch} 安装包。可用版本见 {RELEASES_URL}。")
    if len(matches) != 1:
        raise ReleaseError(f"安装包索引为 macOS {arch} 列出了多个安装包。")
    artifact = matches[0]
    minimum = artifact.get("minimumOsVersion")
    if host_version < version_tuple(minimum):
        raise ReleaseError(f"该安装包要求 macOS {minimum} 或更新版本，当前版本为 {os_version}。")
    artifact_filename(artifact.get("url"))
    sha256 = artifact.get("sha256")
    if not isinstance(sha256, str) or not re.fullmatch(r"[0-9a-fA-F]{64}", sha256):
        raise ReleaseError("安装包索引缺少有效的 SHA-256 校验值。")
    return artifact


def verify_native_image(path):
    checks = (
        (["/usr/bin/hdiutil", "verify", str(path)], "磁盘映像完整性校验失败"),
        (
            ["/usr/sbin/spctl", "--assess", "--type", "open", "--context",
             "context:primary-signature", "--verbose=2", str(path)],
            "Gatekeeper 未能验证磁盘映像的签名",
        ),
    )
    for command, message in checks:
        try:
            result = subprocess.run(
                command,
                capture_output=True,
                text=True,
                timeout=120,
                env={**os.environ, "LC_ALL": "C"},
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise ReleaseError(f"{message}。{error}") from error
        output = result.stdout + result.stderr
        if result.returncode:
            raise ReleaseError(f"{message}。{output.strip()}")
        if command[0] == "/usr/sbin/spctl" and "source=Notarized Developer ID" not in output.splitlines():
            raise ReleaseError("安装包未通过 Developer ID 公证验证。")


def read_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(CHUNK_SIZE), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_existing(path, expected):
    if path.is_symlink() or not path.is_file() or read_digest(path) != expected:
        raise ReleaseError(f"目标路径 {path} 已存在不同的内容，请选用另一下载目录。")
    verify_native_image(path)
    return path


def download_artifact(artifact, download_dir):
    arch, os_version = mac_host()
    artifact = select_artifact({"schemaVersion": 1, "artifacts": [artifact]}, arch, os_version)
    url = artifact["url"]
    expected = artifact["sha256"].lower()
    temporary = None
    try:
        directory = Path(download_dir).expanduser().resolve()
        destination = directory / artifact_filename(url)
        if not directory.is_dir():
            raise ReleaseError(f"下载目录不存在。{directory}")
        if destination.exists() or destination.is_symlink():
            return verify_existing(destination, expected)

        digest = hashlib.sha256()
        with tempfile.NamedTemporaryFile(dir=directory, prefix=".milkbird-download-", suffix=".dmg", delete=False) as stream:
            temporary = Path(stream.name)
            with urlopen(url, timeout=30) as response:
                for chunk in iter(lambda: response.read(CHUNK_SIZE), b""):
                    stream.write(chunk)
                    digest.update(chunk)
        if digest.hexdigest() != expected:
            raise ReleaseError("安装包未通过 SHA-256 校验。请在公开 Release 核对发布索引后重试。")
        verify_native_image(temporary)
        try:
            os.link(temporary, destination)
        except FileExistsError:
            return verify_existing(destination, expected)
        return destination
    except (OSError, HTTPException) as error:
        raise ReleaseError(f"无法下载或读取安装包。{error}") from error
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--select-only", action="store_true", required=True, help="仅查询选包结果")
    parser.add_argument("--arch", choices=ARCHITECTURES, required=True, help="实际使用托盘的 Mac 的芯片架构")
    parser.add_argument("--macos-version", required=True, help="实际使用托盘的 Mac 的 macOS 版本")
    args = parser.parse_args()

    try:
        artifact = select_artifact(get_manifest(), args.arch, args.macos_version)
        print(json.dumps(artifact, ensure_ascii=False, indent=2))
    except ReleaseError as error:
        parser.exit(1, f"{error}\n")
    except KeyboardInterrupt:
        parser.exit(130, "已中断。可以重新运行安装命令。\n")


if __name__ == "__main__":
    main()
