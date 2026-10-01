#!/usr/bin/env python3
"""Collect required apps and verify local and uploaded release payloads."""

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


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def required_files(directory):
    files = {name: directory / name for name in BINARIES}
    for name, path in files.items():
        if (path.is_symlink() or not path.is_file() or not path.stat().st_size
                or not path.stat().st_mode & 0o111):
            raise ValueError(f"Missing or invalid mandatory binary: {name}")
    return files


def collect(source, destination):
    files = required_files(source)
    destination.mkdir()  # A stale dist directory must not hide missing outputs.
    for name, path in files.items():
        target = destination / name
        shutil.copy2(path, target)
        subprocess.run(["strip", str(target)], check=True)
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
            if member is None or not member.isfile() or not member.mode & 0o111:
                raise ValueError(f"Missing or invalid bundled binary: {name}")
            with archive.extractfile(member) as stream:
                archived = hashlib.file_digest(stream, "sha256").hexdigest()
            if archived != digest(path):
                raise ValueError(f"Bundle differs from standalone asset: {name}")


def verify_uploaded(directory, bundle, metadata_path):
    verify_bundle(directory, bundle)
    files = required_files(directory)
    files[bundle.name] = bundle
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
    args = parser.parse_args()
    try:
        if args.command == "collect":
            collect(args.source, args.destination)
        elif args.command == "verify":
            verify_bundle(args.directory, args.bundle)
        else:
            verify_uploaded(args.directory, args.bundle, args.metadata)
    except (OSError, ValueError, KeyError, tarfile.TarError, subprocess.CalledProcessError) as error:
        print(f"Release asset validation failed: {error}", file=sys.stderr)
        return 1
    print(f"Release assets: {args.command} passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
