# DiscoFM 1.0.0 — features and verification

DiscoFM provides a desktop editor and command-line tools for YM2612/YM3438
instruments. Windows x64/MSVC is the validated platform. Other platforms require
a compatible Rust toolchain, C/C++ compiler and audio/windowing backend; runtime
behavior on them has not been verified.

## Available features

| Area | Behavior |
|---|---|
| Synthesis | Nuked-OPN2 native rendering, YM2612/YM3438 profiles, NTSC/PAL clocks, six channels and stereo output |
| FM model | Validated register encodings, eight algorithms, S1/S2/S3/S4 identities, SSG-EG, LFO, feedback and channel 3 special frequencies |
| Editor | Four operator cards, routing diagrams, chip envelope traces, tooltips, numeric controls, undo/redo and an unsaved-changes dialog |
| Playback | CPAL output selection, keyboard/piano/MIDI audition, release/Stop, octave/profile changes and finite A/B clips |
| References | WAV analysis and SF2/SF3/SFZ/DLS sample extraction, catalog search and whole-sample playback |
| Analysis | Monophonic YIN pitch with manual override, FFT/STFT, harmonics, envelopes, centroid, spectral flux and transient candidates |
| Comparison | Shared reference/chip interval, channel, DC setting and gain; optional whole-note level matching |
| Fitting | Basic/advanced DE and refinement, parameter locks, automatic LFO/AM/channel 3 policies, cancellation and ranked history |
| Compute | Parallel CPU rendering/scoring; optional wgpu FFT and spectral reductions; measured Auto backend selection |
| Sessions | Version 1 JSON, validated optional reference/fitting state, up to eight fitting runs and a 1 MiB read/write limit |
| Instruments | TFI/VGI/TYI import/export, patch clipboard and a folder-based patch library |
| Diagnostics | Pin comparisons, independent ymfm comparisons, register traces, application logs and supervised GUI crash reports |

The application version is 1.0.0. The session format remains version 1 and the
fitting objective is version 6. Archived objective versions 1–5 remain readable
with their original scores. Scores from different objective versions are not
directly comparable.

## Verification

The Windows x64/MSVC check for version 1.0.0 on 2026-10-22 passed 163 automated
tests; 18 optional tests remained ignored. Formatting and all-target clippy
with warnings denied passed. `discofm --version` reported `DiscoFM 1.0.0`,
and `validate` retained both exact pin hashes with zero mismatched ticks.

Run from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo run -- validate
```

Use `--workspace`: the default workspace member is the application, so commands
without it do not select every library crate's tests.

The automated suite covers register bounds/mappings, algorithm carriers and
causal paths, envelopes/SSG/LFO, profile isolation, channel 3 frequencies, native
bus timing and offline/live equivalence. Streaming sinc tests compare samples
and frame counts from 8 to 192 kHz. Transport tests cover queue saturation,
stale events, Stop, shutdown and fatal-device muting. Additional tests cover
WAV/bank decoding, analysis, fitting determinism and locks, cancellation,
archive validation, session compatibility, instrument formats and CLI outputs.

Default-note pin regressions retain these fixed integration baselines:

| Profile | Captured ticks | FNV-1a pin hash |
|---|---:|---|
| YM2612 discrete | 127848 | `e5354d82d454959d` |
| YM3438 | 127848 | `8dc66ae06d69b2dd` |

The C comparison driver uses the same pinned Nuked-OPN2 core. Its agreement
verifies integration, while ymfm supplies an independent sample-level numerical
comparison. Neither establishes physical-chip accuracy.

## Optional checks

Tests marked `#[ignore]` require hardware, external files or longer runs. The
normal workspace suite does not run them automatically.

| Check | Requirement |
|---|---|
| `discofm-audio` `device_smoke` | Windows audio output and a quiet WASAPI loopback mix |
| `discofm-audio` `clips_device` | Physical output for clip completion, switching and Stop |
| `discofm-fit` GPU numerical tests | A supported physical wgpu adapter |
| `bass_benchmark` / `steeldrum_benchmark` | `DISCOFM_TEST_BASS_WAV` / `DISCOFM_TEST_STEELDRUM_WAV` |
| `discofm-analysis` installed WAV checks | `DISCOFM_TEST_WAV` / `DISCOFM_TEST_MANDOLIN_WAV` |
| `discofm-soundfont` installed bank check | `DISCOFM_TEST_SOUNDFONT` |
| `long_benchmark`, `hard_targets`, `refit_stress` | Longer fitting/throughput checks; see test headers for controls |

For example:

```powershell
cargo test -p discofm-audio --test device_smoke -- --ignored --nocapture
cargo test -p discofm-audio --test clips_device -- --ignored --nocapture
cargo test -p discofm-fit --lib gpu_ -- --ignored --nocapture
```

Keep copyrighted reference audio outside version control. Example sessions use
relative paths; see [example setup](../examples/README.md).

## Limits

No physical YM2612/YM3438 recording has been supplied. The discrete DAC branch
inherits an upstream approximation marked unverified; external console filters
and analog noise are not modeled. OS loopback verifies the mixer path rather
than speaker output or chip hardware.

Fitting is a finite heuristic search. A lower objective value does not prove an
original patch or perceptual identity. Channel 3 special frequencies are absolute
and do not transpose across the keyboard. Bank import extracts samples without
synthesizing MIDI presets, articulation or bank loops. Analysis is monophonic
and uses selected regions of up to 30 seconds; reference import has no duration
admission limit.

CPU is the default compute backend. GPU and Auto are experimental and accelerate
spectral work only; Nuked-OPN2 synthesis remains on CPU. Audio/device scheduling
and system load affect latency and throughput.

See [hardware accuracy](HARDWARE_ACCURACY.md), [fitting](FITTING.md),
[analysis](REFERENCE_ANALYSIS.md), [soundfonts](SOUNDFONTS.md) and the archived
[automatic-mode](AUTO_MODES_GPU.md) and
[precision/performance](PRECISION_PERFORMANCE.md) measurements for details.
