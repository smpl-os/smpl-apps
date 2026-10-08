import importlib.util
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("release_assets", ROOT / "scripts/release_assets.py")
assets = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(assets)


class ReleaseAssetsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compiler_fixture = tempfile.TemporaryDirectory(prefix="smpl-release-elf-")
        cls.binary = Path(cls.compiler_fixture.name) / "fixture"
        subprocess.run(["cc", "-x", "c", "-", "-o", str(cls.binary)],
                       input="int main(void) { return 0; }\n", text=True, check=True)

    @classmethod
    def tearDownClass(cls):
        cls.compiler_fixture.cleanup()

    def setUp(self):
        self.fixture = tempfile.TemporaryDirectory(prefix="smpl-release-test-")
        self.addCleanup(self.fixture.cleanup)
        self.root = Path(self.fixture.name)
        self.source = self.root / "release"
        self.source.mkdir()
        self.dist = self.root / "dist"
        self.bundle = self.root / "smpl-apps-0.8.23-x86_64.tar.gz"
        self.service = self.root / assets.SERVICE
        shutil.copyfile(assets.SERVICE_SOURCE, self.service)
        self.service.chmod(0o644)
        service_patch = patch.object(assets, "SERVICE_SOURCE", self.service)
        service_patch.start()
        self.addCleanup(service_patch.stop)
        for name in assets.BINARIES:
            shutil.copy2(self.binary, self.source / name)

    def pack(self):
        with tarfile.open(self.bundle, "w:gz") as archive:
            archive.add(self.dist, arcname=".")

    def upload_metadata(self):
        files = assets.required_files(self.dist)
        files[self.bundle.name] = self.bundle
        return {
            "tag_name": "v0.8.23",
            "draft": True,
            "assets": [
                {"name": name, "state": "uploaded", "size": path.stat().st_size,
                 "digest": f"sha256:{assets.digest(path)}"}
                for name, path in files.items()
            ],
        }

    def verify_metadata(self, metadata):
        path = self.root / "metadata.json"
        path.write_text(json.dumps(metadata))
        assets.verify_uploaded(self.dist, self.bundle, path)

    def test_complete_payload_and_optional_xr_are_preserved(self):
        self.assertEqual(len(assets.BINARIES), 11)
        assets.collect(self.source, self.dist)
        for name in ("xr-workspace", "xrctl", "xr-game", "xr-glasses-hotplugd", ".xr-version"):
            (self.dist / name).write_text("optional fixture")
        self.pack()
        assets.verify_bundle(self.dist, self.bundle)
        self.verify_metadata(self.upload_metadata())
        self.assertEqual((self.dist / assets.SERVICE).stat().st_mode & 0o777, 0o644)
        self.assertEqual((self.dist / assets.SERVICE).read_bytes(), self.service.read_bytes())

    def test_service_matches_explicit_foreground_entry(self):
        service = self.service.read_text()
        self.assertIn("ExecStart=/usr/local/bin/smpl-calendar-alertd --foreground\n", service)
        self.assertIn("WantedBy=default.target\n", service)

    def test_missing_or_executable_service_fails_before_collection(self):
        original = self.service.read_bytes()
        self.service.unlink()
        with self.assertRaises(ValueError):
            assets.collect(self.source, self.dist)
        self.assertFalse(self.dist.exists())
        self.service.write_bytes(original)
        self.service.chmod(0o755)
        with self.assertRaises(ValueError):
            assets.collect(self.source, self.dist)
        self.assertFalse(self.dist.exists())

    def test_service_must_be_present_nonexecutable_and_unchanged_in_bundle(self):
        assets.collect(self.source, self.dist)
        service = self.dist / assets.SERVICE
        original = service.read_bytes()
        service.unlink()
        self.pack()
        service.write_bytes(original)
        service.chmod(0o644)
        with self.assertRaises(ValueError):
            assets.verify_bundle(self.dist, self.bundle)
        service.chmod(0o755)
        self.pack()
        service.chmod(0o644)
        with self.assertRaises(ValueError):
            assets.verify_bundle(self.dist, self.bundle)
        self.pack()
        service.write_bytes(original + b"# changed\n")
        with self.assertRaises(ValueError):
            assets.verify_bundle(self.dist, self.bundle)

    def test_each_mandatory_missing_binary_fails_before_collection(self):
        for name in assets.BINARIES:
            with self.subTest(binary=name):
                path = self.source / name
                path.unlink()
                with self.assertRaises(ValueError):
                    assets.collect(self.source, self.dist)
                self.assertFalse(self.dist.exists())
                shutil.copy2(self.binary, path)

    def test_empty_nonexecutable_or_unstrippable_binary_fails(self):
        path = self.source / "settings"
        path.write_bytes(b"")
        with self.assertRaises(ValueError):
            assets.collect(self.source, self.dist)
        shutil.copy2(self.binary, path)
        path.chmod(0o600)
        with self.assertRaises(ValueError):
            assets.collect(self.source, self.dist)
        path.chmod(0o755)
        path.write_text("not an executable object")
        with self.assertRaises(subprocess.CalledProcessError):
            assets.collect(self.source, self.dist)

    def test_stale_dist_cannot_mask_outputs(self):
        self.dist.mkdir()
        with self.assertRaises(FileExistsError):
            assets.collect(self.source, self.dist)

    def test_missing_or_changed_bundle_member_fails(self):
        assets.collect(self.source, self.dist)
        settings = self.dist / "settings"
        original = settings.read_bytes()
        settings.unlink()
        self.pack()
        settings.write_bytes(original)
        settings.chmod(0o755)
        with self.assertRaises(ValueError):
            assets.verify_bundle(self.dist, self.bundle)
        self.pack()
        settings.write_bytes(original + b"changed")
        with self.assertRaises(ValueError):
            assets.verify_bundle(self.dist, self.bundle)

    def test_incomplete_or_mismatched_upload_cannot_publish(self):
        assets.collect(self.source, self.dist)
        self.pack()
        for field, value in [("state", "new"), ("size", 1), ("digest", None),
                             ("digest", "sha256:wrong")]:
            metadata = self.upload_metadata()
            metadata["assets"][0][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.verify_metadata(metadata)
        for index in range(len(assets.BINARIES) + 2):
            metadata = self.upload_metadata()
            metadata["assets"].pop(index)
            with self.subTest(missing=index), self.assertRaises(ValueError):
                self.verify_metadata(metadata)

    def test_extra_uploaded_asset_must_match(self):
        assets.collect(self.source, self.dist)
        self.pack()
        extra = self.root / "control-surface-0.8.23-x86_64.tar.gz"
        extra.write_bytes(b"asset")
        metadata = self.upload_metadata()
        path = self.root / "metadata.json"
        path.write_text(json.dumps(metadata))
        with self.assertRaises(ValueError):
            assets.verify_uploaded(self.dist, self.bundle, path, [extra])
        metadata["assets"].append({"name": extra.name, "state": "uploaded", "size": 5,
                                   "digest": f"sha256:{assets.digest(extra)}"})
        path.write_text(json.dumps(metadata))
        assets.verify_uploaded(self.dist, self.bundle, path, [extra])

    def test_wrong_tag_or_already_published_release_is_rejected(self):
        assets.collect(self.source, self.dist)
        self.pack()
        for field, value in [("tag_name", "v0.8.22"), ("draft", False)]:
            metadata = self.upload_metadata()
            metadata[field] = value
            with self.assertRaises(ValueError):
                self.verify_metadata(metadata)

    def test_workflow_publishes_only_after_payload_verification_without_rebase(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn('test "$GITHUB_REF" = refs/heads/main', workflow)
        self.assertIn("group: smpl-apps-release", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn('git rev-parse origin/main)" = "$RELEASE_SOURCE_SHA"', workflow)
        self.assertIn("git push --atomic origin HEAD:refs/heads/main", workflow)
        self.assertNotIn("git pull", workflow)
        self.assertIn("draft: true", workflow)
        self.assertIn("fail_on_unmatched_files: true", workflow)
        self.assertLess(workflow.index("release_assets.py verify"), workflow.index("git tag"))
        self.assertLess(workflow.index("release_assets.py uploaded"), workflow.index("-F draft=false"))
        for name in assets.BINARIES:
            self.assertIn(f"dist/{name}\n", workflow)
        self.assertIn(f"dist/{assets.SERVICE}\n", workflow)


if __name__ == "__main__":
    unittest.main()


class ControlSurfaceAssetTests(unittest.TestCase):
    """The keypad daemon's own release asset (a usr/ tree for packages)."""

    @classmethod
    def setUpClass(cls):
        cls.compiler_fixture = tempfile.TemporaryDirectory(prefix="smpl-cs-elf-")
        cls.binary = Path(cls.compiler_fixture.name) / "fixture"
        subprocess.run(["cc", "-x", "c", "-", "-o", str(cls.binary)],
                       input="int main(void) { return 0; }\n", text=True, check=True)

    @classmethod
    def tearDownClass(cls):
        cls.compiler_fixture.cleanup()

    def setUp(self):
        fixture = tempfile.TemporaryDirectory(prefix="smpl-cs-test-")
        self.addCleanup(fixture.cleanup)
        self.root = Path(fixture.name)
        self.install = self.root / "install"
        (self.install / "usr/bin").mkdir(parents=True)
        for name in assets.CONTROL_SURFACE_BINARIES:
            shutil.copy2(self.binary, self.install / "usr/bin" / name)
        self.source = self.root / "control-surface"
        shutil.copytree(assets.CONTROL_SURFACE_SOURCE / "firmware/release",
                        self.source / "firmware/release")
        for relative, _ in assets.CONTROL_SURFACE_DATA:
            path = self.source / relative
            if not path.exists():
                path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(assets.CONTROL_SURFACE_SOURCE / relative, path)
        self.tarball = self.root / assets.control_surface_asset("0.8.27")

    def members(self):
        with tarfile.open(self.tarball, "r:gz") as archive:
            return {member.name: member for member in archive.getmembers()}

    def test_asset_has_binaries_data_licences_and_only_current_firmware(self):
        assets.stage_control_surface(self.install, self.tarball, self.source)
        members = self.members()
        for name in assets.CONTROL_SURFACE_BINARIES:
            self.assertEqual(members[f"usr/bin/{name}"].mode, 0o755)
        for _, archived in assets.CONTROL_SURFACE_DATA:
            self.assertEqual(members[archived].mode, 0o644)
            self.assertEqual(members[archived].uid, 0)
        firmware = sorted(name for name in members if name.endswith(".bin"))
        current = [path.with_suffix(".bin").name
                   for path in sorted((self.source / "firmware/release").glob("*.json"))
                   if "supersededBy" not in json.loads(path.read_text())]
        self.assertTrue(current)
        self.assertEqual(firmware, [f"{assets.FIRMWARE_DIR}/{name}" for name in current])
        self.assertIn("usr/share/licenses/control-surface/firmware-LICENSE", members)
        self.assertIn("usr/share/licenses/control-surface/COPYING", members)

    def test_repository_firmware_manifests_match_their_images(self):
        self.assertTrue(assets.current_firmware(assets.CONTROL_SURFACE_SOURCE))

    def test_changed_firmware_image_or_missing_data_fails(self):
        image = next(path.with_suffix(".bin") for path in (self.source / "firmware/release").glob("*.json")
                     if "supersededBy" not in json.loads(path.read_text()))
        image.write_bytes(image.read_bytes() + b"\0")
        with self.assertRaises(ValueError):
            assets.stage_control_surface(self.install, self.tarball, self.source)
        image.write_bytes(image.read_bytes()[:-1])
        (self.source / "COPYING").unlink()
        with self.assertRaises(ValueError):
            assets.stage_control_surface(self.install, self.tarball, self.source)

    def test_missing_or_nonexecutable_binary_fails(self):
        daemon = self.install / "usr/bin/control-surfaced"
        daemon.chmod(0o644)
        with self.assertRaises(ValueError):
            assets.stage_control_surface(self.install, self.tarball, self.source)
        daemon.unlink()
        with self.assertRaises(ValueError):
            assets.stage_control_surface(self.install, self.tarball, self.source)

    def test_tampered_asset_fails_verification(self):
        assets.stage_control_surface(self.install, self.tarball, self.source)
        bad = self.root / "bad.tar.gz"
        with tarfile.open(self.tarball, "r:gz") as source, tarfile.open(bad, "w:gz") as target:
            for member in source.getmembers():
                if member.name.endswith(".bin"):
                    data = source.extractfile(member).read() + b"x"
                    member.size = len(data)
                    target.addfile(member, io.BytesIO(data))
                else:
                    target.addfile(member, source.extractfile(member))
        with self.assertRaises(ValueError):
            assets.verify_control_surface(bad)

    def test_release_workflow_builds_tests_and_uploads_the_asset(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("ctest --test-dir", workflow)
        self.assertIn("release_assets.py control-surface", workflow)
        self.assertIn("${{ env.CS_ASSET }}", workflow)
        self.assertIn('--asset "$CS_ASSET"', workflow)
        self.assertLess(workflow.index("release_assets.py control-surface"), workflow.index("git tag"))
        self.assertNotIn("Co-authored-by", workflow)
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("ctest --test-dir", ci)
