#!/usr/bin/env python3
"""Collect required apps and verify local and uploaded release payloads.

Also stages control-surface (the keypad daemon, built by CMake) as its own
release asset, laid out under usr/ for packages: smplOS installs it with
pacman rather than copying its binary into /usr/local/bin with the apps.
The asset links the system Qt, so it must be built against the oldest Qt it
supports: `control-surface-check` enforces that runtime contract on the ELF
files and `control-surface-smoke` runs them on the baseline system.
"""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile


BINARIES = (
    "start-menu", "notif-center", "settings", "app-center", "webapp-center",
    "sync-center-daemon", "sync-center-gui", "smpl-calendar",
    "smpl-calendar-alertd", "smpl-hints", "smpl-hintsd",
)
LINKAGE_MANIFEST = "linkage-manifest.json"
SERVICE = "smpl-calendar-alertd.service"
SERVICE_SOURCE = Path(__file__).resolve().parents[1] / "calendar/systemd" / SERVICE
CALENDAR_SOURCE = Path(__file__).resolve().parents[1] / "calendar"
CALENDAR_BINARIES = ("smpl-calendar", "smpl-calendar-alertd")

CONTROL_SURFACE_SOURCE = Path(__file__).resolve().parents[1] / "control-surface"
CONTROL_SURFACE_BINARIES = ("control-surfaced", "ch552-padprog", "mouse-remapperd")
CONTROL_SURFACE_LINKAGE_MANIFEST = "usr/share/control-surface/linkage-manifest.json"
FIRMWARE_DIR = "usr/share/control-surface/firmware"
# Broad desktop/toolkit libraries deliberately come from the smplOS baseline.
# ABI-sensitive application/media libraries (libjxl, sndio, ffmpeg, OpenCV,
# WebKit extension libraries, and private build products) are private unless
# explicitly admitted here and must be shipped beside the executables.
SYSTEM_SONAMES = (
    r"ld-linux-x86-64\.so\.2",
    r"libc\.so\.6", r"libm\.so\.6", r"libdl\.so\.2",
    r"libpthread\.so\.0", r"librt\.so\.1", r"libutil\.so\.1",
    r"libgcc_s\.so\.1", r"libstdc\+\+\.so\.6",
    r"libfontconfig\.so\.1", r"libfreetype\.so\.6",
    r"libxkbcommon(?:-x11)?\.so\.0",
    r"libwayland-(?:client|cursor|egl)\.so\.\d+",
    r"libX(?:11|ext|cursor|fixes|randr|render|inerama|i|au|dmcp|xcb)\.so\..+",
    r"libxcb(?:-[A-Za-z0-9_-]+)?\.so\.\d+",
    r"libglib-2\.0\.so\.0", r"libg(?:io|object|module)-2\.0\.so\.0",
    r"libgtk-4\.so\.1", r"libgtk4-layer-shell\.so\.0",
    r"libadwaita-1\.so\.0", r"libgdk_pixbuf-2\.0\.so\.0",
    r"libpango(?:cairo|-ft2|-1\.0)?-1?\.0?\.so\.0",
    r"libWebKitGTK-\d+\.\d+\.so\.\d+", r"libjavascriptcoregtk-\d+\.\d+\.so\.\d+",
    r"lib(?:EGL|GLX|OpenGL|GLESv[12])\.so\.\d+", r"libgbm\.so\.1",
    r"libdrm(?:_[A-Za-z0-9_]+)?\.so\.\d+", r"libvulkan\.so\.1",
    r"libQt6[A-Za-z0-9_]+\.so\.6",
    r"libdbus-1\.so\.3", r"libsystemd\.so\.0", r"libudev\.so\.1",
    r"libz\.so\.1", r"libbz2\.so\.1\.0", r"liblzma\.so\.5", r"libzstd\.so\.1",
    r"libexpat\.so\.1", r"libffi\.so\.8", r"libpcre2-8\.so\.0",
    r"libharfbuzz(?:-gobject)?\.so\.0",
    r"libsqlite3\.so\.0", r"libssl\.so\.3", r"libcrypto\.so\.3",
)
# Runtime baseline of the control-surface asset: the oldest Qt and glibc it
# must run on (smplOS's installed qt6-base 6.11.2 and glibc 2.44). A Qt
# binary needs its build's Qt minor or later (the qt_version_tag ABI guard),
# so release.yml builds and smoke-tests it in the Arch Linux Archive snapshot
# that has exactly this Qt. Raise both together, with that snapshot, only
# when every supported smplOS has the newer Qt.
CONTROL_SURFACE_MAX_QT = (6, 11)
CONTROL_SURFACE_MAX_GLIBC = (2, 44)
SMOKE_TIMEOUT = 30
# (source path, archive path); all mandatory.
CONTROL_SURFACE_DATA = (
    ("data/config.example.jsonc", "usr/share/control-surface/config.example.jsonc"),
    ("data/systemd/control-surface.service", "usr/share/control-surface/examples/control-surface.service"),
    ("data/udev/71-wch-isp-bootloader.rules", "usr/share/control-surface/examples/71-wch-isp-bootloader.rules"),
    ("firmware/release/README.md", f"{FIRMWARE_DIR}/README.md"),
    ("firmware/release/LICENSE", f"{FIRMWARE_DIR}/LICENSE"),
    ("firmware/release/LICENSE", "usr/share/licenses/control-surface/firmware-LICENSE"),
    ("COPYING", "usr/share/licenses/control-surface/COPYING"),
    ("README.md", "usr/share/doc/control-surface/README.md"),
    ("docs/mouse-remapping.md", "usr/share/doc/control-surface/docs/mouse-remapping.md"),
    ("USER-QUICKSTART.md", "usr/share/doc/control-surface/USER-QUICKSTART.md"),
)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def required_binaries(directory):
    files = {name: directory / name for name in BINARIES}
    for name, path in files.items():
        if (path.is_symlink() or not path.is_file() or not path.stat().st_size
                or not path.stat().st_mode & 0o111):
            raise ValueError(f"Missing or invalid mandatory binary: {name}")
    return files


def validate_service(path):
    if (path.is_symlink() or not path.is_file() or not path.stat().st_size
            or path.stat().st_mode & 0o777 != 0o644):
        raise ValueError(f"Missing or invalid mandatory service: {SERVICE}")


def required_files(directory):
    files = required_binaries(directory)
    service = directory / SERVICE
    validate_service(service)
    files[SERVICE] = service
    manifest = directory / LINKAGE_MANIFEST
    if manifest.is_symlink() or not manifest.is_file() or not manifest.stat().st_size:
        raise ValueError(f"Missing or invalid mandatory linkage manifest: {LINKAGE_MANIFEST}")
    files[LINKAGE_MANIFEST] = manifest
    return files


def collect(source, destination):
    files = required_binaries(source)
    validate_service(SERVICE_SOURCE)
    destination.mkdir()  # A stale dist directory must not hide missing outputs.
    for name, path in files.items():
        target = destination / name
        shutil.copy2(path, target)
        subprocess.run(["strip", str(target)], check=True)
    source_lib = source / "lib"
    if source_lib.exists():
        if source_lib.is_symlink() or not source_lib.is_dir():
            raise ValueError("Private library source must be a real directory")
        target_lib = destination / "lib"
        shutil.copytree(source_lib, target_lib, symlinks=False)
        for path in target_lib.rglob("*"):
            if path.is_symlink():
                raise ValueError(f"Private library must not be a symlink: {path.relative_to(destination)}")
            if path.is_file() and path.read_bytes()[:4] == b"\x7fELF":
                subprocess.run(["strip", str(path)], check=True)
    shutil.copyfile(SERVICE_SOURCE, destination / SERVICE)
    (destination / SERVICE).chmod(0o644)
    audit_release(destination, write_manifest=True)
    required_files(destination)


def verify_bundle(directory, bundle):
    audit_release(directory)
    files = {path.relative_to(directory).as_posix(): path
             for path in directory.rglob("*") if path.is_file()}
    with tarfile.open(bundle, "r:gz") as archive:
        members = {}
        for member in archive.getmembers():
            name = member.name.removeprefix("./")
            if name in members:
                raise ValueError(f"Duplicate archive member: {name}")
            members[name] = member
        archived_files = {name: member for name, member in members.items() if member.isfile()}
        if set(archived_files) != set(files):
            missing = sorted(set(files) - set(archived_files))
            extra = sorted(set(archived_files) - set(files))
            raise ValueError(f"Bundle file set differs (missing={missing}, extra={extra})")
        for name, path in files.items():
            member = members.get(name)
            if member is None or not member.isfile():
                raise ValueError(f"Missing or invalid bundled asset: {name}")
            if name == SERVICE:
                if member.mode & 0o777 != 0o644:
                    raise ValueError(f"Bundled service must have mode 0644: {name}")
            elif path.name in BINARIES and not member.mode & 0o111:
                raise ValueError(f"Bundled binary must be executable: {name}")
            with archive.extractfile(member) as stream:
                archived = hashlib.file_digest(stream, "sha256").hexdigest()
            if archived != digest(path):
                raise ValueError(f"Bundle differs from standalone asset: {name}")


def control_surface_asset(version):
    return f"control-surface-{version}-x86_64.tar.gz"


def build_calendar_rpm(source, destination, version):
    """Build a native RPM from calendar-only Fedora binaries, never Arch ones."""
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError(f"Invalid calendar RPM version: {version}")
    files = {}
    for name in CALENDAR_BINARIES:
        path = source / name
        if (path.is_symlink() or not path.is_file() or not path.stat().st_size
                or not path.stat().st_mode & 0o111):
            raise ValueError(f"Missing or invalid calendar binary: {name}")
        output = subprocess.check_output([str(path.resolve()), "--version"], text=True).strip()
        expected = f"{name} v{version}" if name == "smpl-calendar" else f"{name} {version}"
        if output != expected:
            raise ValueError(f"Calendar RPM binary version mismatch: {output}")
        files[name] = path
    validate_service(SERVICE_SOURCE)
    service = SERVICE_SOURCE.read_text()
    entry = "ExecStart=/usr/local/bin/smpl-calendar-alertd --foreground\n"
    if service.count(entry) != 1:
        raise ValueError("Calendar service no longer has its expected executable entry")
    service = service.replace(
        entry, "ExecStart=/usr/bin/smpl-calendar-alertd --foreground --persistent\n")
    files["smpl-calendar.desktop"] = CALENDAR_SOURCE / "packaging/smpl-calendar.desktop"
    files["smpl-calendar-reminders.desktop"] = CALENDAR_SOURCE / "packaging/smpl-calendar-reminders.desktop"
    files["setup-kde-shortcut.py"] = CALENDAR_SOURCE / "packaging/setup-kde-shortcut.py"
    files["DSEG-LICENSE.txt"] = CALENDAR_SOURCE / "ui/assets/DSEG-LICENSE.txt"
    calendar_linkage = linkage_manifest(
        source,
        [files[name] for name in CALENDAR_BINARIES],
        private_library_index(source),
    )
    calendar_linkage_bytes = (
        json.dumps(calendar_linkage, indent=2, sort_keys=True) + "\n"
    ).encode()
    destination.mkdir(parents=True, exist_ok=False)
    topdir = destination.resolve()
    (topdir / "SOURCES").mkdir()
    with tarfile.open(topdir / "SOURCES/smpl-calendar.tar.gz", "w:gz") as archive:
        for name, path in files.items():
            info = archive.gettarinfo(path, arcname=f"smpl-calendar/{name}")
            info.mode = 0o755 if name in CALENDAR_BINARIES else 0o644
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            with path.open("rb") as stream:
                archive.addfile(info, stream)
        data = service.encode()
        info = tarfile.TarInfo(f"smpl-calendar/{SERVICE}")
        info.size = len(data)
        info.mode = 0o644
        archive.addfile(info, io.BytesIO(data))
        info = tarfile.TarInfo("smpl-calendar/linkage-manifest.json")
        info.size = len(calendar_linkage_bytes)
        info.mode = 0o644
        archive.addfile(info, io.BytesIO(calendar_linkage_bytes))
    subprocess.run([
        "rpmbuild", "-bb", "--define", f"_topdir {topdir}",
        "--define", f"calendar_version {version}",
        str(CALENDAR_SOURCE / "packaging/smpl-calendar.spec"),
    ], check=True)
    packages = list((topdir / "RPMS/x86_64").glob(f"smpl-calendar-{version}-1*.x86_64.rpm"))
    if len(packages) != 1:
        raise ValueError("Expected exactly one calendar RPM")
    print(f"Calendar RPM: {packages[0]}")


def current_firmware(source):
    """Firmware images a wizard may offer: each .bin with a matching manifest,
    leaving out superseded ones (kept in the source tree for reference)."""
    images = {}
    for manifest in sorted((source / "firmware/release").glob("*.json")):
        info = json.loads(manifest.read_text())
        if info.get("supersededBy"):
            continue
        image = manifest.with_suffix(".bin")
        if (not image.is_file() or info.get("sha256") != digest(image)
                or info.get("size") != image.stat().st_size
                or info.get("license") != "CC-BY-SA-3.0"):
            raise ValueError(f"Firmware image does not match its manifest: {image.name}")
        images[f"{FIRMWARE_DIR}/{image.name}"] = image
        images[f"{FIRMWARE_DIR}/{manifest.name}"] = manifest
    if not images:
        raise ValueError("No current firmware image to release")
    return images


def stage_control_surface(install_root, tarball, source=CONTROL_SURFACE_SOURCE):
    """Package `cmake --install` output (DESTDIR, prefix /usr) with the data,
    current firmware, docs and licences into a usr/ tree tarball."""
    members = {}
    for name in CONTROL_SURFACE_BINARIES:
        path = install_root / "usr/bin" / name
        if path.is_symlink() or not path.is_file() or not path.stat().st_mode & 0o111:
            raise ValueError(f"Missing or invalid control-surface binary: {name}")
        subprocess.run(["strip", str(path)], check=True)
        members[f"usr/bin/{name}"] = path
    for relative, archived in CONTROL_SURFACE_DATA:
        path = source / relative
        if path.is_symlink() or not path.is_file() or not path.stat().st_size:
            raise ValueError(f"Missing control-surface file: {relative}")
        members[archived] = path
    members.update(current_firmware(source))
    private_libraries = private_library_index(install_root)
    elf_paths = [install_root / "usr/bin" / name for name in CONTROL_SURFACE_BINARIES]
    linkage = linkage_manifest(install_root, elf_paths, private_libraries)
    linkage_bytes = (json.dumps(linkage, indent=2, sort_keys=True) + "\n").encode()

    def normalize(info):
        info.uid = info.gid = 0
        info.uname = info.gname = "root"
        info.mode = 0o755 if info.name.startswith("usr/bin/") else 0o644
        return info

    with tarfile.open(tarball, "w:gz") as archive:
        for archived, path in sorted(members.items()):
            archive.add(path, arcname=archived, filter=normalize)
        info = tarfile.TarInfo(CONTROL_SURFACE_LINKAGE_MANIFEST)
        info.size = len(linkage_bytes)
        normalize(info)
        archive.addfile(info, io.BytesIO(linkage_bytes))
    verify_control_surface(tarball)


def verify_control_surface(tarball):
    with tarfile.open(tarball, "r:gz") as archive:
        members = {member.name: member for member in archive.getmembers()}
        for name in CONTROL_SURFACE_BINARIES:
            member = members.get(f"usr/bin/{name}")
            if member is None or not member.isfile() or not member.mode & 0o111:
                raise ValueError(f"Missing or invalid control-surface binary: {name}")
            with archive.extractfile(member) as stream:
                if stream.read(4) != b"\x7fELF":
                    raise ValueError(f"control-surface binary is not ELF: {name}")
        for _, archived in CONTROL_SURFACE_DATA:
            member = members.get(archived)
            if member is None or not member.isfile() or not member.size:
                raise ValueError(f"Missing control-surface file: {archived}")
        images = [name for name in members
                  if name.startswith(f"{FIRMWARE_DIR}/") and name.endswith(".bin")]
        if not images:
            raise ValueError("No firmware image in the control-surface asset")
        for name in images:
            manifest = members.get(name.removesuffix(".bin") + ".json")
            if manifest is None:
                raise ValueError(f"Firmware image without manifest: {name}")
            with archive.extractfile(manifest) as stream:
                info = json.load(stream)
            with archive.extractfile(members[name]) as stream:
                actual = hashlib.file_digest(stream, "sha256").hexdigest()
            if info.get("supersededBy") or info.get("sha256") != actual:
                raise ValueError(f"Firmware image does not match its manifest: {name}")
        manifest_member = members.get(CONTROL_SURFACE_LINKAGE_MANIFEST)
        if manifest_member is None or not manifest_member.isfile() or not manifest_member.size:
            raise ValueError(f"Missing control-surface linkage manifest: {CONTROL_SURFACE_LINKAGE_MANIFEST}")
        with tempfile.TemporaryDirectory(prefix="control-surface-linkage-") as scratch:
            with tarfile.open(tarball, "r:gz") as source:
                source.extractall(scratch, filter="data")
            root = Path(scratch)
            elf_paths = [root / "usr/bin" / name for name in CONTROL_SURFACE_BINARIES]
            expected = linkage_manifest(root, elf_paths, private_library_index(root))
            encoded = (json.dumps(expected, indent=2, sort_keys=True) + "\n").encode()
            if (root / CONTROL_SURFACE_LINKAGE_MANIFEST).read_bytes() != encoded:
                raise ValueError("Control-surface linkage manifest is stale or mismatched")


def version_tuple(text):
    return tuple(int(part) for part in text.split("."))


def parse_dynamic(text):
    """(DT_NEEDED names, RPATH/RUNPATH values) from `readelf -d -W`."""
    needed = re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", text)
    paths = re.findall(r"\((?:RPATH|RUNPATH)\)\s+Library (?:rpath|runpath): \[([^\]]*)\]", text)
    return needed, paths


def parse_dynamic_tags(text):
    tags = re.findall(
        r"\((RPATH|RUNPATH)\)\s+Library (?:rpath|runpath): \[([^\]]*)\]",
        text,
    )
    return [(tag, value) for tag, value in tags]


def parse_version_needs(text):
    """[(library, version name)] from the version needs in `readelf -V -W`."""
    needs = []
    library = None
    for line in text.splitlines():
        if match := re.search(r"\bFile: (\S+)", line):
            library = match[1]
        elif (match := re.search(r"\bName: (\S+)", line)) and library:
            needs.append((library, match[1]))
    return needs


def is_elf(path):
    try:
        with path.open("rb") as stream:
            return stream.read(4) == b"\x7fELF"
    except OSError:
        return False


def is_system_soname(soname):
    return any(re.fullmatch(pattern, soname) for pattern in SYSTEM_SONAMES)


def readelf_output(path, *options):
    return subprocess.run(
        ["readelf", *options, "-W", str(path)],
        check=True, capture_output=True, text=True,
    ).stdout


def private_library_index(root):
    lib = root / "lib"
    result = {}
    if not lib.exists():
        return result
    if lib.is_symlink() or not lib.is_dir():
        raise ValueError("Bundled lib/ must be a real directory")
    for path in sorted(lib.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"Private libraries must not use symlinks: {path.relative_to(root)}")
        if not path.is_file() or not is_elf(path):
            continue
        dynamic = readelf_output(path, "-d")
        sonames = re.findall(r"\(SONAME\)\s+Library soname: \[([^\]]+)\]", dynamic)
        if len(sonames) != 1:
            raise ValueError(f"Private library lacks exactly one SONAME: {path.relative_to(root)}")
        soname = sonames[0]
        if path.relative_to(lib).as_posix() != soname:
            raise ValueError(
                f"Private library must use its exact versioned SONAME at lib/{soname}, "
                f"not {path.relative_to(root)}"
            )
        if soname in result:
            raise ValueError(f"Duplicate bundled private SONAME: {soname}")
        result[soname] = path
    unexpected = [
        path.relative_to(root).as_posix()
        for path in lib.rglob("*")
        if path.is_file() and not is_elf(path)
    ]
    if unexpected:
        raise ValueError(f"Unexpected non-ELF files under lib/: {unexpected}")
    return result


def safe_origin_path(path):
    components = path.split(":") if path else [""]
    return bool(path) and all(
        component in ("$ORIGIN", "$ORIGIN/lib", "$ORIGIN/../lib")
        for component in components
    )


def parse_ldd_paths(text):
    resolved = {}
    for match in re.finditer(
        r"^\s*(\S+)\s+=>\s+(\S+)\s+\(0x[0-9a-fA-F]+\)\s*$",
        text, re.MULTILINE,
    ):
        resolved[match[1]] = match[2]
    return resolved


def audit_elf(path, root, private_libraries):
    relative = path.relative_to(root).as_posix()
    dynamic = readelf_output(path, "-d")
    needed, legacy_paths = parse_dynamic(dynamic)
    tags = parse_dynamic_tags(dynamic)
    needs = parse_version_needs(readelf_output(path, "-V"))
    problems = []
    for tag, value in tags:
        if tag == "RPATH":
            problems.append(f"{relative}: RPATH is forbidden (use private-closure-free linking or a safe RUNPATH)")
        elif not safe_origin_path(value):
            problems.append(f"{relative}: unsafe RUNPATH {value!r}; only $ORIGIN-relative paths are allowed")
    dependencies = []
    for soname in needed:
        if soname in private_libraries:
            dependency_type = "private"
            resolved = f"lib/{soname}"
        elif is_system_soname(soname):
            dependency_type = "system"
            resolved = None
        else:
            problems.append(
                f"{relative}: {soname} is neither an approved smplOS system SONAME "
                "nor an exact lib/<SONAME> private dependency"
            )
            dependency_type = "unresolved"
            resolved = None
        dependencies.append({"soname": soname, "class": dependency_type, "path": resolved})

    private_needed = {item["soname"] for item in dependencies if item["class"] == "private"}
    runpaths = [value for tag, value in tags if tag == "RUNPATH"]
    if private_needed:
        expected = "$ORIGIN" if path.parent == root / "lib" else "$ORIGIN/lib"
        if expected not in ":".join(runpaths).split(":"):
            problems.append(f"{relative}: bundled private libraries require RUNPATH {expected}")

    env = {
        "PATH": "/usr/bin:/bin",
        "LANG": "C",
        "LD_LIBRARY_PATH": str(root / "lib") if (root / "lib").is_dir() else "",
    }
    result = subprocess.run(
        ["ldd", str(path.resolve())], capture_output=True, text=True, env=env,
        timeout=SMOKE_TIMEOUT,
    )
    closure = result.stdout + result.stderr
    if result.returncode:
        problems.append(f"{relative}: ldd failed ({result.returncode}): {closure.strip()}")
    if "not found" in closure:
        problems.append(f"{relative}: unresolved runtime closure:\n{closure.strip()}")
    resolved_paths = parse_ldd_paths(closure)
    for soname in needed:
        resolved_path = resolved_paths.get(soname)
        if soname in private_libraries:
            expected_path = private_libraries[soname].resolve()
            if resolved_path is None or Path(resolved_path).resolve() != expected_path:
                problems.append(
                    f"{relative}: private {soname} did not resolve to bundled lib/{soname}"
                )
        elif resolved_path is not None and not (
            resolved_path.startswith("/usr/lib/") or resolved_path.startswith("/lib/")
        ):
            problems.append(
                f"{relative}: {soname} resolved outside the system baseline: {resolved_path}"
            )

    versions = {}
    for library, version in needs:
        versions.setdefault(library, []).append(version)
    return {
        "path": relative,
        "sha256": digest(path),
        "needed": dependencies,
        "runpath": runpaths,
        "version_needs": {library: sorted(set(items)) for library, items in sorted(versions.items())},
    }, problems


def linkage_manifest(root, elf_paths, private_libraries):
    entries = []
    problems = []
    for path in sorted(elf_paths):
        entry, found = audit_elf(path, root, private_libraries)
        entries.append(entry)
        problems.extend(found)
    if problems:
        raise ValueError("; ".join(problems))
    return {
        "schema": 1,
        "layout": {
            "binaries": "root",
            "private_libraries": "lib/<exact DT_SONAME>",
            "system_baseline": "smplOS package-managed /usr/lib or /lib; no compat directory",
        },
        "system_soname_policy": list(SYSTEM_SONAMES),
        "private": [
            {"soname": soname, "path": path.relative_to(root).as_posix(), "sha256": digest(path)}
            for soname, path in sorted(private_libraries.items())
        ],
        "elf": entries,
    }


def audit_release(directory, write_manifest=False):
    """Audit every shipped ELF and emit/check an exact, relocatable receipt."""
    required_binaries(directory)
    private_libraries = private_library_index(directory)
    elf_paths = [path for path in directory.rglob("*") if path.is_file() and is_elf(path)]
    if not elf_paths:
        raise ValueError("Release contains no ELF files")
    manifest = linkage_manifest(directory, elf_paths, private_libraries)
    encoded = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    receipt = directory / LINKAGE_MANIFEST
    if write_manifest:
        receipt.write_bytes(encoded)
        return manifest
    if receipt.is_symlink() or not receipt.is_file():
        raise ValueError(f"Missing linkage manifest: {LINKAGE_MANIFEST}")
    if receipt.read_bytes() != encoded:
        raise ValueError(f"Linkage manifest is stale or does not match the shipped ELF files: {LINKAGE_MANIFEST}")
    return manifest


def runtime_problems(name, needed, paths, needs):
    """Why a binary would not run on the control-surface baseline, plus its
    highest Qt/glibc/libstdc++ needs (for the release record)."""
    problems = []
    if paths:
        problems.append(f"{name}: has an RPATH/RUNPATH ({', '.join(paths)}); it must use the system libraries")
    summary = {}
    qt = [version_tuple(m[1]) for library, version in needs
          if library.startswith("libQt6") and (m := re.fullmatch(r"Qt_(6\.\d+)", version))]
    if any(library.startswith("libQt6") for library in needed):
        if not qt:
            problems.append(f"{name}: links Qt 6 without a Qt_6.N version tag; Qt version tagging must stay enabled")
        if any(version == "Qt_6_PRIVATE_API" for _, version in needs):
            problems.append(f"{name}: uses Qt private API, which ties it to one exact Qt build")
    if qt:
        summary["qt"] = max(qt)
        if max(qt) > CONTROL_SURFACE_MAX_QT:
            problems.append(f"{name}: needs Qt {'.'.join(map(str, max(qt)))}, newer than the supported "
                            f"Qt {'.'.join(map(str, CONTROL_SURFACE_MAX_QT))} (rebuild it on the baseline)")
    glibc = [version_tuple(m[1]) for _, version in needs if (m := re.fullmatch(r"GLIBC_(\d+\.\d+(?:\.\d+)?)", version))]
    if glibc:
        summary["glibc"] = max(glibc)
        if max(glibc)[:2] > CONTROL_SURFACE_MAX_GLIBC:
            problems.append(f"{name}: needs glibc {'.'.join(map(str, max(glibc)))}, newer than the supported "
                            f"glibc {'.'.join(map(str, CONTROL_SURFACE_MAX_GLIBC))}")
    glibcxx = [version_tuple(m[1]) for _, version in needs if (m := re.fullmatch(r"GLIBCXX_(\d+(?:\.\d+)+)", version))]
    if glibcxx:
        summary["glibcxx"] = max(glibcxx)
    return problems, summary


def elf_runtime(path, name):
    def readelf(*options):
        return subprocess.run(["readelf", *options, "-W", str(path)], check=True,
                              capture_output=True, text=True).stdout
    needed, paths = parse_dynamic(readelf("-d"))
    return runtime_problems(name, needed, paths, parse_version_needs(readelf("-V")))


def extract_control_surface(tarball, destination):
    with tarfile.open(tarball, "r:gz") as archive:
        archive.extractall(destination, filter="data")


def check_control_surface(tarball):
    """The asset's layout and its binaries' runtime contract (Qt and glibc no
    newer than the baseline, Qt version tags kept, no RPATH)."""
    verify_control_surface(tarball)
    summaries = {}
    with tempfile.TemporaryDirectory(prefix="control-surface-check-") as scratch:
        extract_control_surface(tarball, scratch)
        problems = []
        for name in CONTROL_SURFACE_BINARIES:
            found, summaries[name] = elf_runtime(Path(scratch) / "usr/bin" / name, name)
            problems += found
    if problems:
        raise ValueError("; ".join(problems))
    return summaries


def describe(summaries):
    for name, summary in summaries.items():
        needs = ", ".join(f"{key} {'.'.join(map(str, value))}" for key, value in sorted(summary.items()))
        print(f"{name}: needs {needs}")


def smoke_control_surface(tarball):
    """Run the packaged binaries on this (baseline) system, without a device,
    a D-Bus session, a config or the network: `control-surfaced features
    --json`, `mouse-remapperd features --json` and `ch552-padprog --help`.
    Returns the Qt library they loaded."""
    if os.geteuid() == 0:
        raise ValueError("run the control-surface smoke test as a regular user, as smplOS does")
    summaries = check_control_surface(tarball)
    with tempfile.TemporaryDirectory(prefix="control-surface-smoke-") as scratch:
        root = Path(scratch) / "root"
        home = Path(scratch) / "home"
        home.mkdir()
        extract_control_surface(tarball, root)
        env = {"PATH": "/usr/bin:/bin", "HOME": str(home), "LANG": "C.UTF-8",
               "XDG_CONFIG_HOME": str(home / ".config"), "XDG_RUNTIME_DIR": str(home / "run"),
               "DBUS_SESSION_BUS_ADDRESS": "disabled:"}
        (home / "run").mkdir(mode=0o700)
        daemon = root / "usr/bin/control-surfaced"
        libraries = subprocess.run(["ldd", str(daemon)], check=True, capture_output=True,
                                   text=True, env=env, timeout=SMOKE_TIMEOUT).stdout
        if "not found" in libraries:
            raise ValueError(f"control-surfaced has unresolved libraries:\n{libraries}")
        qt = re.search(r"libQt6Core\.so\.6 => (\S+)", libraries)
        if not qt or not qt[1].startswith(("/usr/lib/", "/lib/")):
            raise ValueError("control-surfaced does not load the system libQt6Core")
        run = subprocess.run([str(daemon), "features", "--json"], capture_output=True,
                             text=True, env=env, timeout=SMOKE_TIMEOUT)
        if run.returncode:
            raise ValueError(f"control-surfaced features --json failed ({run.returncode}): {run.stderr.strip()}")
        features = json.loads(run.stdout)
        for key in ("apiVersion", "bindingKinds", "kdenlive", "device"):
            if key not in features:
                raise ValueError(f"control-surfaced features --json lacks {key}")
        run = subprocess.run([str(root / "usr/bin/mouse-remapperd"), "features", "--json"],
                             capture_output=True, text=True, env=env, timeout=SMOKE_TIMEOUT)
        if run.returncode:
            raise ValueError(f"mouse-remapperd features --json failed ({run.returncode}): {run.stderr.strip()}")
        mouse = json.loads(run.stdout)
        expected = {"apiVersion": 1, "vendor": "1532", "product": "00b4", "learning": True,
                    "identify": True, "passthrough": True, "appProfiles": "hyprland", "kdenliveApi": True,
                    "identifyWheels": True, "guidedCalibration": True, "guidedSetup": True}
        if any(mouse.get(key) != value for key, value in expected.items()):
            raise ValueError("mouse-remapperd features --json does not satisfy the mouse runtime contract")
        run = subprocess.run([str(root / "usr/bin/ch552-padprog"), "--help"], capture_output=True,
                             text=True, env=env, timeout=SMOKE_TIMEOUT)
        if run.returncode or "list | plan | flash | blank" not in run.stdout:
            raise ValueError(f"ch552-padprog --help failed ({run.returncode}): {run.stderr.strip()}")
    describe(summaries)
    runtime = Path(qt[1]).resolve().name
    print(f"control-surfaced ran with {runtime}")
    return runtime


def verify_uploaded(directory, bundle, metadata_path, extra=()):
    verify_bundle(directory, bundle)
    files = required_files(directory)
    files[bundle.name] = bundle
    for path in extra:
        files[path.name] = path
    metadata = json.loads(metadata_path.read_text())
    version = re.fullmatch(r"smpl-apps-(\d+\.\d+\.\d+)-x86_64\.tar\.gz", bundle.name)
    if not version or metadata.get("tag_name") != f"v{version[1]}":
        raise ValueError("Release tag does not match bundle version")
    if metadata.get("draft") is not True:
        raise ValueError("Uploads must be verified before publishing the draft")
    assets = {}
    for asset in metadata["assets"]:
        name = asset["name"]
        if name in assets:
            raise ValueError(f"Duplicate uploaded asset: {name}")
        assets[name] = asset
    for name, path in files.items():
        asset = assets.get(name, {})
        if (asset.get("state") != "uploaded"
                or asset.get("size") != path.stat().st_size
                or asset.get("digest") != f"sha256:{digest(path)}"):
            raise ValueError(f"Missing, incomplete, or mismatched uploaded asset: {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    collect_args = commands.add_parser("collect")
    collect_args.add_argument("source", type=Path)
    collect_args.add_argument("destination", type=Path)
    audit_args = commands.add_parser("audit")
    audit_args.add_argument("directory", type=Path)
    for command in ("verify", "uploaded"):
        args = commands.add_parser(command)
        args.add_argument("directory", type=Path)
        args.add_argument("bundle", type=Path)
        if command == "uploaded":
            args.add_argument("metadata", type=Path)
            args.add_argument("--asset", type=Path, action="append", default=[],
                              help="another uploaded asset that must match (repeatable)")
    stage_args = commands.add_parser("control-surface")
    stage_args.add_argument("install_root", type=Path)
    stage_args.add_argument("tarball", type=Path)
    calendar_args = commands.add_parser("calendar-rpm")
    calendar_args.add_argument("source", type=Path)
    calendar_args.add_argument("destination", type=Path)
    calendar_args.add_argument("version")
    for command in ("control-surface-check", "control-surface-smoke"):
        commands.add_parser(command).add_argument("tarball", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "collect":
            collect(args.source, args.destination)
        elif args.command == "audit":
            audit_release(args.directory, write_manifest=True)
        elif args.command == "verify":
            verify_bundle(args.directory, args.bundle)
        elif args.command == "control-surface":
            stage_control_surface(args.install_root, args.tarball)
        elif args.command == "calendar-rpm":
            build_calendar_rpm(args.source, args.destination, args.version)
        elif args.command == "control-surface-check":
            describe(check_control_surface(args.tarball))
        elif args.command == "control-surface-smoke":
            smoke_control_surface(args.tarball)
        else:
            verify_uploaded(args.directory, args.bundle, args.metadata, args.asset)
    except (OSError, ValueError, KeyError, tarfile.TarError, subprocess.SubprocessError) as error:
        print(f"Release asset validation failed: {error}", file=sys.stderr)
        return 1
    print(f"Release assets: {args.command} passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
