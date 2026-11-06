#!/usr/bin/env bash
# NIRUDICTATE installer: the prebuilt x86_64 binary (or a source build), the
# desktop entry, autostart, and a running service. No root, no system files.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
BIN="${HOME}/.local/bin"
LIB="${HOME}/.local/lib/nirudictate"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
APPS="$DATA/applications"
ICONS="$DATA/icons/hicolor/scalable/apps"
AUTOSTART="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/nirudictate"
B=$'\033[1m'; G=$'\033[32m'; Y=$'\033[33m'; R=$'\033[31m'; N=$'\033[0m'

MODE="${NIRUDICTATE_INSTALL_MODE:-auto}"
START=1
for arg in "$@"; do
  case "$arg" in
    --prebuilt) MODE=prebuilt ;;
    --source) MODE=source ;;
    --no-start) START=0 ;;
    -h|--help)
      printf "Usage: ./install.sh [--prebuilt|--source] [--no-start]\n"
      printf "  --prebuilt  Download the published x86_64 Linux binaries (default when available)\n"
      printf "  --source    Build with cargo (Rust 1.88+, cmake, a C++ compiler)\n"
      printf "  --no-start  Do not start the background service now\n"
      exit 0 ;;
    *) printf "%sERROR%s unknown option: %s\n" "$R" "$N" "$arg"; exit 2 ;;
  esac
done

printf "%sNIRUDICTATE%s — local speech-to-text dictation\n" "$B" "$N"

version_ge() { # version_ge HAVE WANT (major.minor)
  python3 - "$1" "$2" <<'PY' 2>/dev/null
import sys
def v(s):
    p = (s.split('.') + ['0', '0'])[:2]
    return tuple(int(''.join(c for c in x if c.isdigit()) or 0) for x in p)
sys.exit(0 if v(sys.argv[1]) >= v(sys.argv[2]) else 1)
PY
}

glibc_ok() {
  command -v getconf >/dev/null || return 1
  version_ge "$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')" 2.35
}

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
ASSET=""
case "$(uname -m)" in
  x86_64|amd64) ASSET="nirudictate-${VERSION}-x86_64-linux-gnu.tar.gz" ;;
esac
BASE="https://github.com/nirucon/nirudictate/releases/download/v${VERSION}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
BINARY=""
BASELINE=""

download_prebuilt() {
  [ -n "$ASSET" ] || return 1
  glibc_ok || return 1
  command -v curl >/dev/null || return 1
  printf "Downloading prebuilt %s…\n" "$ASSET"
  curl -fL --retry 2 --connect-timeout 15 -o "$TMP/$ASSET" "$BASE/$ASSET" >/dev/null 2>&1 || return 1
  if curl -fL --retry 2 --connect-timeout 15 -o "$TMP/SHA256SUMS" "$BASE/SHA256SUMS-$VERSION" >/dev/null 2>&1; then
    want=$(awk -v a="$ASSET" '{n=$2; sub(/^.*\//, "", n); if (n == a) print $1}' "$TMP/SHA256SUMS")
    have=$(sha256sum "$TMP/$ASSET" | awk '{print $1}')
    if [ -z "$want" ] || [ "$want" != "$have" ]; then
      printf "%sERROR%s checksum mismatch for %s\n" "$R" "$N" "$ASSET"; return 1
    fi
  fi
  mkdir -p "$TMP/pkg"
  tar -xzf "$TMP/$ASSET" -C "$TMP/pkg" || return 1
  [ -f "$TMP/pkg/nirudictate" ] || return 1
  BINARY="$TMP/pkg/nirudictate"
  [ -f "$TMP/pkg/nirudictate-baseline" ] && BASELINE="$TMP/pkg/nirudictate-baseline"
  return 0
}

build_from_source() {
  if ! command -v cargo >/dev/null || ! version_ge "$(cargo --version | awk '{print $2}')" 1.88; then
    printf "%sERROR%s No prebuilt binary is available here, and a source build needs Rust 1.88+.\n" "$R" "$N"
    printf "Install Rust with https://rustup.rs (Debian's packaged Rust is too old), then run again.\n"
    exit 1
  fi
  command -v cmake >/dev/null || { printf "%sERROR%s cmake is required to build whisper.cpp\n" "$R" "$N"; exit 1; }
  printf "Building from source (whisper.cpp takes a few minutes)…\n"
  cd "$ROOT"
  cargo build --release --locked
  BINARY="$ROOT/target/release/nirudictate"
  if ! grep -qw avx2 /proc/cpuinfo 2>/dev/null; then
    printf "This CPU has no AVX2: building the baseline engine as well…\n"
    GGML_AVX=OFF GGML_AVX2=OFF GGML_FMA=OFF GGML_F16C=OFF GGML_BMI2=OFF NIRUDICTATE_ENGINE=baseline \
      cargo build --release --locked --target-dir "$ROOT/target-baseline"
    BASELINE="$ROOT/target-baseline/release/nirudictate"
  fi
}

case "$MODE" in
  prebuilt)
    download_prebuilt || {
      printf "%sERROR%s No compatible prebuilt binary could be downloaded.\n" "$R" "$N"
      printf "Try ./install.sh --source, or check the network and glibc (2.35+).\n"
      exit 1
    } ;;
  source) build_from_source ;;
  *)
    if ! download_prebuilt; then
      printf "Prebuilt binary unavailable; building from source.\n"
      build_from_source
    fi ;;
esac

mkdir -p "$BIN" "$LIB" "$APPS" "$ICONS" "$AUTOSTART" "$STATE"
# Atomic replacement: a running service keeps its old inode.
install -m 0755 "$BINARY" "$BIN/.nirudictate.new.$$"
mv -f "$BIN/.nirudictate.new.$$" "$BIN/nirudictate"
if [ -n "$BASELINE" ]; then
  install -m 0755 "$BASELINE" "$LIB/.baseline.new.$$"
  mv -f "$LIB/.baseline.new.$$" "$LIB/nirudictate-baseline"
fi
install -m 0644 "$ROOT/assets/nirudictate.svg" "$ICONS/nirudictate.svg"
sed "s|^Exec=nirudictate |Exec=$BIN/nirudictate |" "$ROOT/assets/nirudictate.desktop" > "$APPS/nirudictate.desktop"
chmod 0644 "$APPS/nirudictate.desktop"
# Other desktops (GNOME, KDE, Xfce, …) start the service from XDG autostart.
# NIRUX starts it from its session; `nirudictate autostart` honours the
# setting in the panel either way.
sed "s|^Exec=nirudictate |Exec=$BIN/nirudictate |" "$ROOT/assets/nirudictate-autostart.desktop" > "$AUTOSTART/nirudictate.desktop"
chmod 0644 "$AUTOSTART/nirudictate.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" >/dev/null 2>&1 || :
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$DATA/icons/hicolor" >/dev/null 2>&1 || :
printf "%s✓%s Installed %s (%s)\n" "$G" "$N" "$BIN/nirudictate" "$("$BIN/nirudictate" --version)"
[ -n "$BASELINE" ] && printf "%s✓%s Baseline engine for CPUs without AVX2: %s\n" "$G" "$N" "$LIB/nirudictate-baseline"
printf "%s✓%s Desktop entry %s\n" "$G" "$N" "$APPS/nirudictate.desktop"

if ! command -v pw-record >/dev/null && ! command -v parec >/dev/null && ! command -v arecord >/dev/null; then
  printf "%sWARNING%s No audio recorder found. Install one of:\n" "$Y" "$N"
  printf "  Debian/Ubuntu: sudo apt install pipewire-bin   Arch: sudo pacman -S pipewire   Void: sudo xbps-install pipewire\n"
fi

# Start (or restart) the service in the running X session, also when this
# runs over SSH or from `nirux extras install`: the session's display and
# runtime environment are taken from this user's window manager or X server.
session_env() {
  [ -n "${DISPLAY:-}" ] && return 0
  for name in niruwm Xorg X; do
    for pid in $(pgrep -u "$(id -u)" -x "$name" 2>/dev/null); do
      [ -r "/proc/$pid/environ" ] || continue
      d=$(tr '\0' '\n' < "/proc/$pid/environ" | sed -n 's/^DISPLAY=//p' | head -n1)
      [ -n "$d" ] || continue
      export DISPLAY="$d"
      for v in XAUTHORITY XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS XDG_CURRENT_DESKTOP DESKTOP_SESSION; do
        val=$(tr '\0' '\n' < "/proc/$pid/environ" | sed -n "s/^$v=//p" | head -n1)
        [ -n "$val" ] && export "$v=$val"
      done
      return 0
    done
  done
  return 1
}
if [ "$START" = 1 ]; then
  "$BIN/nirudictate" quit >/dev/null 2>&1 || :
  if session_env; then
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      "$BIN/nirudictate" status >/dev/null 2>&1 || break
      sleep 0.2
    done
    setsid "$BIN/nirudictate" autostart </dev/null >>"$STATE/daemon.log" 2>&1 &
    sleep 0.6
    if "$BIN/nirudictate" status >/dev/null 2>&1; then
      printf "%s✓%s Service running on display %s\n" "$G" "$N" "$DISPLAY"
    else
      printf "%sNOTE%s The service starts with your next desktop session (or run: nirudictate daemon).\n" "$Y" "$N"
    fi
  else
    printf "%sNOTE%s No running X session found; the service starts with your next desktop session.\n" "$Y" "$N"
  fi
fi

printf "\nOpen the panel: nirudictate settings (or NIRUDICTATE in the launcher)\n"
printf "First run: download a model there, then press Ctrl+Alt+Space and speak.\n"
