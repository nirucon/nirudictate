# Measurements

All numbers from **othala**: Debian 13, Intel Xeon W-1250 (6 cores / 12
threads, AVX2), 32 GB, NIRUX session with a browser, mail client and music
player open (a realistic desktop, not an idle benchmark machine). NIRUDICTATE
0.1.0 defaults: 5 threads, nice 10, SCHED_BATCH, greedy decoding, full 30 s
window, x86-64-v3 engine.

## Test material

Synthetic speech from [Piper](https://github.com/rhasspy/piper) (voices
`sv_SE-nst-medium`, `en_US-lessac-medium`): eight Swedish sentences (102
words, with å, ä, ö, numbers, names) and five English sentences (59 words),
2–8 s each. Word error rate (WER) is computed after lower-casing and removing
punctuation. Synthetic speech is clean and easy, so the WER here is lower than
for real voices in real rooms; it is used to compare models and settings, not
to promise accuracy. "Time" is the transcription after the model is loaded
(the service loads it while you speak).

## Models

| Model | Format | WER | Time per dictation |
|---|---|---|---|
| KB-Whisper Tiny | q8_0 | 6.9 % | 0.47 s |
| KB-Whisper Tiny | f16 (KBLab) | 6.9 % | 0.52 s |
| KB-Whisper Base | q8_0 | 5.9 % | 0.94 s |
| KB-Whisper Base | f16 (KBLab) | 6.9 % | 1.07 s |
| KB-Whisper Small | **q8_0** | **4.9 %** | **3.04 s** |
| KB-Whisper Small | f16 (KBLab) | 4.9 % | 3.54 s |
| KB-Whisper Small | q5_0 (KBLab) | 4.9 % | 3.71 s |
| KB-Whisper Medium | q8_0 | 4.9 % | 8.91 s |
| KB-Whisper Medium | q5_0 (KBLab) | 4.9 % | 11.27 s |
| KB-Whisper Large | q8_0 (3 sentences) | 4.2 % | 16.9 s |
| Whisper Tiny (English) | q8_0 | 3.4 % | 0.44 s |
| Whisper Base (English) | q8_0 | 6.8 % | 0.90 s |
| Whisper Small (English) | q8_0 | 6.8 % | 2.89 s |
| Whisper Medium (English) | q8_0 | 5.1 % | 8.66 s |
| Whisper Large v3 Turbo | q8_0 | 5.1 % | 14.2 s |

The 8-bit (q8_0) files are the fastest on this CPU at the same accuracy, which
is why NIRUDICTATE uses them (and converted KB-Whisper to q8_0, see
[MODELS.md](MODELS.md)). The relative costs (tiny = 1, base 2, small 6.5,
medium 19, large 36, turbo 32) are stored in `src/models.rs` and drive the
recommendation together with the measured or estimated speed of the computer.

### Shortened encoder window (not used)

Whisper always encodes 30 s. Encoding only the frames that contain speech
(`audio_ctx`) is much faster, but in these tests the small models then repeat
themselves:

| Model | Window | WER | Time |
|---|---|---|---|
| Tiny | speech + 2.5 s | 225 % (repetition loops) | 0.68 s |
| Base | speech + 2.5 s | 208 % (repetition loops) | 0.98 s |
| Small | speech + 2.5 s | 9.8 % | 0.87 s |
| Small | at least 768 frames | 7.8 % | 1.34 s |
| Small | at least 1024 frames | 7.8 % | 1.85 s |
| Small | full (default) | 4.9 % | 3.04 s |

Accuracy wins: the full window is the default. `NIRUDICTATE_AUDIO_CTX=768`
(or `short`) in the service's environment enables the shortened window for
experiments.

Flash attention on the CPU was slower (Small: 4.25 s vs 3.04 s) and is off.

## Recommendation on othala

Estimated time for KB-Whisper Small: 3.1 s, under the 3.5 s limit; Medium
8.9 s is over it. Recommended: **Small** (Swedish) and **Small** (English).
A 4-core laptop without spare cores is estimated at about 3.7 s for Small and
gets **Base**; a CPU without AVX2 gets Tiny. After a few dictations the
measured speed replaces the estimate.

## Resources (service)

| State | RSS | Threads | CPU | Context switches |
|---|---|---|---|---|
| Idle, 60 s | 7.4 MB | 4 | 0 ticks | 0 (15 → 15) |
| Model loaded (KB-Whisper Small q8_0) | 279 MB | 4 (+5 while transcribing) | — | — |
| 70 s after the last dictation with `unload_after_minutes = 1` | 9.3 MB | 4 | 0 | 1 |

While idle the service has no child processes (no recorder), no timer and no
open audio stream. The microphone recorder exists only while recording.

## CAVA frame pacing during transcription

A CAVA visualizer (`cava`, PipeWire input, raw output) analysed a music signal
while the service transcribed KB-Whisper Small dictations back to back (5
transcriptions in 14 s, the engine at nice 10 + SCHED_BATCH, 5 threads on 6
cores). Intervals between CAVA's frame writes, traced with `strace -tt`:

| Window | Frames | Median | p95 | p99 | Max | Gaps > 25 ms |
|---|---|---|---|---|---|---|
| Music only (12 s) | 2420 | 5.3 ms | 7.2 ms | 10.7 ms | 14.6 ms | 0 |
| Music + transcription (14 s) | 3516 | 5.3 ms | 6.5 ms | 7.3 ms | 12.0 ms | 0 |

CAVA's cadence is unchanged while the CPU is busy with transcription: the
low-priority engine threads yield to it.

## Latency of the dictation itself

From the end of speech (silence detected, or the key released) to the text in
the window, KB-Whisper Small on othala: 3.0–3.3 s, of which transcription is
~3.1 s and pasting 0.1–0.3 s. Loading the model on the first dictation
(0.1–0.2 s for Small) happens while the user speaks.

## Engines

The released binaries on othala (KB-Whisper Small, the 8.3 s sentence):

| Engine | Time |
|---|---|
| `nirudictate` (AVX2/FMA/F16C, x86-64-v3) | 3.41 s |
| `nirudictate-baseline` (SSE4.2, forced with `NIRUDICTATE_NO_REEXEC=1`) | 14.35 s |

The baseline engine is about 4.2× slower; the recommendation uses that factor
on CPUs without AVX2, which therefore get Tiny or Base.

## Voice commands

NIRUDICTATE 0.3.0 on othala, KB-Whisper Small (q8_0) for Swedish and Whisper
Small (English, q8_0) installed, 5 threads, full 30 s window. Test clips:
50 Swedish commands spoken by two Piper voices (`sv_SE-nst-medium`,
`sv_SE-lisa-medium`, 100 clips) and the same 50 commands in English by four
(`en_US-lessac`, `en_US-amy`, `en_GB-alan`, `en_US-ryan`, 200 clips); 48 are
commands, 2 per language are questions that must not run anything. Each
clip went through `nirudictate command --dry-run --file` and counts as
right when it gives the same intent as the written phrase. "Wrong" means a
different command; "not understood" means nothing would run. Time is the
transcription after the model is loaded (the service loads it while you
speak), both passes together when the second model was needed.

The Piper Swedish voices are hard: KB-Whisper writes "Lås armen" for "Lås
skärmen" and "Hellerm" for "Helskärm". Real voices in a quiet room do
better; the numbers compare settings, they do not promise accuracy.

| Setting | Swedish clips | English clips | Time (median) |
|---|---|---|---|
| Swedish model only, command prompt (first matcher) | 61 right, 1 wrong | 77 right, 7 wrong | 3.2 s |
| Swedish model only, no prompt (first matcher) | 58 right, 0 wrong | 101 right, 4 wrong | 2.8 s |
| Swedish model, language detection (`auto`, first matcher) | 61 right, 1 wrong | 77 right, 7 wrong | 5.9 s |
| English model, English prompt (first matcher) | | 183 right, 1 wrong | 3.0 s |
| Swedish model only, command prompt (final matcher) | 78 right, 1 wrong | 118 right, 4 wrong | 3.2 s |
| **`both` (default): Swedish model, then English without a prompt** | **78 / 100 right, 1 wrong** | **186 / 200 right, 4 wrong** | **3.3 s** (max 7.4 s and 8.1 s) |
| `both`, shortened window (`NIRUDICTATE_AUDIO_CTX=768`) | 73 right, 1 wrong | 92 / 100 right, 3 wrong (two voices) | 1.5 s |

What this decided:

- **Force the language, do not detect it.** KB-Whisper's language detection
  said Swedish for every English clip, so `auto` gave exactly the same text
  and took almost twice as long. `auto` stays as a setting but is not the
  default.
- **Two models beat one.** With the Swedish model alone 59 % of the English
  clips were understood (KB-Whisper often writes them in Swedish, which the
  matcher handles, but not always well); trying the English model when the
  Swedish text matched nothing gives 93 % at the cost of a second pass for
  those clips only. Swedish clips are unaffected (the English model gets no
  prompt; with the command prompt it once wrote "Open terminal" for
  unclear Swedish speech, a wrong command, so the prompt was dropped for the
  second pass).
- **The command prompt helps Swedish (78 vs 70 with the final matcher) and
  the program names in it do no harm**, but on English speech it pushes
  KB-Whisper to translate; the second pass covers that.
- **Near misses and the KB translations** in the matcher raised the Swedish
  model's results from 61 to 78 (Swedish) and from 77 to 118 (English)
  without new wrong commands.
- Every "wrong" result in the final setting is "open X" with a misheard
  name (`Öppna nästa menyn`, `Öppna lauren`): the launcher then shows its
  list with that text unless a program matches it clearly. No wrong
  workspace, window or power command was produced in 300 clips.
- **The full window stays.** The shortened encoder window halves the time
  but loses 5 Swedish commands in 100.
