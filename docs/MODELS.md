# Models: sources, conversion and checksums

NIRUDICTATE downloads one ggml model file per language and verifies its
SHA-256 against `src/models.rs` before using it.

## Swedish: KB-Whisper (Apache-2.0)

KB-Whisper is a family of Whisper models fine-tuned on more than 50 000 hours
of Swedish speech by **KBLab**, the data lab of the National Library of
Sweden (Kungliga biblioteket). License: Apache License 2.0. Model cards:
<https://huggingface.co/KBLab>.

KBLab publishes whisper.cpp files in every model repository: `ggml-model.bin`
(16-bit) and `ggml-model-q5_0.bin` (5-bit). On x86 CPUs the 8-bit format is
the fastest at full accuracy (see [MEASUREMENTS.md](MEASUREMENTS.md)), and no
8-bit files existed, so they were made from KBLab's 16-bit files:

```sh
# whisper.cpp v1.8.3, examples/quantize
whisper-quantize ggml-model.bin kb-whisper-SIZE-q8_0.bin q8_0
```

| Size | Source (Hugging Face repo @ commit) | Source SHA-256 (= LFS oid) | Published file | SHA-256 |
|---|---|---|---|---|
| tiny | KBLab/kb-whisper-tiny @ `76d796af43a50fa34321efa562c9b9887a187463` | `054187c9…13233118b03` | kb-whisper-tiny-q8_0.bin (43 537 450 B) | `c16fb9e07e9287d931d0c0d9a6683367dc8683eac8df34a0b89bb4ec98893fb0` |
| base | KBLab/kb-whisper-base @ `1499d2d2f0c7ed545bd6f2eec85287cf8d8c8b38` | `f5e3cdb3…ec5de30`… see below | kb-whisper-base-q8_0.bin (81 768 602 B) | `2027be2ad0a60dcb3fa389889669e3b6f5d1e45df7d7a39f090055ad0ec5de30` |
| small | KBLab/kb-whisper-small @ `3564d61a42fc210ceaa55a22a96dd64478959c78` | `de691133…170225b7`… see below | kb-whisper-small-q8_0.bin (264 464 624 B) | `0073fd16fca4ec90e37703c07a20f717f536ef718a7fcb050dbf3979b18255a3` |
| medium | KBLab/kb-whisper-medium @ `0abe10b9d7f75d0902656e5c06c5c4d549604dc5` | `1b7842bc…dd8b015` | kb-whisper-medium-q8_0.bin (823 369 796 B) | `1dd1ace68932cb3f3647882dd572ade251dbc6e4d8e96363e8979fcd434ca358` |
| large | KBLab/kb-whisper-large @ `d5d5984b4d8f7c4847a8ea203f1976285fb28300` | `b66f2dda…f999487`… see below | kb-whisper-large-q8_0.bin (1 656 538 283 B) | `339d086f52e4f5752647ce6bf67cc02cebf197703a5ce4cc2f4252b0c47b38d8` |

Full source checksums (`ggml-model.bin`, identical to the LFS object ids of
the commits above):

```
054187c95948ee0455d428db0c0d6c84d6c6157dab72e86857ced13233118b03  tiny
f5e3cdb33e537eedfa2a749b5cae28c4c511873a1b13362f87dffbe07891d3fe  base
de6911330cbdc131362f7a955682b65c8a5a2394caba73e7ea821a9822efb8c6  small
1b7842bc1c3f79fb3bf043a0a3590961d625a49ef3ccbdceb00e738c5dd8b015  medium
b66f2dda369a88f6c03fe37326d7cc37aa216f6f34e6fc1be686e497ba9c2f39  large
```

The converted files are published as the
[`models-sv-1`](https://github.com/nirucon/nirudictate/releases/tag/models-sv-1)
release of this repository together with the Apache-2.0 license text and a
`SHA256SUMS` file. The quantization of the weights is the only change. A new
conversion gets a new release tag (`models-sv-2`, …); published files are never
replaced.

## English: OpenAI Whisper (MIT)

The whisper.cpp project's own 8-bit conversions of OpenAI's Whisper models,
from <https://huggingface.co/ggerganov/whisper.cpp> at commit
`5359861c739e955e79d9a303bcbc70fb988958b1`:

| Id | File | SHA-256 |
|---|---|---|
| en-tiny | ggml-tiny.en-q8_0.bin | `5bc2b3860aa151a4c6e7bb095e1fcce7cf12c7b020ca08dcec0c6d018bb7dd94` |
| en-base | ggml-base.en-q8_0.bin | `a4d4a0768075e13cfd7e19df3ae2dbc4a68d37d36a7dad45e8410c9a34f8c87e` |
| en-small | ggml-small.en-q8_0.bin | `67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067` |
| en-medium | ggml-medium.en-q8_0.bin | `43fa2cd084de5a04399a896a9a7a786064e221365c01700cea4666005218f11c` |
| en-turbo | ggml-large-v3-turbo-q8_0.bin | `317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1` |

Whisper's weights are released by OpenAI under the MIT license; whisper.cpp
is MIT licensed.
