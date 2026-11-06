# Working on NIRUDICTATE

NIRUDICTATE is a NIRUX extra: local speech-to-text dictation for X11. Read
`README.md` (behaviour and configuration), `docs/ARCHITECTURE.md` (threads,
state, protocols) and `docs/TESTING.md` (how it was verified) first.

## Invariants

- **Local only.** The network is used for model downloads and nothing else.
  Every model URL is pinned to a Hugging Face commit and every file to its
  SHA-256 (`src/models.rs`). Never log or upload audio or text.
- **Idle is free.** While idle the service sleeps in one `poll` with no
  timeout: no microphone, no model, no timers, no animation. The microphone is
  a child process that exists only while recording; the model is loaded on
  first use and dropped after `unload_after_minutes`.
- **The desktop stays smooth.** Transcription runs on the engine thread at
  nice 10 + SCHED_BATCH with a bounded thread count; whisper.cpp's threads
  inherit that. Measure frame pacing of a CAVA visualizer when changing it.
- **Portable binaries.** No `-march=native`. `.cargo/config.toml` fixes the
  whisper.cpp CPU features (x86-64-v3); the release also builds a baseline
  engine (SSE4.2) that the main binary execs on CPUs without AVX2/FMA.
  The release builds on Ubuntu 22.04 (glibc 2.35).
- **NIRUWM is not changed for the shortcut.** The service grabs it itself so
  it sees the key release (push-to-talk).
- **Clipboard manners.** Paste restores the previous clipboard with every
  target it offered (images, rich text). Terminals paste with Ctrl+Shift+V,
  xterm/urxvt with Shift+Insert (PRIMARY).
- **Files:** settings `~/.config/nirudictate/config.toml` (0600), models
  `~/.local/share/nirudictate/models/`, history and measurements
  `~/.local/state/nirudictate/` (0600), socket and live state
  `$XDG_RUNTIME_DIR/nirudictate/`.
- The live state file (`state.json`) and the root property
  `_NIRUDICTATE_STATE` are a public contract for NIRUBAR; keep them
  compatible (`version: 1`).
- Sounds belong to NIRUX (`nirux sounds play dictation-*`); do not ship or
  play sound files here.

## Gates

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
bash -n install.sh uninstall.sh && shellcheck -S warning install.sh uninstall.sh
```

Rust 1.88 or newer (whisper-rs). cmake and a C++ compiler build whisper.cpp.

## Release

Bump `Cargo.toml`, add a `# NIRUDICTATE X.Y.Z` section to `CHANGELOG.md`, tag
`vX.Y.Z`. `release.yml` tests, builds both engines, and publishes
`nirudictate-X.Y.Z-x86_64-linux-gnu.tar.gz` with `SHA256SUMS-X.Y.Z`. Then pin
the tag tarball's SHA-256 in `nirucon/nirux` (`extras_table`).
