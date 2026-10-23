//! Deterministic, hardware-constrained inverse synthesis through Nuked-OPN2.
#![forbid(unsafe_code)]
// wgpu's nested backend types require deeper recursion for Send/Sync checks.
#![recursion_limit = "256"]
mod gpu;
mod loss;
mod parameter;
mod search;
mod tracking;
mod worker;
use discofm_analysis::AnalysisOptions;
use discofm_audio::AudioBuffer;
use discofm_chip::Instrument;
use discofm_engine::RenderRequest;
pub use loss::{DEFAULT_INHARMONIC_WEIGHT, LossBreakdown, LossWeights};
pub use parameter::{OperatorField, Parameter, ParameterLocks, all_parameters};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
pub use worker::{FitWorker, WorkerSnapshot};

pub const MAX_RUNS: usize = 8;
pub const MAX_HISTORY: usize = 256;
pub const MAX_CANDIDATES: usize = 16;
pub const LOSS_VERSION: u32 = 6;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModePolicy {
    #[default]
    Preserve,
    Auto,
}
fn automatic() -> ModePolicy {
    ModePolicy::Auto
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// CPU is the default: chip synthesis is most of each evaluation and stays on
/// the CPU, so Auto/GPU (experimental) rarely save more than a few percent.
/// Sessions saved before GPU support lack the field and also ran on the CPU.
pub enum ComputeMode {
    Auto,
    #[default]
    Cpu,
    Gpu,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitPerformance {
    #[serde(default)]
    pub workers: usize,
    pub backend: String,
    pub adapter: Option<String>,
    pub cpu_fft_ms: f64,
    pub gpu_fft_ms: Option<f64>,
    #[serde(default)]
    pub gpu_dispatches: usize,
    pub render_seconds: f64,
    pub resample_seconds: f64,
    pub features_seconds: f64,
    pub cache_hits: usize,
    pub cache_entries: usize,
    pub note: String,
    /// Trials scored through ymfm to pick which ones the chip renders. Their
    /// time is not in the render/resample/features sums.
    #[serde(default)]
    pub screened: usize,
}

#[derive(Debug, Error)]
pub enum FitError {
    #[error("fitting cancelled")]
    Cancelled,
    #[error("GPU fitting: {0}")]
    Compute(String),
    /// A bug inside the fitter (for example a panic in a worker); see the log.
    #[error("internal fitting failure: {0}")]
    Internal(String),
    #[error("invalid fitting settings: {0}")]
    Settings(&'static str),
    #[error("reference is silent or too quiet for fitting")]
    Silent,
    #[error(
        "reference pitch is ambiguous; set a manual pitch or enable unpitched (percussion) mode"
    )]
    Pitch,
    #[error(
        "selected note exceeds fitting duration limit; narrow the region or increase maximum seconds"
    )]
    Duration,
    #[error(transparent)]
    Analysis(#[from] discofm_analysis::AnalysisError),
    #[error(transparent)]
    Audio(#[from] discofm_audio::AudioError),
    #[error(transparent)]
    Engine(#[from] discofm_engine::EngineError),
    #[error(transparent)]
    Parameter(#[from] discofm_chip::ParameterError),
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitMode {
    Basic,
    #[default]
    Advanced,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitStage {
    Preparing,
    Coarse,
    Envelopes,
    Modulation,
    Refine,
}
impl FitStage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing reference",
            Self::Coarse => "Algorithms and timbre",
            Self::Envelopes => "Envelopes and rate scaling",
            Self::Modulation => "SSG, LFO and sensitivities",
            Self::Refine => "Local refinement",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitConfig {
    /// Zero chooses every available CPU worker, up to 16. One is the serial oracle.
    #[serde(default)]
    pub workers: usize,
    #[serde(default = "automatic")]
    pub lfo_policy: ModePolicy,
    #[serde(default = "automatic")]
    pub am_policy: ModePolicy,
    /// Missing in legacy sessions means preserve the original channel mode.
    #[serde(default)]
    pub channel3_policy: ModePolicy,
    pub mode: FitMode,
    pub seed: u64,
    pub evaluations: usize,
    pub population: usize,
    pub candidates: usize,
    /// Fixed F, or each member's starting F when `adaptive`.
    pub differential_weight: f64,
    /// ymfm screening (about ten times cheaper to render and nearly
    /// rank-identical to Nuked on chip targets). 0 = off, as in runs archived
    /// before it existed. 1 = local refinement skips moves ymfm predicts are
    /// worse in both directions. 2..16 also proposes that many DE trials per
    /// member and renders only the best-screened one with Nuked; that cost
    /// about 50% more time for little gain in tests, so the default is 1.
    #[serde(default)]
    pub screening: usize,
    /// Fixed CR, or each member's starting CR when `adaptive`.
    pub crossover: f64,
    /// jDE self-adaptation of F and CR per population member. Off by default:
    /// on chip-generated targets it helped slightly at 512 evaluations but lost
    /// 15 of 20 seed pairs at 2000 (docs/FITTING.md). Absent means fixed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adaptive: bool,
    pub max_seconds: f64,
    /// Only searches special frequencies if the initial patch already uses CH3.
    pub fit_special_frequencies: bool,
    pub weights: LossWeights,
    /// A single recording gain for the entire note; never a gain per frame.
    #[serde(default)]
    pub match_level: bool,
    #[serde(default)]
    pub compute: ComputeMode,
    /// Percussive/noisy references: no detected pitch is required. The chip
    /// plays the manual pitch, otherwise the audition note, as a drum patch would.
    #[serde(default)]
    pub unpitched: bool,
}
impl Default for FitConfig {
    fn default() -> Self {
        Self {
            workers: 0,
            lfo_policy: ModePolicy::Auto,
            am_policy: ModePolicy::Auto,
            channel3_policy: ModePolicy::Auto,
            mode: FitMode::Advanced,
            seed: 2612,
            evaluations: 512,
            population: 16,
            candidates: 6,
            differential_weight: 0.7,
            crossover: 0.8,
            adaptive: false,
            screening: 1,
            max_seconds: 8.0,
            fit_special_frequencies: false,
            weights: LossWeights::default(),
            match_level: true,
            compute: ComputeMode::Cpu,
            unpitched: false,
        }
    }
}
impl FitConfig {
    pub fn validate(&self) -> Result<(), FitError> {
        if self.workers > 16
            || !(24..=20000).contains(&self.evaluations)
            || !(4..=64).contains(&self.population)
            || self.population > self.evaluations
            || !(2..=MAX_CANDIDATES).contains(&self.candidates)
            || !self.differential_weight.is_finite()
            || !(0.1..=2.0).contains(&self.differential_weight)
            || !self.crossover.is_finite()
            || !(0.0..=1.0).contains(&self.crossover)
            || self.screening > 16
            || !self.max_seconds.is_finite()
            || !(0.05..=30.0).contains(&self.max_seconds)
        {
            return Err(FitError::Settings(
                "budget 24..20000, population 4..64, candidates 2..16, duration 0.05..30 s, F 0.1..2, CR 0..1, screening 0..16",
            ));
        }
        self.weights.validate()
            .and_then(|()| if self.evaluations < self.population * if self.mode == FitMode::Advanced { 8 } else { 3 }
                || (self.mode == FitMode::Advanced && self.evaluations < 96) {
                Err(FitError::Settings("budget must be >=3 populations (Basic), or >=8 populations and 96 evaluations (Advanced)"))
            } else { Ok(()) })
    }
}

#[derive(Clone)]
pub struct FitInput {
    pub reference: Arc<AudioBuffer>,
    pub options: AnalysisOptions,
    /// Absolute trigger position in the original reference, preserved even for
    /// slow attacks. A later analysis-region start uses chip-rendered pre-roll.
    pub note_on_seconds: f64,
    pub key_off_seconds: f64,
    pub initial: Instrument,
    pub audition: RenderRequest,
    pub config: FitConfig,
    pub locks: ParameterLocks,
    pub seeds: Vec<Instrument>,
    pub refine: bool,
}
impl FitInput {
    pub(crate) fn effective_audition(&self) -> RenderRequest {
        let mut request = self.audition.clone();
        if self.initial.channel3_special.is_some()
            || (self.config.mode == FitMode::Advanced
                && self.config.channel3_policy == ModePolicy::Auto)
        {
            request.channel = discofm_chip::Channel::THREE;
        }
        request
    }
    pub fn validate(&self) -> Result<(), FitError> {
        self.config.validate()?;
        self.options.validate()?;
        self.audition.validate()?;
        self.initial
            .validate_channel(self.effective_audition().channel)?;
        if !self.note_on_seconds.is_finite()
            || !self.key_off_seconds.is_finite()
            || self.note_on_seconds < 0.0
            || self.key_off_seconds <= self.note_on_seconds
            || self.seeds.len() > MAX_CANDIDATES
        {
            return Err(FitError::Settings(
                "invalid note-on/key-off markers or too many seeds",
            ));
        }
        if !self.initial.channel.left && !self.initial.channel.right {
            return Err(FitError::Settings(
                "enable at least one stereo output before fitting",
            ));
        }
        for lock in &self.locks {
            lock.validate()?;
        }
        for patch in &self.seeds {
            patch.validate_channel(self.effective_audition().channel)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitCandidate {
    pub evaluation: usize,
    pub stage: FitStage,
    pub instrument: Instrument,
    pub loss: LossBreakdown,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryPoint {
    pub evaluation: usize,
    pub stage: FitStage,
    pub best: f64,
    pub current: f64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitContext {
    pub loss_version: u32,
    /// FNV-1a of prepared samples and rendering/loss settings. Diagnostic identity,
    /// not a cryptographic proof or a promise that different runs share a target.
    pub fingerprint: String,
    pub sample_rate: u32,
    pub frames: usize,
    pub reference_onset_seconds: f64,
    pub reference_start_seconds: f64,
    pub reference_end_seconds: f64,
    pub reference_gain: f64,
    pub pitch_hz: f64,
    pub audition: RenderRequest,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitStatus {
    Completed,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitOutcome {
    pub status: FitStatus,
    pub config: FitConfig,
    pub locks: ParameterLocks,
    pub initial: Instrument,
    pub context: FitContext,
    pub initial_loss: LossBreakdown,
    pub candidates: Vec<FitCandidate>,
    pub history: Vec<HistoryPoint>,
    pub evaluations: usize,
    pub renders: usize,
    pub generations: usize,
    pub elapsed_seconds: f64,
    pub refined: bool,
    #[serde(default)]
    pub performance: FitPerformance,
}
impl FitOutcome {
    pub fn validate(&self) -> Result<(), FitError> {
        self.config.validate()?;
        self.context.audition.validate()?;
        self.initial
            .validate_channel(self.context.audition.channel)?;
        self.initial_loss.validate(&self.config.weights)?;
        let context = &self.context;
        let p = &self.performance;
        if [
            &p.cpu_fft_ms,
            &p.render_seconds,
            &p.resample_seconds,
            &p.features_seconds,
        ]
        .into_iter()
        .any(|v| !v.is_finite() || *v < 0.0)
            || p.gpu_fft_ms.is_some_and(|v| !v.is_finite() || v < 0.0)
            || (!p.backend.is_empty()
                && (p.cache_hits != self.evaluations.saturating_sub(self.renders)
                    || p.cache_entries > self.renders))
            || p.backend.len() > 256
            || p.note.len() > 4096
            || p.adapter.as_ref().is_some_and(|v| v.len() > 1024)
            || p.workers > 16
        {
            return Err(FitError::Settings("invalid performance diagnostics"));
        }
        if context.loss_version >= 2
            && (context.audition.pitch_hz()? / context.pitch_hz)
                .log2()
                .abs()
                > 1e-8
        {
            return Err(FitError::Settings(
                "archive tuning differs from evaluated reference pitch",
            ));
        }
        if !(1..=LOSS_VERSION).contains(&context.loss_version)
            || context.sample_rate != 24000
            || context.frames == 0
            || context.frames > 720000
            || context.fingerprint.len() != 16
            || !context.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            || !context.reference_onset_seconds.is_finite()
            || context.reference_onset_seconds < 0.0
            || !context.reference_end_seconds.is_finite()
            || context.reference_end_seconds <= context.reference_onset_seconds
            || !context.reference_start_seconds.is_finite()
            || context.reference_start_seconds < context.reference_onset_seconds
            || context.reference_start_seconds >= context.reference_end_seconds
            || !context.reference_gain.is_finite()
            || context.reference_gain <= 0.0
            || !context.pitch_hz.is_finite()
            || !(20.0..=4000.0).contains(&context.pitch_hz)
            || self.evaluations == 0
            || self.evaluations > self.config.evaluations
            || self.renders > self.evaluations
            || self.generations > self.evaluations
            || !self.elapsed_seconds.is_finite()
            || self.elapsed_seconds < 0.0
            || self.candidates.is_empty()
            || self.candidates.len() > self.config.candidates
            || self.history.len() > MAX_HISTORY
        {
            return Err(FitError::Settings("invalid fitting archive/context bounds"));
        }
        if (context.audition.total_seconds
            - (context.reference_end_seconds - context.reference_onset_seconds))
            .abs()
            > 1.0 / 8000.0
            || (context.frames as f64 / f64::from(context.sample_rate)
                - (context.reference_end_seconds - context.reference_start_seconds))
                .abs()
                > 2.0 / 8000.0
        {
            return Err(FitError::Settings(
                "archive timing does not match evaluated audio",
            ));
        }
        for lock in &self.locks {
            lock.validate()?;
        }
        let mut previous = -1.0;
        let searchable =
            parameter::searchable(&self.config, FitStage::Refine, &self.initial, &self.locks);
        for (index, candidate) in self.candidates.iter().enumerate() {
            candidate.loss.validate(&self.config.weights)?;
            candidate
                .instrument
                .validate_channel(context.audition.channel)?;
            if candidate.evaluation == 0
                || candidate.evaluation > self.evaluations
                || candidate.loss.total < previous
                || self.candidates[..index]
                    .iter()
                    .any(|old| old.instrument == candidate.instrument)
                || self
                    .locks
                    .iter()
                    .any(|p| p.get(&candidate.instrument) != p.get(&self.initial))
                || all_parameters(self.initial.channel3_special.is_some())
                    .iter()
                    .any(|p| {
                        !searchable.contains(p)
                            && p.get(&candidate.instrument) != p.get(&self.initial)
                    })
                || (candidate.instrument.channel3_special.is_some()
                    != self.initial.channel3_special.is_some()
                    && (context.loss_version < 4 || !searchable.contains(&Parameter::Channel3Mode)))
                || candidate.instrument.channel.left != self.initial.channel.left
                || candidate.instrument.channel.right != self.initial.channel.right
                || candidate.instrument.audition != self.initial.audition
            {
                return Err(FitError::Settings(
                    "invalid candidate ranking, locks or fixed settings",
                ));
            }
            previous = candidate.loss.total;
        }
        let mut evaluation = 0;
        let mut best = self.initial_loss.total;
        for point in &self.history {
            if point.evaluation <= evaluation
                || point.evaluation > self.evaluations
                || !point.best.is_finite()
                || !point.current.is_finite()
                || point.current < 0.0
                || point.best < 0.0
                || point.best > best + 1e-12
                || point.best > point.current + 1e-12
            {
                return Err(FitError::Settings("invalid fitting history"));
            }
            evaluation = point.evaluation;
            best = point.best;
        }
        if self.history.first().is_none_or(|p| {
            p.evaluation != 1
                || (p.best - self.initial_loss.total).abs() > 1e-12
                || (p.current - self.initial_loss.total).abs() > 1e-12
        }) || self.history.last().is_none_or(|p| {
            p.evaluation != self.evaluations
                || (p.best - self.candidates[0].loss.total).abs() > 1e-12
        }) {
            return Err(FitError::Settings(
                "history does not match baseline or final ranking",
            ));
        }
        if self.candidates[0].loss.total > self.initial_loss.total + 1e-12 {
            return Err(FitError::Settings(
                "best candidate is worse than the initial patch",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FittingState {
    pub config: FitConfig,
    pub locks: ParameterLocks,
    pub runs: Vec<FitOutcome>,
}
impl FittingState {
    pub fn validate(&self) -> Result<(), FitError> {
        self.config.validate()?;
        for lock in &self.locks {
            lock.validate()?;
        }
        if self.runs.len() > MAX_RUNS {
            return Err(FitError::Settings(
                "at most eight fitting runs can be retained",
            ));
        }
        for run in &self.runs {
            run.validate()?;
        }
        Ok(())
    }
    pub fn push(&mut self, run: FitOutcome) {
        if self.runs.len() == MAX_RUNS {
            self.runs.remove(0);
        }
        self.runs.push(run);
    }
}

#[derive(Debug, Clone)]
pub struct FitProgress {
    pub stage: FitStage,
    pub evaluations: usize,
    pub budget: usize,
    pub renders: usize,
    pub generation: usize,
    pub current: Option<f64>,
    pub best: Option<FitCandidate>,
    pub elapsed_seconds: f64,
}
/// Workers used when `FitConfig::workers` is zero: every logical CPU, up to
/// 16. Renders dominate and still gain from SMT once scoring is parallel.
pub fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(16)
}
/// A fixed seed and CPU backend reproduce scores, rankings and evaluation counts.
/// Auto benchmarks hardware; GPU arithmetic may vary between adapters/drivers.
/// Callback is called after completed evaluations, never faked.
pub fn fit(
    input: FitInput,
    cancel: impl Fn() -> bool,
    mut progress: impl FnMut(FitProgress),
) -> Result<FitOutcome, FitError> {
    fit_inner(input, &cancel, &mut progress)
}
// Keep search and Rayon GPU worker instantiations inside this crate so callers
// do not inherit wgpu's deeper Send/Sync recursion requirements.
fn fit_inner(
    mut input: FitInput,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(FitProgress),
) -> Result<FitOutcome, FitError> {
    // Weights saved before version 6 lack `inharmonic`; new runs score and
    // archive the default instead of silently disabling the term.
    input.config.weights = input.config.weights.resolved();
    input.validate()?;
    search::fit(input, &cancel, progress)
}
