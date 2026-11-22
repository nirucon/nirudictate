#!/bin/sh
# X11 end-to-end checks of the control panel in a private Xvfb: first frame,
# memory, idle CPU, and the main interactions (tabs, check boxes, radio
# buttons, a slider, recording a shortcut, text fields with å/ä/ö, a drop-down
# list, the clipboard, one window only, the service state following the
# service) plus the idle cost of the service.
# Usage: tests/xvfb.sh PATH/TO/nirudictate   (DISPLAY number: XVFB_DISPLAY, default :67)
set -eu
BIN=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
SB=$(mktemp -d)
PIDS=""
cleanup() { for p in $PIDS; do kill "$p" 2>/dev/null || true; done; rm -rf "$SB"; }
trap cleanup EXIT INT TERM
export HOME="$SB/home" XDG_CONFIG_HOME="$SB/config" XDG_DATA_HOME="$SB/data" \
  XDG_STATE_HOME="$SB/state" XDG_RUNTIME_DIR="$SB/run" XDG_CACHE_HOME="$SB/cache"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
D=${XVFB_DISPLAY:-:67}
Xvfb "$D" -screen 0 1600x900x24 -nolisten tcp >/dev/null 2>&1 &
PIDS="$PIDS $!"
export DISPLAY="$D"
sleep 1
CFG="$XDG_CONFIG_HOME/nirudictate/config.toml"
HITS="$SB/hits"
fail() { echo "FAIL: $*" >&2; cut -c1-160 "$SB/err" >&2; exit 1; }
at() { awk -F'\t' -v l="$1" '$1 == l {print $2, $3; exit}' "$HITS"; }
click() {
  pos=$(at "$1")
  [ -n "$pos" ] || fail "no widget '$1' on screen"
  xdotool mousemove --window "$W" "${pos% *}" "${pos#* }" click 1
  sleep 0.35
}
has() { grep -q "$1" "$CFG" 2>/dev/null; }

NIRUDICTATE_TIMING=1 NIRUDICTATE_HITS="$HITS" "$BIN" settings 2>"$SB/err" &
P=$!
PIDS="$PIDS $P"
sleep 1.5
grep -q 'first frame after' "$SB/err" || fail "no first frame: $(cat "$SB/err")"
ms=$(sed -n 's/.*after \([0-9]*\)\..*/\1/p' "$SB/err" | head -1)
[ "$ms" -lt 1000 ] || fail "first frame took $ms ms"
W=$(xdotool search --pid "$P" | head -1)
[ -n "$W" ] || fail "no window"
xdotool windowfocus "$W" 2>/dev/null || true
sleep 2
rss=$(awk '/VmRSS/ {print $2}' /proc/$P/status)
c0=$(awk '{print $14+$15}' /proc/$P/stat); sleep 5; c1=$(awk '{print $14+$15}' /proc/$P/stat)
echo "panel: first frame ${ms} ms, ${rss} kB, idle $((c1 - c0)) ticks in 5 s"
[ "$rss" -lt 40000 ] || fail "memory $rss kB"
[ $((c1 - c0)) -le 1 ] || fail "the idle panel used CPU"
xprop -id "$W" WM_CLASS | grep -q '"nirudictate"' || fail "WM_CLASS"

# One window only: a second start raises the first and exits.
"$BIN" settings >/dev/null 2>&1 || fail "second start failed"
[ "$(xdotool search --class nirudictate | wc -l)" = 1 ] || fail "a second window opened"

# Output: a check box saves at once.
click "Output"
click "Play the dictation sounds"
has '^sounds = false' || fail "sounds check box"
echo "check box ok"

# Shortcut & Mode: radio button, slider, recording a shortcut.
click "Shortcut & Mode"
click "Toggle (press to start, press again to stop)"
has '^mode = "toggle"' || fail "mode radio"
before=$(grep '^max_seconds' "$CFG" || true)
click "slider:longest"
after=$(grep '^max_seconds' "$CFG")
[ "$before" != "$after" ] || fail "slider did not move ($after)"
click "Change…"
xdotool key ctrl+alt+d
sleep 0.4
has '^key = "ctrl+alt+d"' || fail "shortcut not recorded: $(grep '^key' "$CFG")"
echo "radio, slider, shortcut ok"

# Vocabulary: a multi-line text field (Return, å/ä/ö, BackSpace).
click "Vocabulary"
click "edit:vocabulary"
xdotool key ctrl+End Return
xdotool type --delay 30 "Nordkapp"
xdotool key BackSpace Return
# xdotool types letters missing from the layout through a remapped key
# without Shift, so the test uses lower case å/ä/ö.
xdotool type --delay 30 "smörgåsbord är"
sleep 0.5
grep -q '"Nordkap"' "$CFG" && grep -q '"smörgåsbord är"' "$CFG" || fail "vocabulary: $(grep -A12 '^\[vocabulary' "$CFG")"
# The clipboard: select all and copy.
xdotool key ctrl+a ctrl+c
sleep 0.4
xclip -o -selection clipboard 2>/dev/null | tail -2 | tr '\n' '|' | grep -qx 'Nordkap|smörgåsbord är|\{0,1\}' || fail "copy: $(xclip -o -selection clipboard | tail -3)"
echo "text field and clipboard ok"

# Agent mode: a one-line field.
click "Agent mode"
click "edit:agent-name"
xdotool type --delay 30 "plan"
sleep 0.4
has '^agent = "plan"' || fail "agent name: $(grep -A12 '^\[agent' "$CFG"; grep edit: "$HITS")"
echo "agent field ok"

# Advanced: a drop-down list (theme).
click "Advanced"
click "combo:theme"
click "item:Niru Nord"
has '^theme = "niru-nord"' || fail "theme list: $(grep '^theme' "$CFG")"
echo "drop-down ok"

# History: turn it off.
click "History"
click "Keep a history"
grep -A3 '^\[history' "$CFG" | grep -q 'enabled = false' || fail "history check box"
echo "history ok"

# The service: its state reaches the panel through the root property.
click "Overview"
at "Start service" >/dev/null && [ -n "$(at "Start service")" ] || fail "no Start service button while stopped"
"$BIN" daemon >"$SB/daemon.log" 2>&1 &
S=$!
PIDS="$PIDS $S"
sleep 2.5
[ -z "$(at "Start service")" ] || fail "the panel did not notice the service"
srss=$(awk '/VmRSS/ {print $2}' /proc/$S/status)
c0=$(awk '{print $14+$15}' /proc/$S/stat); p0=$(awk '{print $14+$15}' /proc/$P/stat)
sleep 5
c1=$(awk '{print $14+$15}' /proc/$S/stat); p1=$(awk '{print $14+$15}' /proc/$P/stat)
echo "service: ${srss} kB, idle $((c1 - c0)) ticks in 5 s; panel with service $((p1 - p0)) ticks"
[ $((c1 - c0)) -le 1 ] || fail "the idle service used CPU"
[ $((p1 - p0)) -le 1 ] || fail "the panel polls the idle service"
kill "$S"; sleep 1.5
[ -n "$(at "Start service")" ] || fail "the panel did not notice the service stop"
echo "service state ok"
rss=$(awk '/VmRSS/ {print $2}' /proc/$P/status)
echo "panel after every tab: ${rss} kB"
echo "PASS"
