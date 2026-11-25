import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import prepare_macos_release as release


class ReleasePreparationTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.app = self.root / "MilkBird Tray.app"
        self.app.mkdir()
        self.output = self.root / "output"
        self.metadata = mock.patch.object(release, "app_metadata", return_value=("0.1.0", "11.0"))
        self.metadata.start()
        self.addCleanup(self.metadata.stop)

    def commands(self, *args, **_kwargs):
        if args[:2] == ("/usr/bin/hdiutil", "create"):
            Path(args[-1]).write_bytes(b"disk image fixture")
        return "", ""

    def test_unsigned_preview_never_creates_public_index(self):
        with mock.patch.object(release, "run", side_effect=self.commands) as run:
            image = release.prepare(self.app, self.output, None)
        self.assertTrue(image.name.endswith("-preview.dmg"))
        self.assertEqual(list(self.output.iterdir()), [image])
        self.assertIn("verify", run.call_args.args)

    def test_release_requires_matching_tag_and_credentials_before_writing(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(release.ReleaseError, "发布标签"):
                release.prepare(self.app, self.output, "v0.2.0")
            with self.assertRaisesRegex(release.ReleaseError, "缺少签名"):
                release.prepare(self.app, self.output, "v0.1.0")
        self.assertFalse(self.output.exists())

    def test_manifest_hashes_the_final_verified_image(self):
        signature = b"disk image after notarization and stapling"

        def notarize(image, _credentials):
            image.write_bytes(signature)

        with mock.patch.object(release, "run", side_effect=self.commands), \
             mock.patch.object(release, "signing_credentials", return_value=("fixture",)), \
             mock.patch.object(release, "verify_release_app") as verify, \
             mock.patch.object(release, "notarize_image", side_effect=notarize):
            image = release.prepare(self.app, self.output, "v0.1.0")
        verify.assert_called_once_with(self.app.resolve(), "fixture")
        metadata = json.loads((self.output / release.MANIFEST_NAME).read_text(encoding="utf-8"))
        self.assertEqual(metadata, {
            "schemaVersion": 1,
            "artifacts": [{
                "os": "macos",
                "arch": "arm64",
                "minimumOsVersion": "11.0",
                "url": f"{release.RELEASES_URL}/download/v0.1.0/{image.name}",
                "sha256": hashlib.sha256(signature).hexdigest(),
            }],
        })

    def test_apple_rejection_never_advances_to_stapling(self):
        with mock.patch.object(release, "run", side_effect=[("", ""), ('{"status":"Invalid"}', "")]) as run:
            with self.assertRaisesRegex(release.ReleaseError, "尚未接受"):
                release.notarize_image(self.root / "image.dmg", ("identity", "/fixture.p8", "key", "issuer"))
        self.assertEqual(run.call_count, 2)
