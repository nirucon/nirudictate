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
