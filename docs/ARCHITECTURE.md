# Architecture

Argument parsing and command execution are delegated by the root `discofm`
binary to a Cargo workspace. A module is introduced once it has working
behavior; empty UI/analysis/optimizer crates are not counted as implemented
features.

| Crate | Current responsibility |
|---|---|
| discofm-chip | Validated values, explicit operator identities/algorithm graphs/SSG bits, register encoding |
| discofm-engine | Replaceable cycle backend, opaque Nuked FFI, schedules, offline/live native rendering and read-only hardware snapshots |
| discofm-audio | Offline/streaming sinc resampling, CPAL devices, bounded playback transport and float32 WAV output |
| discofm-reference | Direct-upstream pin comparisons and regression measurements |
| discofm-soundfont | Lazy SF2/SF3/SFZ/DLS catalogs, sample extraction, source identity and shared reference resolver |
| discofm-formats | Version 1 internal JSON sessions and TFI/VGI/TYI instrument import/export |
| discofm-analysis | Broad WAV decoding, bounded-region preparation, YIN, FFT/STFT, harmonics and temporal features |
| discofm-fit | Typed parameter search, shared loss, locks, ranked candidates, history and cancellable worker |
| discofm-ui | egui controls/plots, cancellable preview and file worker, A/B and live transport |
| discofm-cli | Desktop launch, fitting, render/session/inspection/validation, devices and keyboard/stdin auditioning |

## Rendering

`Instrument + RenderRequest -> validated register schedule -> reset chip ->
timestamped port writes -> exact stereo pins -> 24-tick integration -> native
stereo audio -> sinc resampling -> diagnostic WAV`.

Reset, port write, clock-into-buffer and status form the `ChipBackend` contract,
currently implemented by `NukedChip`. Register models and note scheduling can
remain in place when the core is replaced. Rust owns allocation and lifetime
through an opaque pointer rather than mirroring the C struct. Unsafe Rust is
confined to `discofm-engine/src/native.rs` and forbidden in the other crates.

`NukedChip::snapshot(Channel)` reads a bounded 82-word C-shim view without clocking or modifying the core. `ChannelSnapshot` exposes decoded
registers plus real envelope attenuation/stage and phase state, in S1/S2/S3/S4
order. These diagnostics are specific to the current backend; the portable
`ChipBackend` contract remains synthesis-only. `inspect_instrument` uses the
normal setup schedule and returns the loaded state before key-on. Pin-stream
regressions verify that interleaved snapshots leave exact output unchanged.

Build-time adapted copies of the pinned Nuked source move its three global
profile reads into an instance field and use separate symbols. Independent
production chips therefore render concurrently. The pristine upstream oracle
remains separately compiled and serialized; exact pins and hashes verify the
adaptation. Vendor files remain unchanged. Full native pin output is retained for current
offline rendering, with a 30-second note limit and a 40-million-tick trace limit.
This favors inspectability over optimizer throughput. `LiveRenderer` retains the
chip and absolute bus clock while integrating complete 24-tick frames in bounded
128-frame blocks. It reuses the offline setup schedule and has no note-duration
limit or growing pin buffer. Key-off retains phase/envelopes and enforces the
minimum 48-tick register spacing, even before the first audio frame. Exact native
frame regressions compare offline/live across irregular blocks, both clocks,
profiles and all channels. A fresh trigger resets the voice deterministically.

## Real-time playback

`AuditionControls -> bounded trigger queue + atomic release/Stop -> synthesis
worker (LiveRenderer -> StreamingResampler) -> bounded tagged PCM ring -> CPAL
callback -> device`.

The worker creates and owns the non-Send native chip. Native synthesis,
allocation, setup/reload and resampling never enter the application audio
callback. CPAL 0.18.2 manages the output stream; rtrb 0.4 provides the single
producer/consumer rings. Only the control handle publishes triggers (capacity 32).
Release and Stop are atomic, so a full trigger queue cannot drop them. Voice IDs
ignore delayed key-up from older notes. Generations invalidate old PCM on a
trigger, Stop, profile change or patch reload. Stale draining is bounded by one
ring capacity per callback, including when producers publish concurrently.

Each callback starts by filling samples with equilibrium silence. It then reads
prepared frames, converts float/signed/unsigned device formats, averages L/R
for mono and zeros extra channels. Clamping to [-1,1] occurs only at the physical
device boundary; offline diagnostics retain their original gain and DC.
Underruns leave silence and increment atomic counters. On a fatal CPAL error,
output is atomically muted and the control thread is notified.
DeviceChanged/Xrun/RealtimeDenied are counted as nonfatal notifications.
Reopening the stream requires an explicit action. Dropping Playback mutes,
cancels the worker, drops CPAL and joins without requiring the device to drain
PCM before cancellation.

Device enumeration exposes CPAL IDs and default configurations. Playback prefers
the default mix format, or selects a supported format at an explicitly requested
8–192 kHz rate. It supports 1–32 output channels with the mapping above and uses
the backend's default buffer size. The prepared PCM ring defaults to 40 ms and
can be set to 10–100 ms. Additional lead comprises at most 128 native worker
frames, sinc lookahead and device buffering. Stop cannot retract frames already
submitted to the device. Timing is interactive, with no hard real-time guarantee.

The offline kernel also serves streaming sinc conversion. Absolute output
positions and bounded filter history/lookahead carry across blocks. Only a true
stream finish extends the endpoint: after release, playback continues rendering
the chip until Stop or another trigger. Converter history resets on a generation
change. On both chip clocks, tests compare all samples and counts at
8/44.1/48/96/192 kHz.

CLI keyboard events use crossterm 0.29. Windows has native release events; capable
Unix terminals request all key codes and event types through keyboard enhancement.
Other terminals report manual Space/Esc release. The stdin command mode isolates
blocking reads on a thread so the control loop can still poll stream health.
Session reload happens on the control thread and validates before publication.

## Sample-bank references

The existing analysis/fitting/A/B pipeline receives
`SoundBank::open -> sample catalog -> SampleSelection -> AudioBuffer`. Separate
RIFF and SFZ readers reside in `discofm-soundfont`, where PCM extraction never
invokes instrument synthesis. Analysis supplies cancellable, mono-checked
Vorbis decoding for SF3; DLS waves use the existing WAV codec handling. The
catalog indexes metadata first, then decodes only the chosen sample and a valid
stereo mate. A bounded file worker handles catalog/selection jobs. Its
synchronized request ID is published after queue acceptance, and opening a
session cancels earlier imports. Bank sample identity is optional session data;
cached banks check source metadata again before reading.
See [soundfont scope and commands](SOUNDFONTS.md).

## State and export

The UI thread edits egui state. A separate worker receives its bounded file-job
queue and one replaceable preview mailbox. Every edit immediately updates an
atomic revision, while rendering and analysis test cancellation between bounded
units of work. The UI accepts only results matching its current revision. Drag
events are debounced for 150 ms before rendering. Reference analysis is cached
by immutable buffer identity and settings, allowing patch edits to reuse it.
For each revision the worker also derives every plot series (EG traces,
waveforms, envelopes, spectra), which frames then borrow. Texture uploads and
painting stay on the UI thread. Repaint runs at about 30 Hz during work or
playback and 4 Hz while idle. The first audition opens an output device lazily.

UI modules: `theme` (palette and shared widgets), `help` (all tooltip texts),
`toolbar` (header, audition strip, piano), `controls` (channel and operator
cards), `reference` / `banks` (reference panel), `compare_view` (A/B card),
`fitting` (fitting view) and `plots` (charts with hover read-outs, spectrograms,
algorithm diagrams).

`render_diagnostics` replays the exact offline schedule, integrates 128-frame
blocks and samples the actual operator state at native frame boundaries. Its
bounded traces preserve exact offline PCM, including non-frame-aligned key-off.
Preview conversion uses the same streaming sinc kernel and checks cancellation.
No synthetic ADSR or alternate synthesis path is used for editor curves.

The live generation-tagged PCM ring also carries finite A/B clips. Across active
and queued leases, immutable clips retain at most 128 MiB, counting vector
capacity. True endpoint extension finishes conversion on the worker. Completion
is recorded only after the callback consumes the tagged last sample or a
zero-output end marker. At normal EOF, silence is written without adding
underruns. Old clips are invalidated by Stop and switches in the same way as
old live notes.

Hardware parameter types validate both constructors and serde input. Sessions
separate the instrument from audition pitch/profile/clock/duration and optional
reference path. Unknown fields, invalid values, unsupported versions and sessions
over 1 MiB are rejected. The writer applies the same byte limit as the reader; it
writes indented JSON when that fits and compact JSON otherwise.
Private wire data and a custom `Session` deserializer enforce version, audition
and special-channel constraints even through direct serde use. Reading reports
nested value errors with `serde_path_to_error`; trailing JSON is rejected.
Validation checks pitch representability without constructing a render buffer.
Version 1 numeric encodings and optional-field behavior remain compatible.
The optional `reference_analysis` object stores validated preparation settings and
manual note-on/key-off markers; old sessions omit it. Relative reference paths
resolve against the session directory. GUI Save As validates/serializes before
writing an exclusively created sibling temporary file and replacing the selected
JSON destination. It protects the reference source and foreign temporary files.
CLI/WAV outputs require new files; canonical name checks also reject colliding
WAV/trace paths and attempts to overwrite the source session. Exclusive creation
protects existing hard links and symlinks as well.

`InstrumentExporter` is implemented by `InstrumentFormat` (`discofm-formats/src/export.rs`)
for the tracker formats GENNY writes: TFI (42 bytes), VGI (43) and TYI (32, with a
"YI" trailer). All three store raw register values with operators in register
order S1, S3, S2, S4; `omitted` names what a format cannot hold (AM/AMS/FMS in
TFI, panning in TFI/VGI, the global LFO, channel 3 special frequencies and
velocity depth in all three). Decoding validates every field and fills absent
ones with editor defaults. Diagnostic register traces and WAVs are not
instrument exports.

The editor keeps an instrument undo stack (`discofm-ui/src/undo.rs`): a step opens
on the first frame that changes the instrument and closes when no pointer button
is down and no widget has focus, so a drag is one step. Replacements made outside
the widgets (paste, import, library, undo itself) go through
`Editor::replace_instrument`/`instrument_replaced`, which also retunes a held live
note. The unsaved-changes check compares the session with the copy last opened or
saved; library listing, patch loading and export writes are service jobs.
MIDI input (`midi.rs`, midir) forwards note messages from the driver thread over
a channel and wakes the UI, which owns the audition voice as for the piano.
`discofm-gui.exe` (`src/bin/discofm-gui.rs`) is the same editor under the Windows
GUI subsystem: no console window, startup errors in a message box, crashes still
supervised and logged.

## Fitting and reference pipelines

`discofm-analysis` now loads monophonic-reference WAVs and extracts pitch, STFT,
harmonics, envelopes and transients. Preparation produces a separate mono buffer;
the original stereo PCM remains immutable. The UI comparison worker prepares A/B
with the same source interval, channel, rate, DC setting and shared gain; one
optional B gain serves every plot and clip. Common playback attenuation prevents
clipping. See REFERENCE_ANALYSIS.md.
`discofm-fit` evaluates all candidates through cancellable native
rendering, with the same bus schedule as offline rendering and no snapshots or
pin trace allocation. A fixed SplitMix64 generator drives frozen-generation
DE/rand/1/bin, integer projection and local discrete refinement. Stage-specific
dimensions preserve typed hardware constraints, locks and fixed settings.
Complete candidate bases retain normal/special algorithm families through later
stages. Advanced Auto/Preserve policies separately control LFO/FM, AM/AMS and
channel 3 topology. Grouped proposals cross inactive gates; explicit locks take
priority. Pitch/partial trajectories and phase-tolerant log-RMS periodicity reuse
existing features. Ordered batches render/resample on up to 16 independent CPU
workers without changing CPU search decisions.

Reference preparation is done once per run, at 24 kHz. Shared reference gain,
true onset/key-off and pre-roll preserve level/timing meaning. FFT plans and
scratch, reference logarithms/energy and sinc phase tables are reused; a bounded
register-key loss cache up to the evaluation budget (max 20000) avoids duplicate
renders. Progress is coalesced in a mutex mailbox with one terminal result; UI
polling does not block on a queue. Cancellation checks occur during preparation,
native blocks, resampling and FFT windows/GPU readback polling. Drop cancels and
detaches an unfinished worker so driver initialization cannot block the GUI.
Candidate history contains no PCM. See FITTING.md for the objective and bounds.

Fitting settings, the worker and the selected run/candidate belong to the UI.
Candidates are applied only by explicit action. Audition waits for a matching
new preview revision; Stop, release, trigger, A/B or focus loss clears a pending
audition. Editing context cancels an active run, and replacing the session
discards its result. The file worker performs explicit reference analysis/apply
and tags results by revision, independently of chip preview success. Bounded
analysis regions impose no WAV admission limit.

Optional v1 `fitting` stores validated config, locks and up to eight runs. Readers
verify component totals, timings, ranking, uniqueness, fixed fields, locks and
monotonic history. The 1 MiB session byte limit remains. Candidate prediction and
dataset generation remain optional future enhancements.

Optional wgpu compute assigns a slot to each worker for radix-2 STFT and
spectral-distance reductions. Reference spectra/logs are shared and reusable
buffers stay resident; readback returns reductions plus the largest spectrum
for CPU features. Auto measures concurrent CPU/GPU spectral paths including
transfers and selects GPU only for a median improvement of at least 10%.
Synthesis and resampling run on independent CPU instances. The current
objective is v6: v4 introduced pitch, partial and periodicity terms; v5 added
band energies and unpitched mode; v6 added off-grid energy. Archived v1–v5
scores preserve their weights and values.
Loss v3 introduced one global recording gain, two temporal RMS resolutions and
continuous cents through hardware register quantization. Automatic YIN pitch
receives a bounded spectral refinement. Archived v1/v2 scores remain readable
and distinct.

Unmodified pinned ymfm C++14 is also compiled by the engine behind a separate C
ABI in the sole Rust FFI module. The reference crate uses fixed DAC-duty
calibration to compare sample-level output. It can instead compare independent
recorded WAVs with explicitly bounded alignment and optional gain. Fitting can
use ymfm to screen proposals; retained scores and playback use Nuked-OPN2.
