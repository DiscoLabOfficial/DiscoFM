# Hardware accuracy

## Backend and evidence boundary

DiscoFM builds from pinned [Nuked-OPN2](https://github.com/nukeykt/Nuked-OPN2/tree/335747d78cb0abbc3b55b004e62dad9763140115),
revision `335747d78cb0abbc3b55b004e62dad9763140115`. Its integer phase, envelope,
feedback, algorithm, LFO, SSG-EG, channel-3 and DAC implementations generate all
sound. Rust only encodes writes, schedules time and handles output conversion.
There is no generic FM substitute or post-mix ladder effect. Vendored sources
are retained unchanged. The build generates a separate production copy with
instance-local profile state and separate symbols; the original core is compiled
independently for pin comparisons. See [third-party notices](../THIRD_PARTY_NOTICES.md).

Current evidence shows deterministic integration agreement with this core,
without establishing perfect physical YM2612 accuracy: neither a physical chip
nor capture equipment was available. A pinned, unmodified copy of
[ymfm](https://github.com/aaronsgiles/ymfm) provides independent numerical comparisons
of both profiles. Spectra, envelopes and waveforms are compared at sample
boundaries; full pipeline/cycle accuracy is outside ymfm's target. Production
audio continues to come from Nuked-OPN2. Independent validation is described below.

## Clock and bus timing

Nominal NTSC oscillator is 53,693,175 Hz; PAL is 53,203,424 Hz. The chip master
clock is oscillator/7. Each Nuked clock call advances six chip master clocks;
24 calls complete an operator frame (master/144). Native rates are therefore
53,267.038690 Hz NTSC and 52,781.174603 Hz PAL, not 44.1 or 48 kHz.
The [Sega hardware manual](https://consolemods.org/wiki/images/e/e9/32X_Hardware_Manual_1994_Sega_text.pdf#page=54)
and [Nuke's console integration](https://github.com/nukeykt/Genesis-Plus-GX/blob/master/core/sound/sound.c)
provide clock references. These are nominal values, not measurements of a console.

After an explicit reset, initialization keys off all six channels, clears their
panning and disables the PCM DAC. It then loads the patch, writes frequency high
before low and keys on the selected channel. A register transaction sends its
address, waits two ticks, sends data and waits 46 more ticks before the next
transaction; BUSY tests check this conservative spacing. Capture excludes
initialization and starts at the key-on address write, retaining hardware
write/pipeline latency. Key-off data uses the same relative scheduling offset
as key-on data.

For DT=0/MUL=1, the integer phase increment is `(FNUM << BLOCK) >> 1`, and pitch
is `increment * native_rate / 2^20`. BLOCK=0 truncation is respected. MUL=0's
half multiplier and detuning are handled inside the core. Pitch tests allow the
hardware's finite frequency resolution; they do not assert arbitrary exact Hz.

## DAC and output profiles

`YM2612_DISCRETE` is the default. Upstream shifts operator output, clamps the
channel accumulator to -256..255, and models time-multiplexed, polarity-dependent
output gating. Its sign transition offset and residual output produce the ladder
characteristics directly. The relevant source comment says **“not verified”**:
this is an inherited DAC approximation, not calibrated analog hardware output.
The separate channel-six data interface is 8-bit PCM; it is tested diagnostically
and is never used to turn a reference sample into an FM instrument.

`YM3438` selects the CMOS output branch with different gating and without the
discrete ladder branch. `RAW_DIAGNOSTIC` returns an explicit unsupported-profile
error because upstream exposes no such mode. `render_trace` and
`direct_upstream_pins` capture exact stereo MOL/MOR values before integration.

Audio frames average all 24 pin ticks, then apply fixed gain 1/256. This gain is
an application convention, not measured console amplification. Upstream's
discrete ×3 is retained; applying another divide-by-three would misbalance the
profiles. Muted/reset discrete pins can contain +3 and sign-dependent residuals.
They are not forcibly zeroed. At sub-LSB levels, truncation can produce zero for
the positive half-wave and -1 for the negative half-wave in the CMOS profile.
Regressions preserve these behaviors rather than normalizing them away.

Native integration is followed by a deterministic windowed-sinc rate converter,
whose source support grows with the downsampling ratio. No external console
filter, analog noise, dither, saturation or bitcrusher is applied. Float32 WAV
output retains gain and DC, while native pin diagnostics skip resampling entirely.

## Synchronization and validation

Upstream `chip_type` is global mutable state. The generated production copy moves
it into each chip instance, allowing independent workers to render concurrently.
Only the separately compiled, unmodified C oracle uses a process-wide Rust mutex.
Native chip pointers remain owned and neither Send nor Sync; each worker creates
its own instance. Trace rendering advances in batches of at most 4096 ticks.
CPAL playback buffers rendering on a worker outside the device callback.

The separate C oracle calls upstream directly, without the Rust allocation or
chunk driver. Tests compare every captured pin, exercise all six channels/eight
algorithms and SSG encodings, verify pitch/release/stereo, and protect fixed
default-note hashes. Baseline command `cargo run -- validate` on 2026-10-10:

| Profile | Captured ticks | Mismatches | RMS error | FNV-1a pin hash |
|---|---:|---:|---:|---|
| YM2612 discrete | 127848 | 0 | 0 | `e5354d82d454959d` |
| YM3438 | 127848 | 0 | 0 | `8dc66ae06d69b2dd` |

Only emulator integration is established by these pin results. The independent
emulator audio comparison is described below. Physical ladder
calibration, console filtering and THD/spectral measurements against physical
recordings are still unverified.

## Register and envelope validation

The core's actual operator registers, envelope stage, raw/effective attenuation
and phase increment are exposed by read-only diagnostics, without synthetic
ADSR traces. Tests verify that reading diagnostics leaves every captured output
pin intact.

Acceptance additionally exercises all 32 algorithm/carrier combinations; causal
modulation paths distinguishing S2/S3; attack, decay, sustain and release;
frequency-dependent key scaling; DT sign and negative zero; all multipliers;
SSG initial direction, repeating/held endings and key-off; LFO sensitivity/AM
gating and fast/slow rate ordering on both clocks. Each channel-3 operator's
phase increment and measured isolated pitch matches its assigned frequency.
These are behavioral checks of the integrated core, not physical measurements.

## Playback validation

The live renderer reuses exactly the offline patch initialization and key-on
schedule. Integrated native frames match offline output across both profiles,
both clocks and six channels, including special-channel registers and release
before zero/one rendered frame. Streaming resampling matches offline samples
and frame counts at 8/44.1/48/96/192 kHz. These comparisons concern native frames
and converted samples; the independent direct-driver pin regressions above
remain unchanged.

Gain and residual DC survive held/released hardware rendering. Transport Stop
deliberately produces digital silence and invalidates queued note frames. Only
the final device conversion boundary clamps samples to [-1,1]; offline WAV/pin
diagnostics are unchanged. Changing profile restarts a held note for deterministic
comparison rather than splicing tails between output profiles.

Through CPAL output and WASAPI loopback, the opt-in Windows test checks short
generated profile tones, octave selection, release decay and Stop. Its in-memory
capture includes the system mix, requiring other applications to be quiet.
The result verifies OS playback rather than physical speakers or YM2612 hardware.
Nonfatal CPAL notifications are permitted and reported; fatal capture/playback
errors fail the test.

## Editor and reference analysis

Editor envelopes are samples of actual `OperatorSnapshot` raw/effective
attenuation, not ADSR approximations. `render_diagnostics` preserves exact offline
PCM and bus timing across both profiles/clocks, including key-off between frame
boundaries. Snapshot decimation limits plot resolution; very fast EG/SSG changes
can occur between displayed points. Existing native pin hashes remain unchanged.

Reference preparation and feature extraction preserve original PCM and native
chip gain. A/B playback uses separate matched buffers.
Monophonic pitch, Hann-window spectral bins and transient
heuristics are measurements with explicit limits (REFERENCE_ANALYSIS.md), not
new evidence of physical-chip accuracy or inverse-fitting correctness.
Finite A/B transport tests match complete offline sinc output and true endpoints.
The opt-in CPAL clip test verifies consumed frame counts, EOF, switch and Stop;
it does not capture or measure acoustic output.

## Fitting validation

All inverse-synthesis candidates use the Nuked-OPN2 engine with a cancellable
no-snapshot path that preserves offline PCM and bus timing exactly. Tests recover
levels and improve envelope settings on chip-rendered targets, exercise all
algorithm/stage choices and preserve locked registers on every proposal.
Multi-resolution spectral/temporal scores rank candidates; they do not add evidence
of physical hardware accuracy, recover a unique patch or validate every possible
SSG/LFO reconstruction. The existing exact pin regressions remain unchanged.

Native synthesis stays separate from decoded reference files. Expanding WAV
codec/rate/duration support leaves backend and pin integration unchanged. Peaks
above full scale can survive Vorbis decoding and are clipped only during final
device conversion.

## Independent validation

The pinned ymfm core supplies an independent sample-level reference.
Fresh concrete YM2612/YM3438 instances, preserved fractional clocks, ordered bus
writes quantized by less than one frame, and fixed DAC-duty scale are documented
in PRECISION_PERFORMANCE.md. Carrier tests cover both profiles/clocks/banks;
advanced fixtures cover all algorithms, envelopes, detune, SSG and LFO. Exact
Nuked pin hashes remain unchanged. ymfm can screen fitting proposals, while
audition and every retained fitting score use Nuked-OPN2.

Recorded WAVs can also be supplied to `compare`, with an explicit start, bounded
alignment and optional whole-note gain. No physical recording has been provided.
Agreement between emulators adds behavioral evidence without proving the
uncalibrated analog ladder or all hardware behavior. GPU compute affects only
analysis, leaving chip synthesis, register timing and DAC integration unchanged.
