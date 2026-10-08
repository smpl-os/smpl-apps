#!/usr/bin/env python3
"""Collect required apps and verify local and uploaded release payloads.

Also stages control-surface (the keypad daemon, built by CMake) as its own
release asset, laid out under usr/ for packages: smplOS installs it with
pacman rather than copying its binary into /usr/local/bin with the apps.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile


BINARIES = (
    "start-menu", "notif-center", "settings", "app-center", "webapp-center",
    "sync-center-daemon", "sync-center-gui", "smpl-calendar",
    "smpl-calendar-alertd", "smpl-hints", "smpl-hintsd",
)
SERVICE = "smpl-calendar-alertd.service"
SERVICE_SOURCE = Path(__file__).resolve().parents[1] / "calendar/systemd" / SERVICE

CONTROL_SURFACE_SOURCE = Path(__file__).resolve().parents[1] / "control-surface"
CONTROL_SURFACE_BINARIES = ("control-surfaced", "ch552-padprog")
FIRMWARE_DIR = "usr/share/control-surface/firmware"
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
    return files


def collect(source, destination):
    files = required_binaries(source)
    validate_service(SERVICE_SOURCE)
    destination.mkdir()  # A stale dist directory must not hide missing outputs.
    for name, path in files.items():
        target = destination / name
        shutil.copy2(path, target)
        subprocess.run(["strip", str(target)], check=True)
    shutil.copyfile(SERVICE_SOURCE, destination / SERVICE)
    (destination / SERVICE).chmod(0o644)
    required_files(destination)


def verify_bundle(directory, bundle):
    files = required_files(directory)
    with tarfile.open(bundle, "r:gz") as archive:
        members = {}
        for member in archive.getmembers():
            name = member.name.removeprefix("./")
            if name in members:
                raise ValueError(f"Duplicate archive member: {name}")
            members[name] = member
        for name, path in files.items():
            member = members.get(name)
            if member is None or not member.isfile():
                raise ValueError(f"Missing or invalid bundled asset: {name}")
            if name == SERVICE:
                if member.mode & 0o777 != 0o644:
                    raise ValueError(f"Bundled service must have mode 0644: {name}")
            elif not member.mode & 0o111:
                raise ValueError(f"Bundled binary must be executable: {name}")
            with archive.extractfile(member) as stream:
                archived = hashlib.file_digest(stream, "sha256").hexdigest()
            if archived != digest(path):
                raise ValueError(f"Bundle differs from standalone asset: {name}")


def control_surface_asset(version):
    return f"control-surface-{version}-x86_64.tar.gz"


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

    def normalize(info):
        info.uid = info.gid = 0
        info.uname = info.gname = "root"
        info.mode = 0o755 if info.name.startswith("usr/bin/") else 0o644
        return info

    with tarfile.open(tarball, "w:gz") as archive:
        for archived, path in sorted(members.items()):
            archive.add(path, arcname=archived, filter=normalize)
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
    args = parser.parse_args()
    try:
        if args.command == "collect":
            collect(args.source, args.destination)
        elif args.command == "verify":
            verify_bundle(args.directory, args.bundle)
        elif args.command == "control-surface":
            stage_control_surface(args.install_root, args.tarball)
        else:
            verify_uploaded(args.directory, args.bundle, args.metadata, args.asset)
    except (OSError, ValueError, KeyError, tarfile.TarError, subprocess.CalledProcessError) as error:
        print(f"Release asset validation failed: {error}", file=sys.stderr)
        return 1
    print(f"Release assets: {args.command} passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
