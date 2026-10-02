#!/usr/bin/env bash
# Install only the calendar; preserve events, preferences, daemons and other apps.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "${1:-}" == "--with-reminders" && $# == 1 ]]; then
    exec python3 "$root/scripts/deploy-calendar-reminders-local.py"
fi
[[ $# == 0 ]] || { echo "Usage: $0 [--with-reminders]" >&2; exit 1; }
source_binary="$root/target/release/smpl-calendar"
[[ -f "$source_binary" && -x "$source_binary" ]] || {
    echo "Build first: cargo build --release -p smpl-calendar --bin smpl-calendar" >&2
    exit 1
}
"$source_binary" --version

exec pkexec /usr/bin/bash -c '
set -euo pipefail
exec 9>/var/lock/smpl-calendar-install.lock
flock -n 9 || { echo "Another calendar installation is running." >&2; exit 1; }
source_binary="$1"
destination=/usr/local/bin/smpl-calendar
[[ -f "$destination" && ! -L "$destination" ]] || {
    echo "Expected an existing regular binary at $destination; refusing an unverified replacement." >&2
    exit 1
}
install -d -m 755 /var/backups/smpl-calendar
backup="$(mktemp -d /var/backups/smpl-calendar/$(date -u +%Y%m%dT%H%M%SZ).XXXXXX)"
cp -a -- "$destination" "$backup/smpl-calendar"
cmp -- "$destination" "$backup/smpl-calendar"
sha256sum "$backup/smpl-calendar" > "$backup/SHA256SUMS"
sha256sum --check "$backup/SHA256SUMS"
stage="$(mktemp /usr/local/bin/.smpl-calendar.XXXXXX)"
trap '\''rm -f -- "$stage"'\'' EXIT
install -o root -g root -m 755 -- "$source_binary" "$stage"
cmp -- "$source_binary" "$stage"
mv -T -- "$stage" "$destination"
cmp -- "$source_binary" "$destination"
echo "Installed: $destination"
echo "Verified backup: $backup/smpl-calendar"
sha256sum "$destination"
echo "Launch: /usr/local/bin/smpl-calendar (or the taskbar calendar button)"
echo "No running processes were terminated; close an old popup before reopening."
' calendar-install "$source_binary"
