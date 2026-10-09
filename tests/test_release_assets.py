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
import re

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


def qt_fixture(directory, minor=11, private=False, tagged=True, rpath=None):
    """An executable linked against a stub libQt6Core.so.6 the way Qt tags
    its users: a reference to qt_version_tag at version Qt_6.<minor>."""
    directory.mkdir(parents=True)
    script = directory / "qt.map"
    nodes = f"Qt_6.{minor} {{ global: qt_version_tag; local: *; }};\n" if tagged else ""
    if private:
        nodes += "Qt_6_PRIVATE_API { global: qt_private; };\n"
    script.write_text(nodes or "{ global: *; };\n")
    (directory / "qt.c").write_text("const char qt_version_tag = 0;\nconst char qt_private = 0;\n")
    library = ["cc", "-shared", "-fPIC", "-o", str(directory / "libQt6Core.so.6"),
               "-Wl,-soname,libQt6Core.so.6", str(directory / "qt.c")]
    if tagged or private:
        library.append(f"-Wl,--version-script={script}")
    subprocess.run(library, check=True)
    uses = "qt_version_tag + qt_private" if private else "qt_version_tag"
    (directory / "main.c").write_text(
        f"extern const char qt_version_tag, qt_private;\nint main(void) {{ return {uses}; }}\n")
    binary = directory / "fixture"
    command = ["cc", "-o", str(binary), str(directory / "main.c"), str(directory / "libQt6Core.so.6")]
    if rpath:
        command.append(f"-Wl,-rpath,{rpath}")
    subprocess.run(command, check=True)
    return binary


class ControlSurfaceRuntimeTests(unittest.TestCase):
    """The asset's runtime contract: it must run on smplOS's Qt baseline."""

    @classmethod
    def setUpClass(cls):
        cls.fixtures = tempfile.TemporaryDirectory(prefix="smpl-cs-qt-")
        root = Path(cls.fixtures.name)
        cls.baseline = qt_fixture(root / "baseline", minor=assets.CONTROL_SURFACE_MAX_QT[1])
        cls.newer = qt_fixture(root / "newer", minor=assets.CONTROL_SURFACE_MAX_QT[1] + 1)
        cls.untagged = qt_fixture(root / "untagged", tagged=False)
        cls.private = qt_fixture(root / "private", private=True)
        cls.rpath = qt_fixture(root / "rpath", rpath="/opt/qt/lib")

    @classmethod
    def tearDownClass(cls):
        cls.fixtures.cleanup()

    def setUp(self):
        fixture = tempfile.TemporaryDirectory(prefix="smpl-cs-runtime-")
        self.addCleanup(fixture.cleanup)
        self.root = Path(fixture.name)
        self.source = self.root / "control-surface"
        shutil.copytree(assets.CONTROL_SURFACE_SOURCE / "firmware/release", self.source / "firmware/release")
        for relative, _ in assets.CONTROL_SURFACE_DATA:
            path = self.source / relative
            if not path.exists():
                path.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(assets.CONTROL_SURFACE_SOURCE / relative, path)

    def stage(self, binary):
        install = self.root / f"install-{binary.parent.name}"
        (install / "usr/bin").mkdir(parents=True)
        for name in assets.CONTROL_SURFACE_BINARIES:
            shutil.copy2(binary, install / "usr/bin" / name)
        tarball = self.root / f"{binary.parent.name}.tar.gz"
        assets.stage_control_surface(install, tarball, self.source)
        return tarball

    def test_baseline_qt_passes_and_is_reported(self):
        problems, summary = assets.elf_runtime(self.baseline, "fixture")
        self.assertEqual(problems, [])
        self.assertEqual(summary["qt"], assets.CONTROL_SURFACE_MAX_QT)
        summaries = assets.check_control_surface(self.stage(self.baseline))
        self.assertEqual(set(summaries), set(assets.CONTROL_SURFACE_BINARIES))

    def test_asset_built_on_newer_qt_is_rejected(self):
        # v0.8.27 shipped binaries built on rolling Arch's Qt 6.12; smplOS's
        # Qt 6.11.2 refused them ("version `Qt_6.12' not found").
        problems, _ = assets.elf_runtime(self.newer, "fixture")
        self.assertTrue(any(f"Qt 6.{assets.CONTROL_SURFACE_MAX_QT[1] + 1}" in p for p in problems), problems)
        with self.assertRaisesRegex(ValueError, "newer than the supported Qt"):
            assets.check_control_surface(self.stage(self.newer))

    def test_disabled_version_tagging_private_api_and_rpath_are_rejected(self):
        for binary, reason in ((self.untagged, "version tagging must stay enabled"),
                               (self.private, "private API"),
                               (self.rpath, "RPATH/RUNPATH")):
            problems, _ = assets.elf_runtime(binary, "fixture")
            self.assertTrue(any(reason in p for p in problems), (binary, problems))

    def test_glibc_ceiling_and_readelf_parsing(self):
        text = """Version needs section '.gnu.version_r' contains 2 entries:
  000000: Version: 1  File: libQt6Core.so.6  Cnt: 2
  0x0010:   Name: Qt_6.11  Flags: none  Version: 4
  0x0020:   Name: Qt_6  Flags: none  Version: 3
  0x0030: Version: 1  File: libc.so.6  Cnt: 1
  0x0040:   Name: GLIBC_2.38  Flags: none  Version: 2
"""
        needs = assets.parse_version_needs(text)
        self.assertEqual(needs, [("libQt6Core.so.6", "Qt_6.11"), ("libQt6Core.so.6", "Qt_6"), ("libc.so.6", "GLIBC_2.38")])
        problems, summary = assets.runtime_problems("x", ["libQt6Core.so.6", "libc.so.6"], [], needs)
        self.assertEqual((problems, summary["glibc"]), ([], (2, 38)))
        newer = "GLIBC_%d.%d" % (assets.CONTROL_SURFACE_MAX_GLIBC[0], assets.CONTROL_SURFACE_MAX_GLIBC[1] + 1)
        problems, _ = assets.runtime_problems("x", ["libc.so.6"], [], [("libc.so.6", newer)])
        self.assertTrue(problems and "glibc" in problems[0])
        needed, paths = assets.parse_dynamic(
            " 0x1 (NEEDED)  Shared library: [libQt6Core.so.6]\n 0x1d (RUNPATH)  Library runpath: [/x]\n")
        self.assertEqual((needed, paths), (["libQt6Core.so.6"], ["/x"]))

    def test_smoke_runs_only_as_a_regular_user(self):
        with patch.object(assets.os, "geteuid", return_value=0):
            with self.assertRaisesRegex(ValueError, "regular user"):
                assets.smoke_control_surface(self.root / "unused.tar.gz")

    def test_release_workflow_builds_and_smoke_tests_on_the_pinned_baseline(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        build, release = workflow.split("\n  bump-and-release:\n", 1)
        build = build.split("\n  control-surface:\n", 1)[1]
        self.assertRegex(build, r'ARCH_SNAPSHOT: "\d{4}/\d{2}/\d{2}"')
        self.assertIn("https://archive.archlinux.org/repos/%s/$repo/os/$arch", build)
        self.assertIn("pacman -Syyuu", build)
        package = re.search(r'CONTROL_SURFACE_QT_PACKAGE: "qt6-base (\d+)\.(\d+)\.\d+-\d+"', build)
        self.assertEqual((int(package[1]), int(package[2])), assets.CONTROL_SURFACE_MAX_QT)
        self.assertIn('test "$(pacman -Q qt6-base)" = "$CONTROL_SURFACE_QT_PACKAGE"', build)
        self.assertIn("-DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr -DBUILD_TESTING=ON", build)
        self.assertIn("runuser -u tester -- ctest --test-dir build-control-surface", build)
        smoke = build.index("runuser -u tester -- python scripts/release_assets.py control-surface-smoke")
        self.assertLess(build.index("release_assets.py control-surface control-surface-install"), smoke)
        self.assertLess(smoke, build.index("actions/upload-artifact"))
        # The release job publishes that exact build; rolling Arch never rebuilds it.
        self.assertIn("needs: control-surface", release)
        self.assertIn("ref: ${{ needs.control-surface.outputs.source-sha }}", release)
        self.assertNotIn("cmake -S control-surface", release)
        self.assertNotIn("qt6-base", release)
        self.assertLess(release.index('release_assets.py control-surface-check "$CS_ASSET"'), release.index("git tag"))
        self.assertLess(release.index("control-surface-check"), release.index("action-gh-release"))

    def test_qt_version_tagging_is_never_disabled(self):
        sources = [ROOT / ".github/workflows/release.yml", ROOT / ".github/workflows/ci.yml"]
        sources += [path for path in (ROOT / "control-surface").rglob("*")
                    if path.suffix in {".txt", ".cmake", ".cpp", ".h", ".sh"} and path.is_file()]
        for path in sources:
            self.assertNotIn("QT_NO_VERSION_TAGGING", path.read_text(errors="replace"), path)

