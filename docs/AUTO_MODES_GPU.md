# Automatic FM modes and benchmark archive

The measurements below describe the 2026-10-11 implementation and loss v4.
Current worker scheduling, GPU slots, defaults and objective v6 are documented
in [FITTING.md](FITTING.md). The recorded measurements have not been recalculated
for the current objective.

LFO/FM, operator AM/AMS and normal versus channel 3 special mode are now tested
by advanced fitting. Each group has its own Auto/Keep policy in the GUI's search
settings. The corresponding CLI options,
`--lfo`, `--am` and `--channel3`, accept `auto` or `preserve`; locks override the
policies. Auto is used for new runs. Until Auto is selected, legacy sessions
keep their previous channel 3 topology. Basic fitting preserves the existing
advanced controls.

Activation gates are crossed jointly by LFO proposals: disabled, vibrato,
tremolo, combined AM/FM and AM on modulators. All eight hardware rates and every
AMS/FMS sensitivity are available. Auto can disable an unnecessary mode rather
than imposing tremolo on each sample. Reference partials, including inharmonic
frequencies, seed channel 3's S1–S3; S4 keeps the note frequency. Unlike normal
note operators, these absolute registers do not transpose.

The v4 objective reuses the existing multi-resolution STFT to track pitch and up
to twelve partials over time. Exclusive peak assignment, missing-partial penalties,
relative partial energy and phase-tolerant amplitude periodicity supplement
spectral, envelope, transient and tail losses. Faster LFO motion is represented
principally by spectral sidebands. Coherent carrier TL/envelope moves and discrete
carrier removal supplement DE and local search. A final coherent pass revisits
levels/envelopes after independent moves and includes RR when release is audible.
The default envelope weight is 2.5 to balance the newly added tracking evidence;
tail remains 0.5. Loss values from versions 1–3
remain readable but are not directly comparable to v4 values.

## Execution and GPU scope

`--workers 0` selects every logical CPU, up to 16; 1–16 can be set explicitly. Every
render owns a native chip instance. The build adapts the pinned Nuked profile
state into an instance field without editing vendor sources; exact pin/hash tests
compare this path with a separately compiled pristine upstream oracle. Proposal
RNG, population snapshots, cache and score commits remain ordered. CPU worker
count changes scheduling without changing fitting decisions.

(Superseded on 2026-10-22: workers now score whole candidates and own one GPU
slot each; see FITTING.md, "Worker, storage and controls". The measurements
below describe the October 11 implementation.)

The GPU evaluates batches of up to four rendered candidates through
512/1024/4096-point FFTs and spectral-distance reductions. Reference spectra/logs
stay resident and GPU buffers are reused. Only reduced spectral losses and the
largest spectrum needed for other features are read back. Cycle-based
synthesis and sinc conversion run concurrently on the CPU. This is not a GPU
replacement for the YM2612 emulator.

Auto numerically checks the adapter and compares median end-to-end batched
spectral timings, including transfers/readback. It requires at least a 10%
advantage to select GPU. Short clips and small local-search batches can favor CPU.
Forced GPU reports failures instead of silently falling back. The reported WGSL
`target` reserved-name error is fixed by `reference_spectrum`; all specialized
FFT/loss shaders also parse and validate in an automatic no-GPU regression test.

Native blocks, resampling, FFT loops and GPU polling all check cancellation.
A valid cancelled result retains finished batches. Summed worker time is used
for render/resample durations, so those figures can exceed total wall time.
Each run saves its adapter, dispatches, measured calibration and worker counts.

Adapter choice is explicit: every hardware adapter is enumerated and tried in
order discrete → integrated → virtual → other, Vulkan before DX12 within a
type, skipping software adapters and those without 32 KiB workgroup memory. An
integrated GPU is used only when no discrete adapter can open a device; when all
fail, the error lists each adapter's reason. On the development laptop (RTX 3050
plus Ryzen iGPU) the fitter always ran on the RTX 3050; Auto still chooses CPU
there because the measured batched FFT is ~0.45 ms on GPU versus ~0.19 ms on
the CPU workers. The editor window itself renders through OpenGL (glow), which
Windows may place on the integrated GPU independently of the fitter.

Backend validation layers stay off even in debug builds (wgpu enables them there
by default). The Khronos Vulkan validation layer from SDK 1.4.357 crashed debug
GUIs with an access violation inside its own allocator during Auto calibration
(2026-10-22). wgpu-core's API validation still runs; set `WGPU_VALIDATION=1`
(and optionally `WGPU_DEBUG=1`) to load the layers on purpose.

## SteelDrum validation

The local reference is FL Studio's `PERC_SteelDrum_C5.wav`, 44100 Hz, 29696
frames (0.673378685 s). It contains strong inharmonic partials around 528, 1049
and 2099 Hz with different decay rates. Copyrighted reference audio is not
included. `tests/steeldrum_benchmark.rs` compares with the saved v3 patch using
independent full/low-band spectral cosine, 40 ms RMS-envelope error and dominant
frequency; it does not compare objective scores across versions.

Measured release results on Windows x64, 16 logical CPUs and the NVIDIA
GeForce RTX 3050 Laptop GPU through Vulkan, with seed 2612, default patch,
256 queries/population 16, DC removal, normalization, silence trim and note-on
0/key-off at the complete note end:

| Patch/run | Full spectral cosine ↑ | Low-band cosine ↑ | Envelope error ↓ | Dominant peak (Hz) |
|---|---:|---:|---:|---:|
| Previous v3, 256 queries | 0.141525 | 0.874980 | 0.169901 | 1054.191 |
| v4, CPU and GPU, 256 queries | 0.812417 | 0.880505 | 0.137406 | 1050.124 |
| Additional 1024-query refinement | 0.788011 | 0.914823 | 0.126174 | 1050.364 |
| Reference | 1 | 1 | 0 | 1049.001 |

Opening the ready [SteelDrum session](../examples/steeldrum-fitted-session.json)
loads the 256-query patch for its stronger full-spectrum match; both runs remain
available for comparison. An additional refinement took 80.94 s, reducing v4
loss 0.235287→0.210128. The envelope/low band improved at the expense of
full-spectrum cosine, illustrating that lower weighted loss need not improve
every independent metric. The example uses Auto with four workers for future
searches.

For the 256-query patch, full-spectrum cosine distance decreased by 78.1%, and
envelope error by 19.1%.
These independent metrics use actual rendered audio, not fitting scores. CPU
1/4/8 workers produced identical candidates, history and cache counts. GPU and
Auto also selected the same patch here; their loss differed by about 9e-9.
The winner uses algorithm 7, channel 3 special, LFO rate 7/FMS 2, AMS 0, SSG off.
AM bits are available but AMS 0 contributes no tremolo. Hardware selection is
based on matching the signal and does not assert how the original was recorded.

| Execution | Total (s) | Renders | Cache hits | CPU calibration (ms) | GPU calibration (ms) | GPU dispatches |
|---|---:|---:|---:|---:|---:|---:|
| cpu1 | 39.16 | 231 | 25 | 0.000 | — | 0 |
| cpu4 | 16.77 | 231 | 25 | 0.000 | — | 0 |
| cpu8 | 20.08 | 231 | 25 | 0.000 | — | 0 |
| gpu4 | 16.49 | 231 | 25 | 3.226 | 0.599075 | 240 |
| auto4 | 17.03 | 231 | 25 | 2.582 | 0.593075 | 240 |
| gpu8 | 20.61 | 231 | 25 | 6.144 | 0.600375 | 240 |
| auto8 | 22.12 | 231 | 25 | 6.449 | 0.64465 | 240 |

Calibration entries are medians of three batched spectral measurements, including
transfers and readback. Total times are one sequential release run per case,
including preparation/calibration, rather than a repeated wall-time benchmark.
The v4 serial/4-worker ratio was 2.33×. Four workers were faster than eight here;
worker count and system load matter. GPU4's spectral work was about 5.4× faster,
but CPU4/GPU4 total times were effectively close (16.77/16.49 s); synthesis remains
the main cost. Auto chose GPU for the faster spectral stage, not from a benchmark
of the complete fitting run. There is no claim of a large GPU-only overall gain.

The earlier v3 serial run required 23.64 s and 205 renders/51 cache hits. Because
its objective, proposals and workload differ from v4, these figures document
the historical workflow rather than isolating implementation speed. No
percentage comparison is made between version 4 and version 3 scores.

The existing installed Bass regression also passed unchanged at 512 queries:
full cosine 0.884830→0.930449 and envelope error 0.086221→0.079748 versus its
saved M7 fixture. The historical M8 Bass patch remains available and achieved
0.935390/0.076371; a fixed budget spread over more modes does not improve every
metric on every previous sample. Existing good patches remain useful refinement
seeds. No copyrighted reference audio is distributed.

Reproduction from PowerShell (the output file must not exist):

```powershell
$steel = 'C:\Program Files\Image-Line\FL Studio 2024\Data\Patches\Packs\Legacy\Instruments\Percs\PERC_SteelDrum_C5.wav'
.\target\release\discofm.exe fit --reference $steel --evaluations 256 --population 16 --normalize --end 0.6733786848072563 --key-off 0.6733786848072563 --compute cpu --workers 4 --output target/steeldrum-cpu4.json
# Open the prepared example (keeps the WAV in its original installation):
.\target\release\discofm.exe gui --session examples/steeldrum-fitted-session.json
$env:DISCOFM_TEST_STEELDRUM_WAV = $steel
cargo test -p discofm-fit --test steeldrum_benchmark -- --ignored --nocapture
cargo test -p discofm-fit --lib gpu_ -- --ignored --nocapture
```

Verification: `cargo test --workspace` passed 123 automatic tests; fmt and
all-target clippy with warnings denied passed. The installed SteelDrum/Bass
opt-in checks retained their existing criteria, and both physical GPU numerical
tests passed. The final release completed forced-GPU and Auto fitting, session
validation and independent comparisons. Exact native pin hashes remain
`e5354d82d454959d`/`8dc66ae06d69b2dd`; ymfm validation also passed. No new
physical audio recording or CPAL loopback was used for these fitting changes.

Measured matching improves through this finite search, without guaranteeing
original registers or perceptual identity. Comparison with a recorded physical
chip is still pending. Independent ymfm and pristine Nuked checks serve as
numerical references.
