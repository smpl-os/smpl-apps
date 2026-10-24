#!/usr/bin/env bash
# Run as the desktop user; only the DNF transaction requests administrator access.
set -euo pipefail
umask 077

if [[ "${1:-}" == --help && $# == 1 ]]; then
    printf '%s\n' \
        'Usage: bash install-calendar-rpm.sh [--kde-shortcut | --no-kde-shortcut]' \
        'Install/update the latest smpl Calendar RPM for Nobara 43 KDE (x86_64).' \
        'Existing events and preferences are backed up before DNF is invoked.' \
        'On KDE, Meta+Alt+C is assigned to the calendar; conflicting bindings are backed up and reassigned.' \
        '--no-kde-shortcut leaves all KDE bindings unchanged.' \
        'Run as your desktop user, not as root.'
    exit 0
fi
shortcut=auto
case "$#:${1:-}" in
    0:) ;;
    1:--kde-shortcut) shortcut=yes ;;
    1:--no-kde-shortcut) shortcut=no ;;
    *) printf 'Unknown arguments; use --help.\n' >&2; exit 1 ;;
esac
for command in curl python3 pkexec dnf rpm uname id mktemp; do
    command -v "$command" >/dev/null || {
        printf 'Required command is missing: %s\n' "$command" >&2
        exit 1
    }
done
if [[ "$(id -u)" == 0 ]]; then
    printf 'Run as your desktop user, not root. DNF will request graphical authentication.\n' >&2
    exit 1
fi
if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
    printf 'This package requires x86_64 Linux (Nobara 43 KDE).\n' >&2
    exit 1
fi

workdir="$(mktemp -d -t smpl-calendar-install.XXXXXXXX)"
cleanup() {
    rm -f -- "$workdir/release.json" "$workdir/asset.tsv" "$workdir/calendar.rpm" \
        "$workdir/backup-calendar-data.py" "$workdir/backup.json"
    rmdir -- "$workdir"
}
trap cleanup EXIT
download() {
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --fail --silent --show-error --location --retry 3 \
        --connect-timeout 15 --max-time 600 "$@"
}

printf 'Finding the latest smpl Calendar RPM...\n'
download https://api.github.com/repos/smpl-os/smpl-apps/releases/latest \
    -o "$workdir/release.json"
python3 - "$workdir/release.json" > "$workdir/asset.tsv" <<'PY'
import json
import re
import sys
from pathlib import Path

release = json.loads(Path(sys.argv[1]).read_text())
tag = release.get("tag_name", "")
if not re.fullmatch(r"v\d+\.\d+\.\d+", tag) or release.get("draft") or release.get("prerelease"):
    sys.exit("Latest release has an unsupported version or publication state.")
version = tag[1:]
name = f"smpl-calendar-{version}-1.fc43.x86_64.rpm"
assets = [asset for asset in release["assets"] if asset["name"] == name]
if len(assets) != 1:
    sys.exit("The latest release does not contain the Nobara 43 calendar RPM. Nothing was installed.")
asset = assets[0]
url = f"https://github.com/smpl-os/smpl-apps/releases/download/{tag}/{name}"
digest = asset.get("digest", "")
size = asset.get("size")
if (asset.get("browser_download_url") != url or asset.get("state") != "uploaded"
        or not isinstance(digest, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", digest)
        or not isinstance(size, int) or not 0 < size <= 512 * 1024 * 1024):
    sys.exit("The release RPM has invalid download/checksum metadata. Nothing was installed.")
print(version, url, digest[7:], size, sep="\t")
PY
IFS=$'\t' read -r version url digest size < "$workdir/asset.tsv"
download --max-filesize "$size" "$url" -o "$workdir/calendar.rpm"
python3 - "$workdir/calendar.rpm" "$digest" "$size" <<'PY'
import hashlib
import sys
from pathlib import Path

path = Path(sys.argv[1])
with path.open("rb") as stream:
    digest = hashlib.file_digest(stream, "sha256").hexdigest()
if path.stat().st_size != int(sys.argv[3]) or digest != sys.argv[2]:
    sys.exit("RPM size/checksum verification failed. Nothing was installed.")
PY
details="$(rpm -qp --qf '%{NAME}\t%{VERSION}\t%{RELEASE}\t%{ARCH}\n' "$workdir/calendar.rpm")"
if [[ "$details" != $'smpl-calendar\t'"$version"$'\t1.fc43\tx86_64' ]]; then
    printf 'Unexpected RPM identity: %s. Nothing was installed.\n' "$details" >&2
    exit 1
fi

database="${HOME:?HOME must be set}/.local/share/smplos/calendar/events.db"
if [[ -e "$database" || -L "$database" ]]; then
    printf 'Backing up existing events and reminder history...\n'
    download "https://raw.githubusercontent.com/smpl-os/smpl-apps/v${version}/scripts/backup-calendar-data.py" \
        -o "$workdir/backup-calendar-data.py"
    python3 "$workdir/backup-calendar-data.py" > "$workdir/backup.json"
fi
python3 - "$workdir/backup.json" <<'PY'
import hashlib
import json
import os
import sys
import tempfile
from pathlib import Path

home = Path.home()
config = Path(os.environ.get("XDG_CONFIG_HOME") or home / ".config")
state = Path(os.environ.get("XDG_STATE_HOME") or home / ".local/state")
if not config.is_absolute() or not state.is_absolute():
    sys.exit("XDG config/state directories must be absolute. Nothing was installed.")
manifest = Path(sys.argv[1])
directory = None
if manifest.exists():
    info = json.loads(manifest.read_text())
    database = Path(info["database"])
    with database.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    if info["integrity_check"] != "ok" or digest != info["sha256"]:
        sys.exit("Database backup verification failed. Nothing was installed.")
    directory = database.parent
preferences = config / "smplos/calendar.json"
if preferences.exists() or preferences.is_symlink():
    if not preferences.is_file() or preferences.stat().st_uid != os.getuid():
        sys.exit("Calendar preferences must be an owned regular file. Nothing was installed.")
    if directory is None:
        base = state / "smplos/calendar/backups"
        base.mkdir(parents=True, exist_ok=True, mode=0o700)
        directory = Path(tempfile.mkdtemp(prefix="preferences.", dir=base))
    data = preferences.read_bytes()
    target = directory / "calendar.json"
    with target.open("xb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    if target.read_bytes() != data:
        sys.exit("Preferences backup verification failed. Nothing was installed.")
if directory is not None:
    print(f"Verified calendar backup: {directory}")
PY

printf 'Installing/updating smpl Calendar %s through DNF...\n' "$version"
pkexec dnf install --assumeyes "$workdir/calendar.rpm"
if [[ "$shortcut" == auto ]]; then
    desktop="${XDG_CURRENT_DESKTOP:-}"
    case ":${desktop^^}:" in
        *:KDE:*) shortcut=yes ;;
        *) shortcut=no ;;
    esac
fi
if [[ "$shortcut" == yes ]]; then
    printf 'Assigning KDE Meta+Alt+C to smpl Calendar; conflicting bindings will be backed up and reassigned.\n'
    if ! smpl-calendar-kde-shortcut; then
        printf 'Calendar is installed, but KDE shortcut setup failed. No shortcut success is being reported.\n' >&2
        exit 1
    fi
else
    printf 'KDE shortcut setup skipped. Use --kde-shortcut to request it explicitly.\n'
fi
printf '%s\n' \
    'Calendar installation/update completed. Existing calendar data was not reset.' \
    'Open smpl Calendar from the application menu. Reminders start at login or app launch.' \
    'Close and reopen an already-running calendar to use the updated application.'
