#!/usr/bin/env bash
# uninstall.sh removes the cheat-sheet registration and leaves other apps' files.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
export HOME="$T/home" XDG_DATA_HOME="$T/home/.local/share" XDG_CONFIG_HOME="$T/home/.config" XDG_STATE_HOME="$T/home/.local/state"
kd="$XDG_DATA_HOME/niru/keys.d"
mkdir -p "$kd"
printf 'app = "NIRUDICTATE"\n' > "$kd/nirudictate.toml"
printf 'app = "OTHER"\n' > "$kd/other.toml"
bash "$here/uninstall.sh" >/dev/null
[ ! -e "$kd/nirudictate.toml" ] || { echo "FAIL: keys.d/nirudictate.toml left behind"; exit 1; }
[ -e "$kd/other.toml" ] || { echo "FAIL: another app's keys.d file was removed"; exit 1; }
echo "PASS uninstall removes keys.d/nirudictate.toml"
