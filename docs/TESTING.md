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
