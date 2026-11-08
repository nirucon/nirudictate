# NIRUDICTATE 0.2.1

- **Cheat sheet registration.** The service writes
  `~/.local/share/niru/keys.d/nirudictate.toml` (`$XDG_DATA_HOME`) from the
  shortcuts it really uses, each time it starts or reloads its settings. NIRUWM
  0.18's cheat sheet (Super+/) lists them under *Dictation* and flags clashes.
  The agent key is listed only while agent mode is on and opencode is
  installed. The file is rewritten only when it changes.
- Keys are written in NIRUWM's `[[keys]]` syntax; a key NIRUWM has no name for
  (Insert, Home, …) is left out instead of making the file invalid.
- `uninstall.sh` (and `nirux extras uninstall nirudictate`) removes the file.
- Tests: key syntax, file content per setting, write mode; a shell test that
  uninstall removes the file and leaves other programs' files.

# NIRUDICTATE 0.2.0

Agent mode: talk to [opencode](https://opencode.ai).

- **Shift+Alt+Space** records like the dictation shortcut (tap to start, it
  stops after a pause or a second tap; hold to talk), transcribes locally,
  and sends the text to opencode instead of pasting it.
- A small themed **confirmation box** shows the text and **sends it by itself
  after 2 s**; **Enter** sends at once, **Esc** cancels (nothing is sent).
- Delivery goes through opencode's **server API**: NIRUDICTATE starts its own
  `opencode serve` on 127.0.0.1 with a random password that travels only in
  the environment. No keystrokes are injected and the message never appears
  in a command line.
- **One ongoing conversation**, kept across restarts
  (`~/.local/state/nirudictate/agent.json`). A **quick double tap** of the
  shortcut, **Start a new conversation** in the panel or
  `nirudictate agent new` begins a new one.
- The opencode TUI runs attached to that server in a terminal (kitty,
  alacritty or xterm) that always lives in **NIRUWM scratchpad 9**
  (Super+Alt+9). It is started when it is missing and shown on top with each
  message. Closing it stops its server. Needs NIRUWM 0.17.9 (`niructl
  scratchpad send N --window ID`); older window managers get a normal window.
- opencode's **default agent** unless `[agent] agent` names one; permissions
  are never auto-approved (no `--auto`). Working folder `~` by default.
- Both **opencode 1.x** (yargs CLI: `serve`, `attach --session`,
  `/session/ID/prompt_async`) and **2.x** (`--server`, `--session`,
  `/api/session/ID/prompt`).
- **Without opencode** the agent shortcut is not grabbed; `status`, the panel
  and `doctor` say so.
- `doctor` warns when the agent shortcut clashes with an XKB layout switch
  such as `grp:alt_shift_toggle`, and checks opencode, the folder and the
  agent shortcut grab.
- New panel tab **Agent mode**; new `[agent]` settings (`enabled`, `key`,
  `directory`, `agent`, `confirm_seconds`, `double_tap_new`, `terminal`,
  `opencode`). 0.1 config files load unchanged.
- New CLI: `nirudictate agent [status|toggle|new|show|send|file FILE.wav]`.
- The overlay shows **AI** (and **NEW AI** for a new conversation) while it
  records for the agent, and "Sending to opencode…".
- Live state: `state.json` gains `"target":"agent"` during an agent
  dictation (still `version: 1`; NIRUBAR 0.18.90 shows `● AI`). `status
  --json` gains an `agent` object.
- Speaking the reply aloud (Piper TTS) is left for a later release.

# NIRUDICTATE 0.1.0

First release: local speech-to-text dictation for the NIRUX desktop (X11).

- Press **Ctrl+Alt+Space** anywhere: a short tap starts recording and it stops
  by itself after a pause (or tap again); hold the keys while speaking and
  release to insert (push-to-talk). Toggle-only and hold-only modes too.
- **Swedish by default** with KBLab's KB-Whisper models; English with
  OpenAI Whisper (whisper.cpp). Everything runs on this computer with
  whisper.cpp; the network is only used to download a model, which is
  checksum-verified.
- The text is pasted into the focused window (Ctrl+Shift+V in terminals,
  Shift+Insert in xterm/urxvt) and the previous clipboard, including images
  and rich text, is restored. Typing with XTest is the fallback and an
  option; å, ä, ö and any other character work on every keyboard layout.
- A small themed overlay at the bottom of the screen shows a level meter and
  the time while recording, and an animation while transcribing.
- Control panel (`nirudictate settings`): status and microphone test, models
  with download/delete and a hardware-based recommendation, shortcut and
  mode, output, vocabulary, history and advanced settings. It follows the
  NIRUX theme live.
- Vocabulary list passed to the model as a hint; optional local history
  (mode 0600, retention limit, clear all).
- Resource-lean: idle service at 0 % CPU with no timers and a few MB of
  memory; the microphone is open only while recording; the model loads while
  you speak and is unloaded after 10 idle minutes; transcription runs at low
  priority with a bounded number of threads.
- Live state for status bars (`$XDG_RUNTIME_DIR/nirudictate/state.json` and
  the root property `_NIRUDICTATE_STATE`), used by NIRUBAR's `dictation`
  module. Start/stop/done/error sounds through `nirux sounds`.
- CLI: `start`, `stop`, `toggle`, `cancel`, `status [--json]`, `language`,
  `models list|recommend|install|delete|verify`, `transcribe`, `bench`,
  `doctor`.
- Prebuilt x86_64 binaries for glibc 2.35+ (Debian 12/13, Arch, Void) with
  two whisper.cpp engines: AVX2/FMA and a baseline for older CPUs, picked at
  start-up.
