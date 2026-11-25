import hashlib
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock
from urllib.error import HTTPError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import milkbird_release as release


class ReleaseTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.download_dir = Path(temporary.name).resolve()
        self.payload = b"a" * (release.CHUNK_SIZE + 10)
        self.arm = self.artifact("arm64", "14")
        self.intel = self.artifact("x86_64", "13.0")

    def artifact(self, arch, minimum):
        return {
            "os": "macos",
            "arch": arch,
            "minimumOsVersion": minimum,
            "url": f"{release.RELEASES_URL}/download/v0.1.0/MilkBird-{arch}.dmg",
            "sha256": hashlib.sha256(self.payload).hexdigest(),
        }

    def test_selects_native_architecture_and_respects_minimum_os(self):
        manifest = {"schemaVersion": 1, "artifacts": [self.arm, self.intel]}
        self.assertEqual(release.select_artifact(manifest, "arm64", "14.0"), self.arm)
        self.assertEqual(release.select_artifact(manifest, "x86_64", "13"), self.intel)
        with self.assertRaisesRegex(release.ReleaseError, "要求 macOS 14"):
            release.select_artifact(manifest, "arm64", "13.6.8")
        with self.assertRaisesRegex(release.ReleaseError, "多个安装包"):
            release.select_artifact({"schemaVersion": 1, "artifacts": [self.arm, self.arm]}, "arm64", "14")

    def test_release_asset_must_be_a_fixed_github_release(self):
        for url in (
            "https://other.example/MilkBird.dmg",
            f"{release.RELEASES_URL}/latest/download/MilkBird.dmg",
            f"{release.RELEASES_URL}/download/v0.1.0/../MilkBird.dmg",
        ):
            with self.subTest(url=url), self.assertRaises(release.ReleaseError):
                release.artifact_filename(url)

    def test_missing_manifest_reports_public_release_status(self):
        missing = HTTPError(release.MANIFEST_URL, 404, "Not Found", None, None)
        with mock.patch.object(release, "urlopen", side_effect=missing):
            with self.assertRaisesRegex(release.ReleaseError, "没有公开的安装包索引"):
                release.get_manifest()

    def test_download_verifies_once_then_reuses_verified_file(self):
        with mock.patch.object(release, "mac_host", return_value=("arm64", "14.0")), \
             mock.patch.object(release, "urlopen", return_value=io.BytesIO(self.payload)) as fetch, \
             mock.patch.object(release, "verify_native_image") as verify:
            path = release.download_artifact(self.arm, self.download_dir)
            self.assertEqual(path.read_bytes(), self.payload)
            self.assertEqual(release.download_artifact(self.arm, self.download_dir), path)
            fetch.assert_called_once()
            self.assertEqual(verify.call_count, 2)
            path.write_bytes(b"already exists")
            with self.assertRaisesRegex(release.ReleaseError, "已存在不同"):
                release.download_artifact(self.arm, self.download_dir)
            self.assertEqual(path.read_bytes(), b"already exists")
        self.assertEqual(list(self.download_dir.iterdir()), [path])

    def test_failed_checks_and_interruption_leave_no_installable_file(self):
        with mock.patch.object(release, "mac_host", return_value=("arm64", "14.0")), \
             mock.patch.object(release, "urlopen", return_value=io.BytesIO(b"unexpected")) as fetch, \
             mock.patch.object(release, "verify_native_image") as verify:
            with self.assertRaisesRegex(release.ReleaseError, "SHA-256"):
                release.download_artifact(self.arm, self.download_dir)
            verify.assert_not_called()
            self.assertEqual(list(self.download_dir.iterdir()), [])

            fetch.side_effect = KeyboardInterrupt()
            with self.assertRaises(KeyboardInterrupt):
                release.download_artifact(self.arm, self.download_dir)
            self.assertEqual(list(self.download_dir.iterdir()), [])

            fetch.side_effect = None
            fetch.return_value = io.BytesIO(self.payload)
            verify.side_effect = release.ReleaseError("Gatekeeper rejected")
            with self.assertRaisesRegex(release.ReleaseError, "Gatekeeper"):
                release.download_artifact(self.arm, self.download_dir)
            self.assertEqual(list(self.download_dir.iterdir()), [])

    def test_native_image_requires_notarized_developer_id(self):
        with mock.patch.object(release.subprocess, "run", side_effect=[
            subprocess.CompletedProcess([], 0, "", ""),
            subprocess.CompletedProcess([], 0, "", "source=Notarized Developer ID\n"),
        ]) as run:
            release.verify_native_image(self.download_dir / "MilkBird.dmg")
            self.assertEqual(run.call_args_list[0].args[0][1], "verify")
            self.assertIn("context:primary-signature", run.call_args_list[1].args[0])
        with mock.patch.object(release.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "", "")):
            with self.assertRaisesRegex(release.ReleaseError, "Developer ID"):
                release.verify_native_image(self.download_dir / "MilkBird.dmg")
