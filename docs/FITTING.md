# Fitting — automatic modes and parallel evaluation

The GUI and `fit` CLI search Nuked-OPN2 renders. In Basic mode,
the search varies the eight algorithms, feedback, MUL, TL and raw detune.
Advanced mode also varies AR/D1R/D2R/SL/RR/RS, every valid SSG-EG encoding,
operator AM, LFO enable/rate and AMS/FMS. Normal/channel 3 special mode, LFO/FM
and operator AM/AMS are compared automatically in new advanced runs, each under
an independent Auto/Preserve policy. Locks override those policies; Basic keeps
the controls unchanged. Each run holds stereo routing, software velocity
response and render profile/clock/velocity fixed.

Auto channel 3 evaluates on hardware channel 3 and seeds independent S1–S3
FNUM/BLOCK values from tracked partials, accounting for MUL. S4 retains the note
frequency. Special frequencies are absolute registers and do not automatically
transpose across the keyboard. Seeds assign the strongest remaining reference
partials to actual carriers and reserve the note-frequency partial for S4 when
it is available. Muted, locked carriers consume no partial. Relative TL starts
preserve reference peak balance; level-matched runs use available chip headroom.
Locking a special frequency freezes topology.
Legacy JSON without a channel 3 policy defaults to Preserve; the old
`fit_special_frequencies` option remains valid for an already special patch.

Acceptance checks CPU decisions for determinism at fixed input/seed, coverage of
every algorithm, actual improvement on chip-generated references and exact
locks on each proposal. It also covers all advanced stages, cancellation, valid
ranked history, refinement and session round trips. Verification combines
actual chip renders, a separate controlled objective for proposal coverage,
CLI runs and headless egui tests.

## Search

Three distinct donors other than the target and one forced mutant coordinate
define DE/rand/1/bin. Each generation reads a frozen population and projects
latent floating coordinates onto valid discrete fields. SplitMix64 is fixed
rather than chosen by thread or platform RNG. SSG has nine encoded choices:
disabled and raw 8–15. Unless the algorithm is locked or a selected candidate is
explicitly being refined, coarse algorithm enumeration seeds the search. Later
populations retain each algorithm's best normal and special families (up to 16
total), complete ranked patches and external refinement seeds. Seeding from old
candidates reapplies locks.

Basic budgets split 70% coarse / 30% local; advanced splits 30% coarse, 25%
envelopes, 25% modulation, 20% local. Structured ratio/envelope seeds supplement
the eight algorithms, respecting locks and Basic's fixed envelopes. Timbre seeds
add hardware idioms that random DE rarely assembles: additive odd (MUL 1/3/5/7,
square-like) and full (1/2/3/4, saw-like) series on algorithm 7 with 1/n carrier
levels, single-operator feedback 6/7 buzz, and feedback-7 high-MUL modulators
(algorithms 4–6) for the chaotic noise behind FM snares and hats. Each shape has a
sustained and a percussive envelope. Seeds are admitted only after reserving
four DE generations of the coarse budget: at 256 queries none enter (three of
them regressed SteelDrum by winning early under crude envelopes), at the default
512 all fourteen do. Buzz companions stay quiet rather than muted, because fully
silent operators make later mutations inaudible. Refinement opens with a ratio sweep of the
leader: distant MUL values (0.5, 1, 2, 3, 5, 7, 10, 15) per audible operator and
feedback 0/3/5/6/7, skipping values that adjacent coordinate steps already test. Advanced
local steps start at moderate discrete offsets, then adjacent and larger moves,
prioritizing carrier levels and D1R/D2R/SL before attack/release. Adjacent EG
rates are tested early because these registers change decay speed nonlinearly.
RR is excluded when the scored capture ends at key-off. Coherent carrier TL
shifts preserve partial balance while avoiding excessive amplification of the
YM2612 DAC floor by recording-gain compensation. Carrier removal and shared
envelope moves are also tested for the leading normal and special families,
with a reserved share for each topology. Local refinement reserves up to 24
queries after independent moves for a coherent D1R/SL/D2R/AR/RR pass and
headroom recovery when all audible carriers are quiet. RR is considered only
when release is observable. Any unspent allowance returns to local coordinates. Every stage uses
the same complete objective, so best candidates never regress across stages.
Mode proposals cross inactive gates together: off, vibrato, tremolo, both, and
AM on modulators for periodic timbre. All eight LFO rates are proposed; joint
AMS/FMS enumeration and DE/local moves refine sensitivity and AM masks.
SSG proposals cover disabled and all eight valid enabled shapes. Automatic
topology is also explored during refinement.
### ymfm screening

A note renders about ten times faster in ymfm than in Nuked-OPN2. On 240
perturbations of two chip-generated targets (an FB5 tone and an FB7 snare), the
fitter's loss ranked ymfm renders almost identically to Nuked renders: Spearman
0.998 and 0.995, all of Nuked's best tenth within ymfm's best third, and Nuked's
best at ymfm rank 1 and 2 (`ymfm_screening_rank_agreement` in `loss.rs`). This
agreement lets `screening` choose which patches the chip renders. Every score
recorded, ranked or archived still comes from Nuked.

- `screening = 1` (default): local coordinate refinement scores the leader and
  both directions of the next `2 * workers` moves through ymfm and skips a move
  predicted worse both ways. Scores are cached by register key, because at a
  local optimum sweeps repeat identical moves. A sweep that would render nothing
  runs once unscreened, so the stage never stalls.
- `screening = 2..16`: DE also proposes that many trials per member and renders
  the best-screened one.
- `0`: off; runs archived before screening lack the field and read as 0.

Measured on the five chip-generated hard targets with CPU, seeds paired
(`screening_seed_spread` in `tests/hard_targets.rs`):

| Setting | Budget | Result vs off | Wall time |
|---|---|---|---|
| refinement (1), 6 seeds | 512 | better in 23 of 23 non-tied pairs; medians −6.5% snare, −9.4% kick, −15.4% tom, −10.7% buzz | +88% before the cache |
| refinement (1), 6 seeds | 1000 | better in 19 of 19 non-tied pairs; medians −12.1% snare, −21.5% kick, −3.5% tom | +43% with the cache |
| DE trials (4), 4 seeds | 2000 | 7 better, 5 worse, 8 ties; −8.4% kick, +2.5% snare | +48% |

Spending equal wall time on refinement screening or extra unscreened evaluations
gives roughly comparable results. At equal evaluation budgets, refinement
screening is consistently better. It is therefore the default, while DE trials
are not.

### Differential evolution parameters

`adaptive = true` switches DE to jDE (Brest et al. 2006): each member carries its
own F and CR, resampled with probability 0.1 (F in 0.1..1) and kept only when its
trial wins. It is off by default. On the same targets jDE matched or slightly
beat fixed F 0.7/CR 0.8 at 512 evaluations (kick −6%, tom −1.4%), but at 2000
fixed values won 15 of 20 seed pairs (medians +2.5% to +8.7% for jDE). The fixed
mode draws no extra random numbers, so runs archived before the option replay
unchanged.

Starting refinement re-evaluates the selected patch and seeds with the current
reference, profile, locks and weights. This creates a new run rather than
resuming a DE population bit for bit.

Config allows 24–20000 evaluations, population 4–64, ranked count 2–16,
differential weight 0.1–2 and crossover 0–1. Basic requires budget >=3*population;
advanced requires >=8*population and at least 96. Default is advanced, 512
evaluations, population 16, six candidates, seed 2612, max duration 8 seconds.
The maximum render window is configurable to 30 seconds, measured from actual
note-on, including pre-roll before a later scoring start. This does not limit WAV
opening or absolute source timestamps. Exhausting the finite budget reports
completion; it does not guarantee a globally optimal or unique original patch.

## Shared objective

Analysis/pitch preparation happens once. A manual pitch takes precedence over
the detected estimate, whose confidence must otherwise be >=0.5. The nearest
MIDI note and continuous fine tuning in cents are stored for synthesis, then
quantized through actual chip FNUM/BLOCK. Errors explicitly report silence,
uncertain pitch, invalid markers or oversize selected note spans. Manual note-on
and key-off markers use absolute source time. Trimming silence does not reset
a slow attack at an amplitude threshold; a later selection renders all pre-roll
from note-on and scores only its selected region.

Both signals use 24 kHz sinc conversion, selected Mix/Left/Right, DC preparation
and the same preparation gain measured from the reference. With `match_level`
enabled, a single bounded recording gain RMS(A)/RMS(B) scales the whole B only
for comparison. This is a nuisance recording parameter, never a per-window
normalization or change to chip gain. Disable it to retain absolute level errors.
Multi-resolution STFT uses periodic Hann windows 512/1024/4096,
hop N/4, centers covering the whole scored interval including endpoints, and
zero padding. Component losses are:

- Log magnitude error with a reference-derived floor and 60 dB scaling.
- Spectral convergence, Frobenius error/reference magnitude norm, capped at 100.
- Two RMS resolutions: two periods bounded to 10–40 ms, and a fixed 40 ms
  window, sampled every 5 ms and averaged for envelope/tail scoring;
  half the sum of relative L1 and L2 errors, plus 0.25 times log-RMS error
  with a −40 dB reference-RMS floor.
- Up to 32 normalized harmonic energies from the largest STFT resolution.
- RMS derivative transients, centroid/positive-flux modulation trajectories.
- Tail RMS/roughness from key-off or the final quarter, whichever starts earlier;
  an additional 0.5 times log-RMS error retains weak decay information even when
  the manual key-off is at the end of a one-shot note.

Version 5 adds `bands`: 24 log-spaced bands from 40 Hz to 11 kHz over the 4096
STFT, compared as mean absolute log energy with a floor 60 dB below the loudest
reference band (default weight 0.5). Summing bins averages the realization
variance of chaotic feedback noise, which per-bin STFT errors mostly measure.
It uses the largest spectrum, which the GPU path already reads back.

Version 6 adds `inharmonic` (GUI "Off-grid", default weight 0.5, 0 when
unpitched): the share of average 4096-STFT power from f0/4 to 11 kHz lying more
than max(f0/4, two bins) from every harmonic, scored one-sided as
`max(0, ln((candidate + 0.01) / (reference + 0.01)))`. Harmonic peaks, partial
tracks and pitch could not see a MUL ×½ patch an octave low: its even partials
sit on every reference harmonic. A BeepBox StringsL fit (v5, 6000/64) put 55% of
its energy at (k + ½)·f0 and still scored 0.318, against 0.263 for a clean patch
the search found only by chance; such patches sound like a square/buzz one octave
down. References that carry their own off-grid energy (bells, detuned layers)
allow the candidate the same share. Weights saved before v6 lack the field: it
counts as 0 when archived runs are revalidated, and `fit` resolves it to the
default for new runs, so reopened sessions get the term too.

Unpitched (percussion) mode is an optional config flag. It skips the YIN
confidence requirement: the chip plays the manual pitch, otherwise the body
note, as drum patches are keyed on hardware. The body note is the lowest peak of
the whole-note spectrum within 6 dB of the strongest one in 20..4000 Hz, refined
by the (low-confidence) YIN estimate when that lies within a semitone; a MUL 1
carrier then lands on the body and MUL 2..15 reach the brighter partials. With no
peak at all it falls back to the audition note. Feedback-7 noise has no clear
body: on a chip-generated FB7 snare keyed at 261.6 Hz it chose 225.7 Hz, so set a
manual pitch when the note matters (for example with most registers locked). Enabling it from the GUI checkbox or
`fit --unpitched true` switches to the `LossWeights::unpitched()` preset: pitch,
partials, periodicity and harmonic weights 0 (they would compare against an
arbitrary playback note) and bands 1; every other weight keeps its default.
A first preset that also lowered log-STFT/convergence let noise seeds win early
and stall on synthetic drum targets. Weights remain editable and are stored in
the session.
The chosen note shapes MUL/DT ratios; it is not a claim about the original note.
Pitch sweeps of kicks are usually driver FNUM writes over time, not patch
registers, and remain outside static patch fitting.

Additional version 4 components are pitch trajectories (weight 0.5), partial
frequency/amplitude trajectories (0.75), and detrended log-RMS periodicity (0.5).
Up to 12 reference partials include inharmonic peaks. Assignment is exclusive
within each frame and weighted by partial strength; disappearing audible frames
are penalized. Integrated relative partial energy penalizes inverted dominance,
even if every partial has the right frequency. Missing fundamentals receive
weaker harmonic support. Periodicity
is phase tolerant and contributes no evidence for very short or silent curves.
Pitch/partial tracking reuses the 4096 STFT, with 1024-sample hops at 24 kHz
(42.67 ms). Faster vibrato is primarily represented by spectral sidebands, rather
than a resolved 48/72 Hz pitch contour. Dense tracks remain internal to a run.

The seven spectral/temporal default weights in v4 are 1, 1, 2.5, 0.25, 0.35, 0.25
and 0.5; v5 keeps them and adds bands 0.5. Envelope error combines relative L2 convergence, linear L1 error and
log-RMS error so energetic attacks and quiet tails both contribute. L2 uses a
global reference-RMS floor, keeping nearly silent tail denominators bounded. The total is
their weighted mean. Zero-weight components are allowed; all-zero/negative/
nonfinite weights and inconsistent archived totals fail validation. These losses
are engineering heuristics, not a calibrated perceptual score or an SSG/LFO
classifier. Sparse/broadband or mismatched tuning references may remain poor fits.

Prepared samples/settings are identified for comparison by the FNV fingerprint,
which provides no cryptographic provenance guarantee. Loss version 1 archives include profile,
clock, note, trigger/scoring/end times, shared gain and evaluated frame count.
Version 2 additionally scores fine tuning, recording gain, bass-adaptive RMS and
log-envelope/one-shot tail protection.
Its fingerprint includes the effective FFT backend and level-matching setting.
Version 3 uses both adaptive and fixed 40 ms RMS resolutions for envelope/tail
scoring. Automatic pitch also receives bounded spectral refinement before fitting.
Version 4 adds tracking components, automatic modes and batched search.
Version 5 adds band energies, unpitched mode and timbre/ratio search moves.
Version 6 adds the one-sided off-grid (inharmonic) energy term.
Versions 1–5 remain readable with their original scores; absent new weights and
components default to zero. Version 1 defaults
to gain 1 and cents 0.
Do not directly compare scores produced by different objective versions.

## Worker, storage and controls

One coordinator publishes latest progress plus a terminal result without queue
backpressure. `workers=0` uses every logical CPU, up to 16; 1–16 can be
selected explicitly. The worker threads are a rayon pool created once per fit
(`in_place_scope` keeps the caller's cancel callback on the coordinator); spawning
threads for each of the coordinate stage's thousands of small batches cost
about 5% of wall time on a 2000-evaluation run. Workers pull candidates from a shared index and each one
renders on its own native chip, converts to 24 kHz and computes the complete
loss (spectra, envelopes, tracking) on its own thread with private FFT scratch;
there is no serial scoring stage that idles the other workers. A loss is a pure
function of the patch, so results are committed in proposal order and CPU
decisions are identical for any worker count. Proposal RNG, deduplication,
score commits and ranking remain ordered. The fitter downmixes once and runs a
single-channel sinc conversion that is bit-identical to channel 0 of the stereo
converter.

Sequential stages (coordinate refinement, coherent carrier moves) submit only
a few candidates at a time. Up to half the workers that such a batch would leave
idle score the stage's next planned proposals from the same leader instead
(every busy thread slows each render on the reference laptop, so a full batch of
guesses delayed the wanted scores whenever they were discarded). Those scores
are kept aside and used only if the search later proposes the identical patch;
they are never recorded, ranked or counted as evaluations, and they are
discarded when a move improves the leader. Speculation therefore changes wall
time, never a decision. Render/resample diagnostics sum worker durations,
including speculative renders, and may exceed wall time. Completed queries
include cache hits; render count only includes chip evaluations whose scores
the search used.

Chip synthesis dominates: on a 6 s note one Nuked-OPN2 render costs about
0.45 s alone on the reference laptop, against roughly 25 ms of scoring. Render
throughput is bounded by the CPU (shared clocks and SMT): 2.2 renders/s with one
thread, 8.0 with eight and 12.3 with sixteen. A DE generation can use at most
`population` workers at once. The bounded register-key loss cache retains
up to the evaluation budget (maximum 20000), not candidate audio. Reference FFT
layouts/plans, reference logarithms/energies and sinc phase tables are reused. No synthesis, FFT,
file I/O or fitting runs inside the CPAL callback.

Preparation loops, native 128-frame blocks, converter blocks and FFT windows/GPU
readback polling all check cancellation. Dropping a worker requests cancellation
without waiting for driver initialization. A cancellation after baseline returns
an archive with completed candidates; before baseline, it returns Cancelled
without an archive. Search settings expose separate Auto/Keep policies for
LFO/vibrato, AM/tremolo and channel 3, including automatic modes in a legacy
session. Lock all covers special-frequency identities even before the
selected candidate is applied. Only explicit Apply changes the editor.
Cancellation retains results from a finished batch and discards unfinished
speculative work. Committed queries determine progress, which keeps the actual
completed/budget values on cancel. Context edits cancel runs, session replacement
discards old pending results, and Stop/focus loss cancels delayed candidate
audition.

The session stores up to eight runs, sixteen unique ranked candidates per run and
at most 256 convergence points. When the curve fills, every other interior point
is dropped and the sampling stride doubles, keeping the curve evenly spaced and
the first point of every stage; dropping the oldest point instead used to leave
only the baseline and the last tenth of a long run. Baseline and final points are retained; readers verify
timings, locks/fixed fields, ranking and monotonic best loss. Sessions remain v1,
with an optional fitting object and the existing 1 MiB read/write limit.
Sessions are written indented when they fit; otherwise the writer falls back to
compact JSON, which the reader accepts unchanged. Eight maximal runs (16
candidates with special frequencies, 256 history points, longest diagnostics)
measure 816 KiB indented and 394 KiB compact, so they always fit (test
`eight_maximal_fitting_runs_fit_the_session_limit`).
CLI output requires a new JSON file and preserves reference/input sessions.
Escape/Ctrl+C cancel interactively; `--cancel-after N` requests cancellation after
observing at least N completed queries, with possible polling overshoot.

`compute` is CPU (default), Auto or GPU; Auto and GPU are experimental. Chip
synthesis is about 93% of an evaluation (render 186 s of 199 s summed over a
2000-evaluation run), so the GPU can only shorten the rest. Sessions saved before
GPU support lack the field and read as CPU, which is what they used. The CPU path
uses a real-input FFT (realfft) and windowed RMS from one prefix sum of squares.
With GPU each worker owns a slot (signal, output
and readback buffers) and submits its candidate through 512/1024/4096 FFTs and
log/squared spectral-distance reductions as soon as the render finishes, so
transfers overlap with other workers' renders instead of queueing behind one
lock. Reference spectra/logs and window layouts are uploaded once and shared by
all slots. Readback contains reduced distances and only the largest spectrum
needed for the remaining CPU features. Nuked-OPN2 synthesis, resampling and the
other features remain parallel CPU work, so the GPU can shorten only the small
spectral share of an evaluation; it cannot accelerate the chip emulation.

Auto warms the GPU, validates magnitudes against CPU, then measures three rounds
of one candidate per worker scored concurrently on each backend (GPU slots
versus parallel CPU threads with cached reference logs). Renders of a batch
finish together, so this is the shape the search produces. GPU is selected
only when its median is at least 10% faster. GPU mode requires successful compute and never silently
falls back. Auto can fall back during initialization/calibration with a visible
reason; a device error during search is terminal. CPU is reproducible by seed;
GPU floating point may vary by adapter/driver, and Auto's choice depends on load.
Session diagnostics retain backend, adapter, timings, real dispatches and cache
counts. See AUTO_MODES_GPU.md for SteelDrum measurements and reproduction, and
PRECISION_PERFORMANCE.md for archived precision and performance comparisons.

DE follows the original [Storn/Price algorithm](https://link.springer.com/article/10.1023/A:1008202821328)
and the documented [SciPy rand1bin formulation](https://docs.scipy.org/doc/scipy/reference/generated/scipy.optimize.differential_evolution.html).
The spectral objective uses the multi-resolution log/convergence pattern in
[ParallelWaveGAN](https://github.com/kan-bayashi/ParallelWaveGAN/blob/master/parallel_wavegan/losses/stft_loss.py),
with DiscoFM-specific envelope, transient, modulation, harmonic and tail terms.
