#!/usr/bin/env bash
# Remove NIRUDICTATE. Models, settings and history stay unless --purge is given.
set -euo pipefail
BIN="${HOME}/.local/bin/nirudictate"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
STATE="${XDG_STATE_HOME:-$HOME/.local/state}"
[ -x "$BIN" ] && "$BIN" quit >/dev/null 2>&1 || :
rm -f "$BIN" "$HOME/.local/lib/nirudictate/nirudictate-baseline" \
  "$DATA/applications/nirudictate.desktop" \
  "$DATA/icons/hicolor/scalable/apps/nirudictate.svg" \
  "$CONFIG/autostart/nirudictate.desktop" \
  "$DATA/niru/keys.d/nirudictate.toml"
rmdir "$HOME/.local/lib/nirudictate" 2>/dev/null || :
if [ "${1:-}" = "--purge" ]; then
  rm -rf "$CONFIG/nirudictate" "$STATE/nirudictate" "$DATA/nirudictate"
  printf "Removed NIRUDICTATE, its settings, history and downloaded models.\n"
else
  printf "Removed NIRUDICTATE. Settings, history and models were kept:\n"
  printf "  %s\n  %s\n  %s\n" "$CONFIG/nirudictate" "$STATE/nirudictate" "$DATA/nirudictate/models"
  printf "Run ./uninstall.sh --purge to remove them too.\n"
fi
