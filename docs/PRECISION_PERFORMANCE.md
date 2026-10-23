# Precision and performance — benchmark archive

Historical measurements for objective versions 1–3 are recorded here. For the
current automatic modes, loss v6, parallel synthesis and GPU spectral
reductions, consult [AUTO_MODES_GPU.md](AUTO_MODES_GPU.md) and [FITTING.md](FITTING.md).
M7 and M8 below identify the stored baseline and updated benchmark runs, not
application release versions.

## Acceptance and comparison scope

The same installed FL Studio `BASS_EfEm_C2.wav`, seed 2612, initial default patch,
512 evaluations, population 16, note-on 0 and key-off 0.2 s form the baseline.
Its PCM16 mono recording has 10510 frames at 44100 Hz (0.238322 s), RMS 0.4470,
peak 0.9993. Its original YIN estimate was 65.583 Hz; new automatic analysis
refines fine tuning spectrally before fitting. Its audio is not distributed here.

Acceptance measures a whole-note Hann FFT, spectral cosine in 30–1000 Hz and
30–10000 Hz, plus 40 ms RMS envelopes with 5 ms hops. Fixed alignment, DC removal
and the same optional whole-note RMS calibration apply to both versions. These
metrics are independent of the fitter's multi-resolution STFT score. The
requirements are at least 10% improvement in one distance, no >2% regression in
the others and a dominant-peak difference <20 cents. The opt-in benchmark checks
them against the saved M7 patch in `tests/fixtures/bass-m7-baseline.json`.

Executed local 512-evaluation CPU test:

| Metric | M7 baseline | M8 |
|---|---:|---:|
| Spectral cosine, 30–1000 Hz (higher better) | 0.885167 | 0.942526 |
| Spectral cosine, 30–10000 Hz | 0.884830 | 0.935390 |
| Envelope relative error (lower better) | 0.086221 | 0.076371 |

This reduces full-spectrum cosine distance by 43.9% and envelope error by 11.4%.
The original M7 release run took 21.43 s, with 446 chip renders. M8 release
took 11.92 s with 443 renders and 69 cache hits: 44.4% less elapsed time (1.80×),
on the same machine, sample, budget and starting patch. Search proposals also
changed, so this is whole-fitter performance, not an isolated kernel speedup.
The final CPU/GPU 512-query sessions and independent comparison are saved under
`target/m8-bass-*-pitch-final.json`. Loss v1/v2/v3 scores are not directly comparable,
and none of these metrics guarantees perceptual identity.

Re-evaluating a prior candidate with the current pitch/objective and refining it
for another 1024 queries produced full cosine 0.954587 and envelope error
0.074992. The resulting patch is saved in
`examples/bass-efem-fitted-session.json`, separately from the 512-query timing
comparison above. Individual metrics can trade places during refinement; a
lower combined loss need not improve every independent measure.

## Efficiency and GPU

Sinc filtering retains the same Blackman window, cutoff, support and time zero.
2048-phase tables with linear interpolation remove trigonometric calculations
from each output tap. A four-entry shared cache retains at most ~64.1 MiB of
tables; common native-to-24 kHz tables are ~2.4 MiB. Extreme ratios use analytic
sinc. Tests compare against that original analytic kernel with <5e-7 maximum
sample difference, retain stopband assertions, and preserve exact streaming/
offline correspondence. Native chip pins are unchanged.

Reference logarithms/energies and up to 20000 complete register-key scores are
also cached by the fitter, alongside retained reference FFT layouts/plans and
candidate scratch. Structured ratio/envelope starts seed advanced search, and
its local-refinement allocation doubles to 30%. Nuked-OPN2 still renders every
candidate.

Envelope and tail scoring average pitch-adaptive 10–40 ms and fixed 40 ms RMS
windows, with 5 ms hops and logarithmic protection for quiet decays. The tail
always includes the final quarter, even when key-off equals the reference end.
This prevents a nearly silent late candidate from receiving an empty tail score.
The objective is version 3; versions 1/2 remain readable with their saved scores.

## Mandolin A/B correction

The reported `STR_MandoLynn_B2.wav` uses Vorbis ACM inside RIFF. The installed
file decodes to 41773 mono frames at 44100 Hz (0.947234 s). With FFT 8192,
DC removal, normalization and silence trimming, YIN originally estimated
245.71 Hz although the whole-note fundamental is near 247.09 Hz. A bounded
Hann spectral refinement now estimates 246.965 Hz on the strongest YIN region;
it preserves the octave and manual override. It requires a resolved fundamental
within ±25 cents and sufficient energy, rather than selecting the loudest harmonic.

One source interval, channel, 48 kHz rate, DC option and reference normalization
gain now serve A/B. Level matching optionally applies one RMS gain across all
of B. Listening and plots share prepared buffers; a common attenuation supplies
playback headroom to both clips when needed. Missing B tails stay visible as
silence padding. Imported one-shots use end-of-note key-off by default, with
explicit timing markers still set manually. A selected fitting result must
still be applied. A visible button indicates when B remains the editor's
previous patch.

The prior 2048-query candidate, refined with 1024 additional queries and the
corrected pitch, increased independent full-spectrum cosine from 0.379058 to
0.828123 and reduced envelope error from 0.067603 to 0.060955. The resulting
dominant peak is 247.043 Hz versus the reference's 247.091 Hz. These gains include
both corrected tuning and parameter refinement, not a pitch-only experiment.
The ready session is `examples/mandolynn-fitted-session.json`; it retains FFT8192,
DC removal, normalization, trim and full-duration key-off, plus both search runs.

wgpu 30.0.1 performs actual radix-2 FFTs, with one workgroup per Hann window,
sizes 512/1024/4096, 256 threads and up to 32 KiB shared memory. Device/pipelines
are shared; signal, windows, centers, output and readback buffers persist per run.
GPU only accelerates spectral analysis; synthesis, resampling and remaining
feature/loss calculations stay on CPU. No approximate GPU synthesizer is used.

Auto warms and checks GPU magnitudes against CPU, including dimensions,
finiteness, absolute and relative errors. It measures three transfers/dispatches/
readbacks per backend and requires a 10% median improvement to select GPU.
Forced GPU never silently substitutes CPU. Auto initialization/calibration
failure retains CPU and records the reason; search-time device errors are visible.
Readback polls cancellation, has a five-second deadline and always unmaps buffers.
Worker disposal does not block the UI while a driver initializes.

The local RTX 3050 Laptop (4 GiB, driver 616.92) successfully ran Vulkan compute.
The final short-bass calibration measured CPU FFT 0.322 ms and GPU 1.107 ms;
Auto therefore prefers CPU for this short clip. Forced GPU took 13.96 s for the
same 512 evaluations/443 renders, with 1341 real compute dispatches and the same
best loss and independent metrics as CPU. Those FFT numbers include readback,
not shader-only timing. GPU arithmetic can change near-tied scores across devices;
select CPU for reproducible scores/ranking by seed. Actual backend, adapter,
dispatch count, timings and cache counts are saved in each new run.

GPU was selected for the first 2048-query mandolin run (CPU 1.189 ms, GPU
0.897 ms), while the final refinement chose CPU (1.099 ms versus GPU 1.222 ms).
Because driver/device/system load changes timings, Auto measures every run
instead of promising a GPU speedup. Most CPU time still goes to the native
native renderer.

## Independent ymfm and real recordings

Unmodified ymfm is pinned at `81aec25ccbb98f4873a255f7551ac4dadac59b4a`,
BSD-3-Clause. It is compiled as C++14 behind a separate C ABI and used only for
comparison. Each render constructs a new interface and concrete YM2612/YM3438
object. The regular note API does not schedule CSM/timers.

The external ymfm rate keeps Nuked's fractional NTSC/PAL clock, with 144 master
clocks per sample. Ordered bus events take effect at the next sample boundary,
less than one native frame later. The comparison therefore measures sample-level
behavior rather than bit-exact chip pins.

Fixed scales match DiscoFM's existing pin integrator:
YM3438 = raw/32768 ×0.75; YM2612 = raw/32768 ×0.76171875, accounting for DAC
duty cycle and ymfm's 64/65 compensation. No gain is fitted to ymfm per patch.
Tests cover both profiles, both clocks, both register banks, continuous tuning,
eight algorithms, envelopes, detune, SSG and LFO. Spectral/envelope tolerances
are explicit; waveform differences remain reported.

`compare --session patch.json` compares with ymfm. Add `--reference recording.wav`
for an independently recorded note. `--start`, `--align-ms` (0–20 ms) and
`--match-level` are explicit recording controls. The selected recording must
contain the session's full duration. `--output` requires a new JSON report and
includes patch, request and source revisions. No physical YM2612/YM3438 recording
was provided, so physical hardware agreement remains unverified.

## Reproduce

Final verification on Windows x64: 108 automatic workspace tests, fmt and
all-target clippy with warnings denied passed. Exact installed bass/mandolin
opt-in regressions and physical-GPU FFT numerical tests also passed. Final
release `validate` retained both exact pin hashes and passed independent ymfm
carrier checks. Native release GUI checks covered the mandolin comparison,
fitting view and bass fitting; each window closed normally.

```powershell
cargo test --workspace --release --locked --offline
cargo test -p discofm-fit --lib gpu_fft --locked --offline -- --ignored --nocapture
$env:DISCOFM_TEST_BASS_WAV = 'C:\Program Files\Image-Line\FL Studio 2024\Data\Patches\Packs\Legacy\Instruments\Bass\BASS_EfEm_C2.wav'
cargo test -p discofm-fit --test bass_benchmark --locked --offline -- --ignored --nocapture
$env:DISCOFM_TEST_MANDOLIN_WAV = 'C:\Program Files\Image-Line\FL Studio 2024\Data\Patches\Packs\Legacy\Instruments\Strings\STR_MandoLynn_B2.wav'
cargo test -p discofm-analysis --test wav_variants mandolin --locked --offline -- --ignored --nocapture
cargo run --release -- fit --reference $env:DISCOFM_TEST_BASS_WAV --output target/bass-cpu.json --compute cpu --key-off 0.2
cargo run --release -- fit --reference $env:DISCOFM_TEST_BASS_WAV --output target/bass-gpu.json --compute gpu --key-off 0.2
cargo run --release -- compare --session target/bass-cpu.json --reference $env:DISCOFM_TEST_BASS_WAV --match-level --output target/bass-comparison.json
cargo run --release -- validate
```
