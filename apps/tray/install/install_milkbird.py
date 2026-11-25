#!/usr/bin/env python3
"""Install the MilkBird Skill from a compatible Mac app into one project."""

import argparse
import hashlib
import json
import os
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from milkbird_release import (
    ReleaseError,
    download_artifact,
    get_manifest,
    mac_host,
    select_artifact,
    version_tuple,
)

APP_NAME = "MilkBird Tray.app"
APP_ID = "cn.fordece.milkbird.tray"
RECEIPT = ".milkbird-install.json"
BRIDGE = "scripts/a2w_tray.py"


class InstallError(Exception):
    pass


def native_tool(*args):
    try:
        return subprocess.run(
            args, check=True, capture_output=True, text=True, timeout=30
        ).stdout
    except (OSError, subprocess.SubprocessError) as error:
        raise InstallError(f"macOS 校验失败：{Path(args[0]).name}。") from error


def app_skill(app, arch, os_version, preview=False):
    try:
        with (app / "Contents/Info.plist").open("rb") as stream:
            info = plistlib.load(stream)
    except (OSError, ValueError, plistlib.InvalidFileException) as error:
        raise InstallError(f"无法读取应用包：{app}") from error

    if not isinstance(info, dict) or info.get("CFBundleIdentifier") != APP_ID:
        raise InstallError(f"应用标识与 MilkBird Tray 不符：{app}")
    executable = info.get("CFBundleExecutable")
    if not isinstance(executable, str) or Path(executable).name != executable:
        raise InstallError(f"应用包缺少有效的可执行文件：{app}")
    binary = app / "Contents/MacOS" / executable
    if not binary.is_file() or binary.is_symlink() or not os.access(binary, os.X_OK):
        raise InstallError(f"应用可执行文件不可用：{binary}")
    description = native_tool("/usr/bin/file", "-b", str(binary))
    if "Mach-O" not in description or not re.search(rf"\b{arch}\b", description):
        raise InstallError(f"应用包不支持本机 CPU 架构 {arch}：{app}")

    requirements = info.get("LSMinimumSystemVersionByArchitecture", {})
    if not isinstance(requirements, dict):
        raise InstallError(f"应用包的最低系统版本格式有误：{app}")
    minimum = requirements.get(arch, info.get("LSMinimumSystemVersion"))
    if minimum is None or version_tuple(os_version) < version_tuple(minimum):
        raise InstallError(f"应用包的最低系统版本未知或高于本机版本：{app}")

    if not preview:
        native_tool("/usr/bin/codesign", "--verify", "--deep", "--strict", str(app))
        native_tool("/usr/sbin/spctl", "--assess", "--type", "execute", str(app))

    source = app / "Contents/Resources/skills/a2w"
    files = skill_files(source)
    if not {"SKILL.md", BRIDGE}.issubset(files):
        raise InstallError(f"应用包缺少完整的 a2w Skill：{app}")
    if os.path.lexists(source / RECEIPT):
        raise InstallError(f"应用内的 Skill 已包含安装记录：{app}")
    return source


def project_skill(project, create=False):
    project = project.expanduser().resolve(strict=True)
    if not project.is_dir() or project in (Path.home().resolve(), Path(project.anchor)):
        raise InstallError("请选择一个明确的项目目录。")
    agents = project / ".agents"
    discovery = agents / "skills"
    for directory in (agents, discovery):
        if directory.is_symlink() or (directory.exists() and not directory.is_dir()):
            raise InstallError(f"项目 Skill 目录不可用：{directory}")
        if create:
            directory.mkdir(exist_ok=True)
    return discovery / "a2w"


def skill_files(directory):
    if directory.is_symlink() or not directory.is_dir():
        raise InstallError(f"Skill 目录不可用：{directory}")
    files = {}
    for path in directory.rglob("*"):
        if path.is_symlink():
            raise InstallError(f"Skill 含符号链接：{path}")
        if path.is_dir():
            continue
        if not path.is_file():
            raise InstallError(f"Skill 含非普通文件：{path}")
        relative = path.relative_to(directory).as_posix()
        if relative != RECEIPT:
            files[relative] = hashlib.sha256(path.read_bytes()).hexdigest()
    return files


def install_skill(project, source):
    expected = skill_files(source)
    target = project_skill(project, create=True)
    if os.path.lexists(target):
        if skill_files(target) == expected:
            return target, False
        raise InstallError(f"项目已有不同的 a2w Skill，请先检查并保留已有改动：{target}")

    with tempfile.TemporaryDirectory(prefix=".milkbird-install-", dir=target.parent.parent) as staging:
        staged = Path(staging) / "a2w"
        shutil.copytree(source, staged)
        if skill_files(staged) != expected:
            raise InstallError("安装期间应用内的 Skill 内容发生变化，请重试。")
        (staged / RECEIPT).write_text(
            json.dumps(expected, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        try:
            os.rename(staged, target)
        except OSError:
            if os.path.lexists(target) and skill_files(target) == expected:
                return target, False
            raise
    return target, True


def uninstall_skill(project):
    target = project_skill(project)
    if not os.path.lexists(target):
        return False

    current = skill_files(target)
    receipt = target / RECEIPT
    if not receipt.is_file():
        raise InstallError(f"项目 Skill 由其他方式安装，请先检查目录：{target}")
    try:
        recorded = json.loads(receipt.read_text(encoding="utf-8"))
    except ValueError as error:
        raise InstallError(f"项目 Skill 安装记录不可读：{receipt}") from error
    if current != recorded:
        raise InstallError(f"项目 Skill 内容已改动，请先检查并保留已有改动：{target}")

    with tempfile.TemporaryDirectory(prefix=".milkbird-remove-", dir=target.parent.parent) as staging:
        os.rename(target, Path(staging) / "a2w")
    return True


def check_bridge(skill):
    try:
        result = subprocess.run(
            [sys.executable, str(skill / BRIDGE), "status"],
            capture_output=True, text=True, timeout=12,
        )
        reply = json.loads(result.stdout)
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        raise InstallError("无法读取托盘状态。请打开 MilkBird Tray 后重试。") from error
    if result.returncode == 0 and isinstance(reply, dict) and reply.get("ok") is True:
        data = reply.get("data")
        if isinstance(data, dict) and isinstance(data.get("loggedIn"), bool):
            return data["loggedIn"]
    raise InstallError("托盘未连通。请打开 MilkBird Tray 后重新运行安装命令。")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", type=Path, required=True, help="要安装 Skill 的项目根目录")
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--app", type=Path, help="已有 MilkBird Tray.app 的路径")
    source.add_argument("--download-dir", type=Path, help="保存匹配的已验证 DMG 的现有目录")
    parser.add_argument("--preview", action="store_true", help="复用自己构建的本地开发预览应用")
    parser.add_argument("--uninstall", action="store_true", help="移除本工具安装的当前项目 Skill")
    args = parser.parse_args(argv)
    if args.preview and not args.app:
        parser.error("--preview 需要 --app")
    if args.uninstall and (args.app or args.download_dir):
        parser.error("--uninstall 只需要 --project")

    try:
        project_skill(args.project)
        if args.uninstall:
            removed = uninstall_skill(args.project)
            print("已卸载当前项目的 a2w Skill。" if removed else "当前项目没有 a2w Skill。")
            return 0

        arch, os_version = mac_host()
        if args.app:
            skill = app_skill(args.app.expanduser().resolve(), arch, os_version, args.preview)
        else:
            skill = None
            errors = []
            for location in (Path("/Applications"), Path.home() / "Applications"):
                app = location / APP_NAME
                if not app.exists():
                    continue
                try:
                    skill = app_skill(app, arch, os_version)
                    break
                except (InstallError, ReleaseError) as error:
                    errors.append(str(error))
            if skill is None:
                if args.download_dir:
                    artifact = select_artifact(get_manifest(), arch, os_version)
                    dmg = download_artifact(artifact, args.download_dir.expanduser())
                    print(f"已验证安装包：{dmg}")
                    print("请用 Finder 打开安装包，将 MilkBird Tray 拖入「应用程序」，再重试项目安装。")
                    return 0
                for error in errors:
                    print(error, file=sys.stderr)
                raise InstallError(
                    f"未找到适用于本机 macOS {os_version}（{arch}）的应用。"
                    "请用 --app 指定已有应用，或用 --download-dir 选择安装包的保存目录。"
                )

        target, installed = install_skill(args.project, skill)
        print(f"{'已安装' if installed else '已复用'}项目 Skill：{target}", flush=True)
        if check_bridge(target):
            print("托盘已连通，校园账号已登录。")
        else:
            print("托盘已连通，请在 MilkBird Tray 中完成校园邮箱登录。")
        return 0
    except (InstallError, ReleaseError, OSError) as error:
        print(error, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("已中断。可以重新运行同一命令。", file=sys.stderr)
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
