use crate::{Clock, Profile, same_path};
use clap::{Args, ValueEnum};
use discofm_analysis::AnalysisOptions;
use discofm_fit::{
    FitInput, FitMode, FitOutcome, FitWorker, FittingState, OperatorField, Parameter,
    all_parameters,
};
use discofm_formats::{ReferenceSettings, Session, read_session, write_session};
use std::{
    error::Error,
    fs::File,
    io::IsTerminal,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Basic,
    Advanced,
}
#[derive(Clone, Copy, ValueEnum)]
enum Compute {
    Auto,
    Cpu,
    Gpu,
}
#[derive(Clone, Copy, ValueEnum)]
enum Policy {
    Auto,
    Preserve,
}
impl From<Policy> for discofm_fit::ModePolicy {
    fn from(value: Policy) -> Self {
        match value {
            Policy::Auto => Self::Auto,
            Policy::Preserve => Self::Preserve,
        }
    }
}
#[derive(Args)]
pub(crate) struct FitArgs {
    /// Parallel synthesis workers; zero selects every logical CPU, up to 16.
    #[arg(long)]
    workers: Option<usize>,
    #[arg(long, value_enum)]
    lfo: Option<Policy>,
    #[arg(long, value_enum)]
    am: Option<Policy>,
    #[arg(long, value_enum)]
    channel3: Option<Policy>,
    #[arg(long)]
    reference: PathBuf,
    /// One-based soundfont sample ID; use `samples` to list.
    #[arg(long, value_parser=clap::value_parser!(u32).range(1..))]
    sample: Option<u32>,
    /// New internal .json session; includes ranked patches and fitting history.
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    session: Option<PathBuf>,
    #[arg(long, value_enum)]
    mode: Option<Mode>,
    #[arg(long)]
    evaluations: Option<usize>,
    #[arg(long)]
    population: Option<usize>,
    #[arg(long)]
    seed: Option<u64>,
    #[arg(long)]
    max_seconds: Option<f64>,
    #[arg(long)]
    pitch: Option<f64>,
    #[arg(long)]
    note_on: Option<f64>,
    /// Key-off time in the reference, in seconds. Default: the session's saved
    /// marker, else the end of the selected region (as in the editor).
    #[arg(long)]
    key_off: Option<f64>,
    #[arg(long)]
    start: Option<f64>,
    #[arg(long)]
    end: Option<f64>,
    #[arg(long)]
    normalize: bool,
    /// Batched spectral loss backend: cpu (default); auto and gpu are
    /// experimental (auto measures CPU and GPU before choosing).
    #[arg(long, value_enum)]
    compute: Option<Compute>,
    /// ymfm screening: 0 off, 1 (default) skips refinement moves ymfm predicts
    /// are worse, 2..16 also screens that many DE trials per member.
    #[arg(long)]
    screening: Option<usize>,
    /// jDE self-adaptation of F/CR (default false: fixed F 0.7, CR 0.8).
    #[arg(long, action = clap::ArgAction::Set)]
    adaptive: Option<bool>,
    /// Compensate one recording gain for the whole note (true for new runs).
    #[arg(long, action = clap::ArgAction::Set)]
    match_level: Option<bool>,
    /// Percussion/noise: no detected pitch required; plays --pitch or the
    /// lowest strong spectral peak and switches to band-energy loss weights.
    #[arg(long, action = clap::ArgAction::Set)]
    unpitched: Option<bool>,
    #[arg(long, value_enum)]
    profile: Option<Profile>,
    #[arg(long, value_enum)]
    clock: Option<Clock>,
    /// Repeat: algorithm, feedback, lfo_enabled, lfo_rate, ams, fms, s1.mul, s4.tl, all.
    #[arg(long)]
    lock: Vec<String>,
    #[arg(long)]
    unlock_all: bool,
    /// Refine this 1-based candidate from the latest saved run.
    #[arg(long)]
    refine_candidate: Option<usize>,
    /// Scriptable cancellation after at least this many completed evaluations.
    #[arg(long)]
    cancel_after: Option<usize>,
}
fn parameter(text: &str) -> Result<Parameter, Box<dyn Error>> {
    let text = text.to_ascii_lowercase();
    let global = match text.as_str() {
        "algorithm" => Some(Parameter::Algorithm),
        "channel3_mode" => Some(Parameter::Channel3Mode),
        "feedback" => Some(Parameter::Feedback),
        "lfo_enabled" => Some(Parameter::LfoEnabled),
        "lfo_rate" => Some(Parameter::LfoRate),
        "ams" => Some(Parameter::Ams),
        "fms" => Some(Parameter::Fms),
        _ => None,
    };
    if let Some(p) = global {
        return Ok(p);
    }
    if let Some((operator, field)) = text.split_once('.') {
        let index = operator
            .strip_prefix('s')
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|v| (1..=4).contains(v))
            .map(|v| v - 1)
            .ok_or("lock operator must be s1..s4")?;
        let field = match field {
            "mul" => OperatorField::Mul,
            "tl" => OperatorField::Tl,
            "dt" => OperatorField::Dt,
            "ar" => OperatorField::Ar,
            "d1r" => OperatorField::D1r,
            "d2r" => OperatorField::D2r,
            "sl" => OperatorField::Sl,
            "rr" => OperatorField::Rr,
            "rs" => OperatorField::Rs,
            "am" => OperatorField::Am,
            "ssg" => OperatorField::Ssg,
            "fnum" if index < 3 => return Ok(Parameter::SpecialFnum { index }),
            "block" if index < 3 => return Ok(Parameter::SpecialBlock { index }),
            _ => return Err("unknown operator lock field".into()),
        };
        return Ok(Parameter::Operator { index, field });
    }
    Err(format!("unknown parameter lock: {text}").into())
}
struct KeyboardCancel(bool);
impl KeyboardCancel {
    fn open() -> Result<Self, Box<dyn Error>> {
        let interactive = std::io::stdin().is_terminal();
        if interactive {
            crossterm::terminal::enable_raw_mode()?;
        }
        Ok(Self(interactive))
    }
    fn requested(&self) -> Result<bool, Box<dyn Error>> {
        use crossterm::event::{self, Event, KeyCode, KeyModifiers};
        if self.0
            && event::poll(Duration::ZERO)?
            && let Event::Key(key) = event::read()?
        {
            return Ok(key.code == KeyCode::Esc
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)));
        }
        Ok(false)
    }
}
impl Drop for KeyboardCancel {
    fn drop(&mut self) {
        if self.0 {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}
fn optimize(input: FitInput, cancel_after: Option<usize>) -> Result<FitOutcome, Box<dyn Error>> {
    let keyboard = KeyboardCancel::open()?;
    let worker = FitWorker::start(input)?;
    let mut last_log = Instant::now() - Duration::from_secs(1);
    loop {
        let snapshot = worker.poll();
        if let Some(progress) = snapshot.progress {
            if cancel_after.is_some_and(|limit| progress.evaluations >= limit) {
                worker.cancel();
            }
            if last_log.elapsed() >= Duration::from_millis(250) {
                eprint!(
                    "\r{} · {}/{} evaluations · {} renders · best {} · {:.1} s\r\n",
                    progress.stage.label(),
                    progress.evaluations,
                    progress.budget,
                    progress.renders,
                    progress
                        .best
                        .as_ref()
                        .map_or("—".into(), |c| format!("{:.6}", c.loss.total)),
                    progress.elapsed_seconds
                );
                last_log = Instant::now();
            }
        }
        if let Some(result) = snapshot.result {
            return result.map_err(Into::into);
        }
        if keyboard.requested()? {
            worker.cancel();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub(crate) fn run(args: FitArgs) -> Result<(), Box<dyn Error>> {
    if args.output.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "fitting output must be a new file",
        )
        .into());
    }
    if !args
        .output
        .extension()
        .is_some_and(|v| v.eq_ignore_ascii_case("json"))
    {
        return Err("fitting output must have extension .json (internal session)".into());
    }
    if same_path(&args.output, &args.reference)?
        || args
            .session
            .as_ref()
            .map(|p| same_path(&args.output, p))
            .transpose()?
            .unwrap_or(false)
    {
        return Err("fitting output must not overwrite reference or input session".into());
    }
    if args.cancel_after == Some(0) {
        return Err("cancel-after must be at least 1".into());
    }
    let mut session = match args.session {
        Some(ref path) => read_session(File::open(path)?)?,
        None => Session::default(),
    };
    let saved = session.reference_analysis.clone();
    let mut options = saved
        .as_ref()
        .map_or_else(AnalysisOptions::default, |s| s.options.clone());
    if let Some(pitch) = args.pitch {
        options.pitch_override_hz = Some(pitch);
    }
    if let Some(start) = args.start {
        options.start_seconds = start;
    }
    if let Some(end) = args.end {
        options.end_seconds = Some(end);
    }
    if args.normalize {
        options.normalize = true;
    }
    if let Some(profile) = args.profile {
        session.audition.profile = profile.into();
    }
    if let Some(clock) = args.clock {
        session.audition.clock = clock.into();
    }
    let note_on = args
        .note_on
        .unwrap_or_else(|| saved.as_ref().map_or(0.0, |s| s.note_on_seconds));
    let key_off = args
        .key_off
        .or_else(|| saved.as_ref().map(|s| s.key_off_seconds));
    let state = session.fitting.get_or_insert_with(FittingState::default);
    let mut config = state.config.clone();
    if let Some(workers) = args.workers {
        config.workers = workers;
    }
    if let Some(policy) = args.lfo {
        config.lfo_policy = policy.into();
    }
    if let Some(policy) = args.am {
        config.am_policy = policy.into();
    }
    if let Some(policy) = args.channel3 {
        config.channel3_policy = policy.into();
    }
    if let Some(compute) = args.compute {
        config.compute = match compute {
            Compute::Auto => discofm_fit::ComputeMode::Auto,
            Compute::Cpu => discofm_fit::ComputeMode::Cpu,
            Compute::Gpu => discofm_fit::ComputeMode::Gpu,
        };
    }
    if let Some(value) = args.screening {
        config.screening = value;
    }
    if let Some(value) = args.adaptive {
        config.adaptive = value;
    }
    if let Some(value) = args.match_level {
        config.match_level = value;
    }
    if let Some(value) = args.unpitched.filter(|v| *v != config.unpitched) {
        config.unpitched = value;
        config.weights = if value {
            discofm_fit::LossWeights::unpitched()
        } else {
            discofm_fit::LossWeights::default()
        };
    }
    if let Some(mode) = args.mode {
        config.mode = match mode {
            Mode::Basic => FitMode::Basic,
            Mode::Advanced => FitMode::Advanced,
        };
    }
    if let Some(evaluations) = args.evaluations {
        config.evaluations = evaluations;
    }
    if let Some(population) = args.population {
        config.population = population;
    }
    if let Some(seed) = args.seed {
        config.seed = seed;
    }
    if let Some(seconds) = args.max_seconds {
        config.max_seconds = seconds;
    }
    if args.unlock_all {
        state.locks.clear();
    }
    let (initial, seeds) = if let Some(index) = args.refine_candidate {
        let run = state.runs.last().ok_or("no fitting history to refine")?;
        let candidate = index
            .checked_sub(1)
            .and_then(|i| run.candidates.get(i))
            .ok_or("refine-candidate is outside the saved candidate list")?;
        (
            candidate.instrument.clone(),
            run.candidates
                .iter()
                .map(|c| c.instrument.clone())
                .collect(),
        )
    } else {
        (session.instrument.clone(), Vec::new())
    };
    for lock in args.lock {
        if lock.eq_ignore_ascii_case("all") {
            state
                .locks
                .extend(all_parameters(initial.channel3_special.is_some()));
        } else {
            state.locks.insert(parameter(&lock)?);
        }
    }
    if let Some(channel) = initial.required_channel() {
        session.audition.channel = channel;
    }
    config.validate()?;
    let reference_path = args.reference.canonicalize()?;
    let locks = state.locks.clone();
    let saved = crate::banks::saved_selection(&session, args.session.as_deref(), &reference_path);
    let selection = crate::banks::selection(&reference_path, args.sample, saved)?;
    let reference = Arc::new(discofm_soundfont::read_reference(
        &reference_path,
        selection.as_ref(),
        &|| false,
    )?);
    // The editor's default: key-off at the end of the selected region. The old
    // fallback, the audition gate (0.5 s), released long notes far too early.
    let key_off = key_off.unwrap_or_else(|| {
        let duration = reference.frames.len() as f64 / f64::from(reference.sample_rate);
        options
            .end_seconds
            .unwrap_or(options.start_seconds + discofm_analysis::MAX_SECONDS)
            .min(duration)
    });
    let input = FitInput {
        reference,
        options: options.clone(),
        note_on_seconds: note_on,
        key_off_seconds: key_off,
        initial,
        audition: session.audition.clone(),
        config: config.clone(),
        locks,
        seeds,
        refine: args.refine_candidate.is_some(),
    };
    let outcome = optimize(input, args.cancel_after)?;
    println!(
        "{:?}: loss {:.6} -> {:.6}; {} candidates, {} evaluations, {} renders, {:.2} s",
        outcome.status,
        outcome.initial_loss.total,
        outcome.candidates[0].loss.total,
        outcome.candidates.len(),
        outcome.evaluations,
        outcome.renders,
        outcome.elapsed_seconds
    );
    session.instrument = outcome.candidates[0].instrument.clone();
    println!(
        "{} · {:?} · FFT CPU {:.3} ms / GPU {:?} ms · cache {} hits / {} entries · synthesis {:.2} s / resample {:.2} s / features {:.2} s · recording gain ×{:.3}\n{}",
        outcome.performance.backend,
        outcome.performance.adapter,
        outcome.performance.cpu_fft_ms,
        outcome.performance.gpu_fft_ms,
        outcome.performance.cache_hits,
        outcome.performance.cache_entries,
        outcome.performance.render_seconds,
        outcome.performance.resample_seconds,
        outcome.performance.features_seconds,
        outcome.candidates[0].loss.recording_gain,
        outcome.performance.note
    );
    session.audition = outcome.context.audition.clone();
    session.reference_path = Some(reference_path.to_string_lossy().into_owned());
    session.reference_sample = selection;
    session.reference_analysis = Some(ReferenceSettings {
        options,
        note_on_seconds: note_on,
        key_off_seconds: key_off,
    });
    let state = session.fitting.get_or_insert_with(FittingState::default);
    state.config = config;
    state.push(outcome);
    let mut bytes = Vec::new();
    write_session(&mut bytes, &session)?;
    use std::io::Write;
    File::create_new(&args.output)?.write_all(&bytes)?;
    println!("Fitting session saved: {}", args.output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_names_map_to_exact_hardware_fields_and_reject_invalid_slots() {
        assert_eq!(
            parameter("s4.tl").unwrap(),
            Parameter::Operator {
                index: 3,
                field: OperatorField::Tl
            }
        );
        assert_eq!(parameter("LFO_ENABLED").unwrap(), Parameter::LfoEnabled);
        for invalid in [
            "s0.tl",
            "s5.mul",
            "s4.fnum",
            "s1.velocity",
            "Algorithm.extra",
        ] {
            assert!(parameter(invalid).is_err());
        }
    }
}
