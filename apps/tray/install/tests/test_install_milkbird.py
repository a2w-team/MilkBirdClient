import contextlib
import io
import os
import plistlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import install_milkbird as installer


STATUS_SCRIPT = """\
import json
import os
import sys

assert sys.argv[1:] == ["status"]
state = os.environ["MILKBIRD_INSTALL_TEST_STATUS"]
if state == "unavailable":
    print(json.dumps({
        "ok": False,
        "code": "tray_unavailable",
        "message": "Open MilkBird Tray and try again.",
    }))
    sys.exit(1)
print(json.dumps({
    "ok": True,
    "action": "status",
    "data": {"loggedIn": state == "logged-in"},
}))
"""


class InstallTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.project = self.root / "Selected project"
        self.project.mkdir()
        self.other_project = self.root / "Other project"
        self.other_project.mkdir()
        self.app = self.root / "MilkBird Tray.app"
        contents = self.app / "Contents"
        self.source = contents / "Resources" / "skills" / "a2w"
        (self.source / "references").mkdir(parents=True)
        (self.source / "scripts").mkdir()
        (self.source / "SKILL.md").write_text(
            "---\nname: a2w\n---\n# MilkBird\n", encoding="utf-8"
        )
        (self.source / "references" / "demand-confirmation.md").write_text(
            "# Demand confirmation\n", encoding="utf-8"
        )
        (self.source / "scripts" / "a2w_tray.py").write_text(
            STATUS_SCRIPT, encoding="utf-8"
        )
        binary = contents / "MacOS" / "milkbird-tray"
        binary.parent.mkdir()
        binary.write_bytes(b"placeholder for native verification")
        binary.chmod(0o755)
        self.info = contents / "Info.plist"
        self.bundle_properties = {
            "CFBundleIdentifier": "cn.fordece.milkbird.tray",
            "CFBundleShortVersionString": "0.1.0",
            "CFBundleExecutable": binary.name,
            "LSMinimumSystemVersion": "11.0",
        }
        self.write_info()
        self.target = self.project / ".agents" / "skills" / "a2w"

    def write_info(self):
        with self.info.open("wb") as stream:
            plistlib.dump(self.bundle_properties, stream)

    def native_tool(self, *args):
        command = Path(args[0]).name
        if command == "file":
            return "Mach-O 64-bit executable arm64"
        if command == "codesign":
            return ""
        if command == "spctl":
            return "accepted\nsource=Notarized Developer ID"
        raise AssertionError(f"Unexpected native command: {command}")

    def test_release_reuse_checks_native_app_and_signature(self):
        with mock.patch.object(installer, "native_tool", side_effect=self.native_tool) as native:
            source = installer.app_skill(self.app, "arm64", "14.0")
        self.assertEqual(source, self.source)
        self.assertEqual(
            {Path(call.args[0]).name for call in native.call_args_list},
            {"file", "codesign", "spctl"},
        )

    def test_unsigned_bundle_requires_explicit_preview(self):
        def preview_tools(*args):
            if Path(args[0]).name != "file":
                raise installer.InstallError("invalid signature")
            return self.native_tool(*args)

        with mock.patch.object(installer, "native_tool", side_effect=preview_tools):
            with self.assertRaises(installer.InstallError):
                installer.app_skill(self.app, "arm64", "14.0")
            self.assertEqual(
                installer.app_skill(self.app, "arm64", "14.0", preview=True),
                self.source,
            )

    def test_preview_also_checks_architecture_os_and_bundle_identity(self):
        with mock.patch.object(installer, "native_tool", side_effect=self.native_tool):
            with self.assertRaises(installer.InstallError):
                installer.app_skill(self.app, "x86_64", "14.0", preview=True)
            with self.assertRaises(installer.InstallError):
                installer.app_skill(self.app, "arm64", "10.15", preview=True)
            self.bundle_properties["CFBundleIdentifier"] = "example.other-app"
            self.write_info()
            with self.assertRaises(installer.InstallError):
                installer.app_skill(self.app, "arm64", "14.0", preview=True)

    def test_install_repeat_connect_and_uninstall_only_selected_project(self):
        other_skill, _ = installer.install_skill(self.other_project, self.source)
        self.assertEqual(installer.install_skill(self.project, self.source), (self.target, True))
        self.assertEqual(installer.install_skill(self.project, self.source), (self.target, False))
        self.assertTrue((self.target / "references" / "demand-confirmation.md").is_file())

        with mock.patch.dict(os.environ, {"MILKBIRD_INSTALL_TEST_STATUS": "unavailable"}):
            with self.assertRaises(installer.InstallError):
                installer.check_bridge(self.target)
        with mock.patch.dict(os.environ, {"MILKBIRD_INSTALL_TEST_STATUS": "logged-out"}):
            self.assertFalse(installer.check_bridge(self.target))
        with mock.patch.dict(os.environ, {"MILKBIRD_INSTALL_TEST_STATUS": "logged-in"}):
            self.assertTrue(installer.check_bridge(self.target))

        self.assertTrue(installer.uninstall_skill(self.project))
        self.assertFalse(installer.uninstall_skill(self.project))
        self.assertFalse(self.target.exists())
        self.assertTrue(other_skill.is_dir())
        self.assertTrue(self.source.is_dir())

    def test_interrupted_publish_leaves_no_partial_install_and_retry_works(self):
        with mock.patch.object(installer.os, "rename", side_effect=OSError("simulated interruption")):
            with self.assertRaises((installer.InstallError, OSError)):
                installer.install_skill(self.project, self.source)
        self.assertFalse(self.target.exists())
        self.assertEqual(list((self.project / ".agents").glob(".milkbird-install-*")), [])
        self.assertEqual(installer.install_skill(self.project, self.source), (self.target, True))

    def test_cli_retries_status_after_tray_opens(self):
        command = ["--project", str(self.project), "--app", str(self.app), "--preview"]
        output = io.StringIO()
        with mock.patch.object(installer, "native_tool", side_effect=self.native_tool), \
             mock.patch.object(installer, "mac_host", return_value=("arm64", "14.0")), \
             contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            with mock.patch.dict(os.environ, {"MILKBIRD_INSTALL_TEST_STATUS": "unavailable"}):
                self.assertEqual(installer.main(command), 1)
            self.assertTrue(self.target.is_dir())
            with mock.patch.dict(os.environ, {"MILKBIRD_INSTALL_TEST_STATUS": "logged-in"}):
                self.assertEqual(installer.main(command), 0)
        self.assertIn("已复用", output.getvalue())
        self.assertIn("已登录", output.getvalue())

    def test_unmanaged_or_locally_edited_skill_is_preserved(self):
        self.target.mkdir(parents=True)
        local = self.target / "SKILL.md"
        local.write_text("Existing skill", encoding="utf-8")
        with self.assertRaises(installer.InstallError):
            installer.install_skill(self.project, self.source)
        with self.assertRaises(installer.InstallError):
            installer.uninstall_skill(self.project)
        self.assertEqual(local.read_text(encoding="utf-8"), "Existing skill")

        local.unlink()
        self.target.rmdir()
        installer.install_skill(self.project, self.source)
        local.write_text("Edited after install", encoding="utf-8")
        with self.assertRaises(installer.InstallError):
            installer.uninstall_skill(self.project)
        self.assertEqual(local.read_text(encoding="utf-8"), "Edited after install")

    def test_symlinked_skill_parent_cannot_write_to_another_project(self):
        (self.project / ".agents").symlink_to(
            self.other_project, target_is_directory=True
        )
        with self.assertRaises(installer.InstallError):
            installer.install_skill(self.project, self.source)
        self.assertEqual(list(self.other_project.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
