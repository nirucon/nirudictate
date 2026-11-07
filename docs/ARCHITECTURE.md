# Architecture

One binary, `nirudictate`, with three roles: the background **service**
(`nirudictate daemon`, started by the session), the **control panel**
(`nirudictate settings`, eframe/egui) and the **CLI** (everything else). The
CLI and the panel talk to the service over a Unix socket.

```
            ┌─────────────── nirudictate daemon ────────────────┐
 X server ──┤ main thread: poll(X fd, wake pipe, signal pipe)   │
 (grab,     │   shortcut → Controller (hybrid/toggle/hold)       │
  overlay)  │   Phase: Idle → Recording → Transcribing →         │
            │          Inserting → Idle                          │
            │   overlay drawn in software (only while visible)   │
            │                                                    │
            │ audio thread  ← pw-record | parec | arecord child  │  (only while recording)
            │ engine thread ← whisper.cpp, nice 10, SCHED_BATCH  │  (model loaded on demand)
            │ insert thread ← own X connection: clipboard, XTest │
            │ agent thread  ← opencode server + TUI (agent mode) │  (only after the first agent message)
            │ control thread← $XDG_RUNTIME_DIR/nirudictate/control.sock
            └────────────────────────────────────────────────────┘
```

## Idle cost

The main thread blocks in `poll(-1)`; the control thread blocks in
`accept`; the engine thread blocks in `recv` (or `recv_timeout` until the
model is unloaded); the insert thread blocks in `poll(-1)`. Nothing wakes
until a key, a request or a signal arrives. The process holds no model and
no audio device while idle.

## Shortcut

`XGrabKey` on the root window for the configured key with the Lock/NumLock
variants. XKB *detectable auto-repeat* is enabled for the connection, so a
held key produces one press and one release. `hotkey::Controller` turns the
events into Start/Stop:

| mode   | press (idle) | release                         | press (recording) |
|--------|--------------|---------------------------------|-------------------|
| hybrid | start        | stop if held ≥ `hold_ms`        | stop              |
| toggle | start        | —                               | stop              |
| hold   | start        | stop                            | —                 |

A failed grab (another client owns the combination) is reported in
`status`, the panel and `doctor`.

## Recording

`audio::Capture` starts the recorder child with 16 kHz mono s16 output on a
pipe (pw-record writes a Sun AU header to pipes; it is detected and skipped).
The audio thread keeps the samples, a 64-value level history for the meters
and a voice-activity detector (20 ms frames, adaptive noise floor). The main
loop ticks at 30 Hz only while recording or transcribing: it redraws the
overlay and stops on silence (`silence_seconds` after speech), on the length
limit, or cancels when nothing was said.

While the user speaks, the engine is asked to preload the model, so the load
time is hidden behind the recording.

## Transcription

Leading/trailing silence is trimmed (with a 300 ms margin); clips without
speech are dropped before whisper sees them, which avoids its subtitle-credit
hallucinations. whisper.cpp runs greedy decoding, no timestamps, the language
fixed, the vocabulary as the initial prompt. Segments with a high no-speech
probability are dropped and `text::finish` removes annotations like
`[MUSIK]` and known credit phrases, then capitalises and adds the trailing
space.

## Insertion

On its own thread and X connection:

1. Read the focused window (`_NET_ACTIVE_WINDOW`) and its `WM_CLASS`.
2. Wait until Ctrl/Alt/Shift/Super are released (push-to-talk ends with them
   held); after 1.5 s release them with XTest.
3. Save the current CLIPBOARD (or PRIMARY for xterm/urxvt): every target it
   lists, INCR transfers included.
4. Own the selection with the text, send Ctrl+V / Ctrl+Shift+V /
   Shift+Insert with XTest, and wait until the focused client (matched by its
   X resource-id base) has fetched it.
5. Offer the saved content again (or clear the selection when it was empty).

If nothing fetched the text, it is typed instead: each character's keysym is
looked up in the keyboard map (with Shift for level 2); characters the
layout lacks are typed through a spare keycode remapped for that character.

## Agent mode

A second grab (`[agent] key`, default Shift+Alt+Space) sets the dictation's
target to the agent; the same `Controller` handles it, with one addition: a
second tap within 450 ms of the tap that started the recording is a
`DoubleTap` (keep recording, start a new conversation). After transcription
the phase goes `Confirming` (the overlay draws the box; Escape, Return and
KP_Enter are grabbed on the root only while it is shown) and then `Sending`.

`agent.rs` owns everything opencode on its own thread, so the main loop never
waits for HTTP or a window:

1. `opencode --version` decides the API: 1.x or 2.x.
2. `opencode serve --hostname 127.0.0.1 --port P` in the agent folder, in its
   own process group with `PR_SET_PDEATHSIG`. A 24-byte random password goes
   to it (and to the TUI) only as `OPENCODE_SERVER_PASSWORD` /
   `OPENCODE_PASSWORD`; stdout and stderr go to /dev/null. Health is polled
   with short timeouts (opencode accepts connections before it answers them).
3. The session id in `agent.json` is checked (`GET`) and reused; otherwise,
   or after a double tap / `agent new`, a session is created.
4. The text goes in with `POST /session/ID/prompt_async` (1.x) or
   `POST /api/session/ID/prompt` (2.x), with basic auth.
5. If no TUI runs for that session, a terminal with class
   `nirudictate-agent` runs `opencode attach URL --dir D --session ID` (1.x)
   or `opencode --server URL --session ID D` (2.x). Its window is found by
   class and `_NET_WM_PID`, sent to slot 9 with `_NIRUWM_COMMAND` 18 (window
   d[2] to slot d[1], independent of focus; NIRUWM 0.17.9) and shown with
   command 15. `_NIRUWM_SCRATCHPADS` confirms it; an older NIRUWM leaves a
   normal window, which is activated instead.
6. When the terminal exits, its waiter thread tells the agent thread, which
   stops the server. `quit` stops both.

## State for others

- `$XDG_RUNTIME_DIR/nirudictate/state.json` — `{"version":1,"state":…,"pid":…,
  "pid_start":…,"since":…,"language":…}`, replaced atomically on every change.
- Root window property `_NIRUDICTATE_STATE` (UTF8_STRING `idle`, `recording`,
  `transcribing`), so a bar that listens to root property changes repaints at
  once. It is deleted when the service exits.
- `nirudictate status --json` and `nirudictate state`.

## Engines

`.cargo/config.toml` configures whisper.cpp (GGML) with `GGML_NATIVE=OFF` and
AVX/AVX2/FMA/F16C/BMI2 on. The release builds a second binary with those off
(`NIRUDICTATE_ENGINE=baseline`). At start-up the main binary checks the CPU
and, without AVX2/FMA/F16C/BMI2, execs `nirudictate-baseline` (next to it or
in `~/.local/lib/nirudictate/`).

## Panel

eframe 0.29 with the glow renderer, X11 only. The theme module is NIRUAMP's:
NIRUX theme files and `appearance.toml`, Omarchy's `colors.toml` outside the
NIRU session, watched with inotify for live switching. Every change is saved
immediately (`config.toml`, atomic) and the service reloads it.
