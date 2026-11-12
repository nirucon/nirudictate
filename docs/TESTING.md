# Testing

## Automated (CI)

`cargo test --locked` covers, without an X server or audio device:

- shortcut parsing and the hybrid/toggle/hold state machine, including
  repeated presses and presses while transcribing;
- settings defaults, round trip, partial files and validation;
- the model catalog (ids, checksums, URLs) and the hardware recommendation;
- voice-activity detection, silence trimming, WAV reading/writing and
  resampling, the Sun AU header pw-record writes to pipes, `pw-dump` parsing;
- text clean-up (spacing, capitals, trailing space, silence hallucinations
  such as subtitle credits) and the vocabulary prompt;
- history append/trim/delete/clear and its 0600 mode;
- terminal detection for the paste key and keysyms for å/ä/ö and Unicode;
- the overlay renderer (every view, transparent corners);
- the control-socket wire format;
- the NIRUX/Omarchy theme resolution and live watcher (shared with NIRUAMP).

CI also runs rustfmt, clippy `-D warnings`, the MSRV (Rust 1.88) build and
shellcheck on the installers. The release workflow checks that both engines
were configured as intended (AVX2 on/off) and that both binaries start.

## On a real desktop (othala, NIRUX, Debian 13)

othala has no microphone. A PipeWire virtual source was used instead:

```sh
pw-loopback -n nd-vmic \
  --capture-props='media.class=Audio/Sink node.name=nd_vsink audio.position=[MONO]' \
  --playback-props='media.class=Audio/Source node.name=nd_vmic audio.position=[MONO]' &
# [advanced] microphone = "nd_vmic"
pw-play --target nd_vsink speech.wav     # "speak"
```

Speech came from Piper (Swedish `sv_SE-nst-medium`, English
`en_US-lessac-medium`). The shortcut was pressed with XTest
(`xdotool keydown/keyup ctrl+alt+space`), which goes through the same X
passive grab as a keyboard.

| Test | Result |
|---|---|
| Ctrl+Alt+Space grab with NIRUWM running | grabbed (no conflict) |
| Hold (push-to-talk), release to insert into kitty | ✓ Ctrl+Shift+V, text with å/ä/ö correct |
| Tap, auto-stop after 2 s of silence, kitty | ✓ |
| Key auto-repeat while held does not stop the recording | ✓ (after fix) |
| Previous clipboard text restored after pasting | ✓ `ORIGINAL-CLIP åäö` |
| GTK entry (zenity) | ✓ Ctrl+V |
| Image on the clipboard restored byte for byte | ✓ (`image/png`, SHA-256 equal) |
| xterm: Shift+Insert, PRIMARY restored | ✓ |
| Typing mode (XTest) into a GTK entry, "Åsa" | ✓ |
| Clipboard manager (niruclipboard polling) does not count as the paste | ✓ (after fix) |
| English model, kitty | ✓ |
| Overlay: recording, push-to-talk, transcribing | screenshots below |
| Panel in three themes, live theme switch | screenshots below |
| NIRUBAR `dictation` module: idle / recording / transcribing | ✓ hidden / red ● / accent … |
| Idle CPU, RSS, wakeups | 0 ticks, 7.4 MB, 0 context switches in 60 s |
| Model unload after the idle time | 279 MB → 9.3 MB |
| CAVA frame pacing during transcription | unchanged, see MEASUREMENTS.md |
| Locked screen | the insert refuses when another client holds the keyboard |

Screenshots of the panel and the overlay are attached to the 0.1.0 release
report; the panel adapts to a narrow tiled column (tabs move to the top).

## By hand

```sh
nirudictate doctor                 # everything the service needs
nirudictate status --json
nirudictate transcribe file.wav    # through the running service
nirudictate bench file.wav sv-base sv-small
```

## Agent mode (0.2.0)

- Unit tests: version parsing for both opencode styles, the request paths and
  bodies of both APIs (default agent unless configured), the TUI command lines
  (no `--auto`, no password), terminal choice and class, basic auth, the
  scratchpad property, XKB conflicts, the double tap, the confirmation box
  wrapping, config defaults and validation, and the IPC wire form.
- Isolated Xvfb on othala (`:57`, the release NIRUWM 0.17.9 build with
  bar and wallpaper off, kitty, the real opencode 1.19.0 and its configured
  provider): `nirudictate agent file speech.wav` with a WAV made with the
  system libflite (`cmu_us_slt`) → confirmation box → auto-send after 2 s →
  the default `build` agent answers in the TUI in scratchpad 9; Esc cancels
  (nothing sent); Enter sends at once; a double tap of Shift+Alt+Space shows
  NEW AI; holding it records for the agent; `agent new` starts a new
  session and the TUI follows it; hiding and `agent show`; closing the TUI
  stops the server and the next message continues the same session; doctor
  with and without an XKB `grp:alt_shift_toggle` clash; a missing opencode
  leaves the key ungrabbed; quitting stops server and TUI.
- opencode 2.x was checked against its source (tag v2.1.3: `serve`,
  `--server`, `--session`, `/api/session`, `/api/session/ID/prompt`,
  `/api/info`, `OPENCODE_PASSWORD`) and `--help` on a 2.1.3 install; it was not
  run end to end.


## Voice commands (0.3.0)

- Unit tests: the matcher's table (over 400 Swedish and English phrasings,
  each checked with both language preferences), what KB-Whisper writes for
  English speech, real near misses from the measurements, over 50 phrases
  that must run nothing, numbers in both languages, every example in the
  rule table parsing to its own rule, the plans (argv, never a shell, never
  `niructl do shell/start/quit`, dangerous ones ask), free text keeping the
  original words, the runner (missing program, failure line, standard input,
  `launched\tName`, an old `niructl`), the program list for the prompt, the
  NIRUWM key clash check, the config, the cheat sheet file and the IPC wire
  form.
- Accuracy and latency with the real models: see
  [MEASUREMENTS.md](MEASUREMENTS.md#voice-commands).
- Isolated Xvfb on othala (`:57`, its own runtime, config, data and state
  directories, sounds off, stub `niructl`, `nirulauncher`, `playerctl`,
  `wpctl`, `nirushot` and `pw-record` that only log their arguments):
  `nirudictate command file` with Piper clips → "Öppnar terminal" and
  `niructl do terminal`; "Vad är klockan" → "Förstod inte: Vad är klockan?"
  with nothing run; "Stäng av datorn" → the red confirmation box, Esc →
  "Avbrutet"; "Shut down the computer" → the English box, `command confirm`
  → `niructl shutdown`; "Open Thunderbird" → `nirulauncher --apps --query
  Thunderbird --launch-best` and "Opening Spotify" from the stub's answer; a
  `niructl` without `do` → "Uppdatera NIRUWM: niructl saknar röstkommandon
  (do)"; `command toggle` → the recording pill with CMD. Screenshots were
  checked by eye in the owner's theme.
- Not tested end to end: the real `niructl do` and
  `nirulauncher --launch-best`/`--note --stdin` (built in parallel); the code
  follows the shared interface and falls back as described.
