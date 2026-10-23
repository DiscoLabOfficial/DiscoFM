use clap::Args;
use discofm_audio::{AudioBuffer, from_native};
use discofm_engine::{BACKEND_COMMIT, YMFM_COMMIT, render_note_cancellable};
use discofm_formats::read_session;
use std::{error::Error, fs::File, path::PathBuf};

#[derive(Args)]
pub(crate) struct CompareArgs {
    #[arg(long)]
    session: PathBuf,
    /// WAV recorded from hardware, or any independent reference for this patch.
    /// Omit to compare with the pinned ymfm core.
    #[arg(long)]
    reference: Option<PathBuf>,
    #[arg(long, requires="reference", value_parser=clap::value_parser!(u32).range(1..))]
    sample: Option<u32>,
    /// Start of the note in the recorded file, in seconds.
    #[arg(long, default_value_t = 0.0)]
    start: f64,
    /// Bounded waveform alignment; no time stretching.
    #[arg(long, default_value_t = 0.0)]
    align_ms: f64,
    #[arg(long)]
    match_level: bool,
    /// Optional new JSON report file.
    #[arg(long)]
    output: Option<PathBuf>,
}
pub(crate) fn run(args: CompareArgs) -> Result<(), Box<dyn Error>> {
    if !args.start.is_finite()
        || args.start < 0.0
        || !args.align_ms.is_finite()
        || !(0.0..=20.0).contains(&args.align_ms)
    {
        return Err("require start >=0 and alignment within 0..20 ms".into());
    }
    if args.output.as_ref().is_some_and(|p| p.exists()) {
        return Err("comparison output must be a new file".into());
    }
    let session = read_session(File::open(&args.session)?)?;
    let (source, report) = if let Some(path) = &args.reference {
        let saved = crate::banks::saved_selection(&session, Some(&args.session), path);
        let selection = crate::banks::selection(path, args.sample, saved)?;
        let recording = discofm_soundfont::read_reference(path, selection.as_ref(), &|| false)?;
        let start = (args.start * f64::from(recording.sample_rate)).round() as usize;
        let count =
            (session.audition.total_seconds * f64::from(recording.sample_rate)).round() as usize;
        let end = start
            .checked_add(count)
            .filter(|end| *end <= recording.frames.len())
            .ok_or("recording does not contain a complete note at this start")?;
        let reference = AudioBuffer {
            frames: recording.frames[start..end].to_vec(),
            sample_rate: recording.sample_rate,
        };
        let actual = from_native(
            &render_note_cancellable(&session.instrument, &session.audition, || false)?,
            24000,
        )?;
        (
            path.display().to_string(),
            discofm_reference::compare_audio(
                &actual,
                &reference,
                args.align_ms / 1000.0,
                args.match_level,
            )?,
        )
    } else {
        if args.start != 0.0 || args.align_ms != 0.0 || args.match_level {
            return Err("start/alignment/gain overrides apply only to recorded WAV comparisons; ymfm uses fixed calibration".into());
        }
        (
            format!("ymfm {YMFM_COMMIT}"),
            discofm_reference::compare_ymfm(&session.instrument, &session.audition)?,
        )
    };
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "source":source,"nuked_commit":BACKEND_COMMIT,"ymfm_commit":YMFM_COMMIT,
        "request":session.audition,"instrument":session.instrument,
        "recording_start_seconds":args.start,"comparison":report,
        "scope":"Audio behavior comparison; not bit-exact pin or physical-hardware certification."
    }))?;
    if let Some(path) = args.output {
        use std::io::Write;
        File::create_new(path)?.write_all(json.as_bytes())?;
    }
    println!("{json}");
    Ok(())
}
