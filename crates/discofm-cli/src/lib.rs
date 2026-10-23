//! Desktop launch, diagnostic rendering and backend integration validation.
#![forbid(unsafe_code)]
mod audition;
mod banks;
mod comparison;
mod fitting;
use clap::{Parser, Subcommand, ValueEnum};
use discofm_audio::{from_native, write_wav};
use discofm_engine::{BACKEND_COMMIT, ChipProfile, ClockConfig, render_note};
use discofm_formats::{InstrumentFormat, Session, read_session, write_session};
use std::{
    error::Error,
    fs::File,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
#[command(
    name = "DiscoFM",
    bin_name = "discofm",
    version,
    about = "DiscoFM — YM2612/YM3438 instrument editor, reference analysis and inverse synthesis"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Open the graphical instrument editor and audio/sample-bank reference analyzer.
    Gui {
        #[arg(long)]
        session: Option<PathBuf>,
        #[arg(long)]
        reference: Option<PathBuf>,
        /// One-based internal sample ID (use `samples` to list).
        #[arg(long, requires="reference", value_parser=clap::value_parser!(u32).range(1..))]
        sample: Option<u32>,
        #[arg(long, hide = true)]
        smoke_screenshot: Option<PathBuf>,
    },
    /// List audio output device IDs and their default configurations.
    Devices,
    /// List the internal samples of an SF2/SF3/SFZ/DLS bank as JSON.
    Samples { bank: PathBuf },
    /// Extract one internal sample to a new float32 stereo WAV.
    ExtractSample {
        bank: PathBuf,
        #[arg(long, value_parser=clap::value_parser!(u32).range(1..))]
        sample: u32,
        #[arg(long)]
        output: PathBuf,
    },
    /// Play one internal sample once through CPAL at its original rate/pitch.
    PlaySample {
        bank: PathBuf,
        #[arg(long, value_parser=clap::value_parser!(u32).range(1..))]
        sample: u32,
        #[arg(long)]
        device: Option<String>,
    },
    /// Audition a session through CPAL with live note/release/stop controls.
    Play(audition::PlayArgs),
    /// Fit YM2612 parameters to a WAV or soundfont sample. Esc/Ctrl+C cancels in a terminal.
    Fit(Box<fitting::FitArgs>),
    /// Compare a session with pinned ymfm, or an independent recorded WAV.
    Compare(comparison::CompareArgs),
    /// Render FM through Nuked-OPN2 to a stereo float32 diagnostic WAV.
    Render {
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        session: Option<PathBuf>,
        #[arg(long)]
        note: Option<u8>,
        #[arg(long)]
        key_on: Option<f64>,
        #[arg(long)]
        seconds: Option<f64>,
        #[arg(long, value_enum)]
        profile: Option<Profile>,
        #[arg(long, value_enum)]
        clock: Option<Clock>,
        #[arg(long, default_value_t = 48000)]
        sample_rate: u32,
        /// Save exact port events and capture metadata for diagnostic comparison.
        #[arg(long)]
        trace: Option<PathBuf>,
    },
    /// Save an editable, versioned internal session (not instrument export).
    NewSession {
        #[arg(long)]
        output: PathBuf,
    },
    /// Export a session's instrument as a tracker instrument (TFI, VGI or TYI,
    /// as GENNY writes them). The format follows the output extension.
    Export {
        #[arg(long)]
        session: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Create a new session from a TFI, VGI or TYI instrument file.
    ImportInstrument {
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Validate a session and show routing plus actual decoded chip registers.
    InspectSession {
        #[arg(long)]
        session: PathBuf,
    },
    /// Compare the Rust bridge against a separate direct-upstream C driver.
    Validate,
}
#[derive(Clone, Copy, ValueEnum)]
enum Profile {
    Ym2612,
    Ym3438,
    RawDiagnostic,
}
impl From<Profile> for ChipProfile {
    fn from(profile: Profile) -> Self {
        match profile {
            Profile::Ym2612 => Self::Ym2612Discrete,
            Profile::Ym3438 => Self::Ym3438,
            Profile::RawDiagnostic => Self::RawDiagnostic,
        }
    }
}
#[derive(Clone, Copy, ValueEnum)]
enum Clock {
    Ntsc,
    Pal,
}
impl From<Clock> for ClockConfig {
    fn from(clock: Clock) -> Self {
        match clock {
            Clock::Ntsc => Self::Ntsc,
            Clock::Pal => Self::Pal,
        }
    }
}

fn destination(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    if absolute.exists() {
        return absolute.canonicalize();
    }
    let parent = absolute
        .parent()
        .ok_or_else(|| std::io::Error::other("destination has no parent"))?
        .canonicalize()?;
    let name = absolute
        .file_name()
        .ok_or_else(|| std::io::Error::other("destination has no filename"))?;
    Ok(parent.join(name))
}
fn same_path(a: &Path, b: &Path) -> std::io::Result<bool> {
    let a = destination(a)?;
    let b = destination(b)?;
    if cfg!(windows) {
        Ok(a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy()))
    } else {
        Ok(a == b)
    }
}
fn execute(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        None => discofm_ui::run(discofm_ui::LaunchOptions::default())?,
        Some(command) => match command {
            Command::Gui {
                session,
                reference,
                sample,
                smoke_screenshot,
            } => {
                let reference_sample = if sample.is_some() {
                    reference
                        .as_ref()
                        .map(|p| banks::selection(p, sample, None))
                        .transpose()?
                        .flatten()
                } else {
                    None
                };
                discofm_ui::run(discofm_ui::LaunchOptions {
                    session,
                    reference,
                    reference_sample,
                    smoke_screenshot,
                })?;
            }
            Command::Samples { bank } => banks::list(&bank)?,
            Command::ExtractSample {
                bank,
                sample,
                output,
            } => banks::extract(&bank, sample, &output)?,
            Command::PlaySample {
                bank,
                sample,
                device,
            } => banks::play(&bank, sample, device)?,
            Command::Devices => println!(
                "{}",
                serde_json::to_string_pretty(&discofm_audio::output_devices()?)?
            ),
            Command::Play(args) => audition::run(args)?,
            Command::Fit(args) => fitting::run(*args)?,
            Command::Compare(args) => comparison::run(args)?,
            Command::NewSession { output } => {
                write_session(File::create_new(&output)?, &Session::default())?;
                println!("Internal session saved: {}", output.display());
            }
            Command::Export { session, output } => {
                let format = InstrumentFormat::from_path(&output)
                    .ok_or("output must end in .tfi, .vgi or .tyi")?;
                if same_path(&output, &session)? {
                    return Err("export must not overwrite the input session".into());
                }
                let session = read_session(File::open(session)?)?;
                let omitted = format.omitted(&session.instrument);
                std::io::Write::write_all(
                    &mut File::create_new(&output)?,
                    &format.encode(&session.instrument),
                )?;
                println!("{} instrument saved: {}", format.name(), output.display());
                if !omitted.is_empty() {
                    eprintln!("Not stored by this format: {}", omitted.join(", "));
                }
            }
            Command::ImportInstrument { input, output } => {
                let session = Session {
                    instrument: InstrumentFormat::read(&input)?,
                    ..Session::default()
                };
                write_session(File::create_new(&output)?, &session)?;
                println!("Internal session saved: {}", output.display());
            }
            Command::InspectSession { session } => {
                let session = read_session(File::open(session)?)?;
                let algorithm = session.instrument.channel.algorithm;
                let snapshot =
                    discofm_engine::inspect_instrument(&session.instrument, &session.audition)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "session_version": session.version, "profile": session.audition.profile,
                        "clock": session.audition.clock, "channel": session.audition.channel,
                        "channel3_specific": session.instrument.required_channel().is_some(),
                        "algorithm_register": algorithm.get(), "algorithm_display": algorithm.display_number(),
                        "routing": algorithm.routing(), "feedback_operator": algorithm.feedback_operator(),
                        "hardware": snapshot
                    }))?
                );
            }
            Command::Render {
                output,
                session,
                note,
                key_on,
                seconds,
                profile,
                clock,
                sample_rate,
                trace,
            } => {
                if let Some(path) = &trace
                    && same_path(&output, path)?
                {
                    return Err("WAV and trace destinations must be different".into());
                }
                if let Some(path) = &session
                    && (same_path(&output, path)?
                        || trace
                            .as_ref()
                            .map(|trace| same_path(path, trace))
                            .transpose()?
                            .unwrap_or(false))
                {
                    return Err("render destinations must not overwrite the input session".into());
                }
                let mut session = match session {
                    Some(path) => read_session(File::open(path)?)?,
                    None => Session::default(),
                };
                let request = &mut session.audition;
                if let Some(note) = note {
                    request.note = note;
                }
                if let Some(key_on) = key_on {
                    request.key_on_seconds = key_on;
                }
                if let Some(seconds) = seconds {
                    request.total_seconds = seconds;
                }
                if let Some(profile) = profile {
                    request.profile = profile.into();
                }
                if let Some(clock) = clock {
                    request.clock = clock.into();
                }
                let native = render_note(&session.instrument, request)?;
                let audio = from_native(&native, sample_rate)?;
                write_wav(&output, &audio)?;
                if let Some(path) = trace {
                    serde_json::to_writer_pretty(
                        File::create_new(path)?,
                        &serde_json::json!({
                            "diagnostic_trace_version": 1, "backend_commit": BACKEND_COMMIT,
                            "profile": request.profile, "clock": request.clock,
                            "tick_hz": request.clock.tick_hz(), "capture_start": native.schedule.capture_start,
                            "capture_ticks": native.schedule.frames * 24,
                            "key_off_tick": native.schedule.key_off_tick, "events": native.schedule.events
                        }),
                    )?;
                }
                println!(
                    "Rendered {} stereo frames at {} Hz: {}",
                    audio.frames.len(),
                    audio.sample_rate,
                    output.display()
                );
                println!(
                    "{:?}, {:?}, native {:.6} Hz, FNUM {}, BLOCK {}, backend {}",
                    request.profile,
                    request.clock,
                    native.sample_rate,
                    native.schedule.frequency.fnum(),
                    native.schedule.frequency.block(),
                    BACKEND_COMMIT
                );
            }
            Command::Validate => {
                let session = Session::default();
                for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
                    let request = discofm_engine::RenderRequest {
                        profile,
                        key_on_seconds: 0.05,
                        total_seconds: 0.1,
                        ..session.audition.clone()
                    };
                    let report = discofm_reference::compare_note(&session.instrument, &request)?;
                    println!("{:?}: {}", profile, serde_json::to_string(&report)?);
                    if report.mismatched_ticks != 0 {
                        return Err("direct upstream comparison failed".into());
                    }
                }
                println!("Independent ymfm {}", discofm_engine::YMFM_COMMIT);
                for profile in [ChipProfile::Ym2612Discrete, ChipProfile::Ym3438] {
                    let request = discofm_engine::RenderRequest {
                        profile,
                        key_on_seconds: 0.12,
                        total_seconds: 0.25,
                        ..session.audition.clone()
                    };
                    let report = discofm_reference::compare_ymfm(&session.instrument, &request)?;
                    println!("{:?}: {}", profile, serde_json::to_string(&report)?);
                    if report.spectral_cosine_full < 0.98 || report.envelope_relative_error > 0.1 {
                        return Err("independent ymfm sine comparison exceeds tolerance".into());
                    }
                }
                println!(
                    "ymfm audio behavior verified for this fixture; physical hardware recordings remain unverified."
                );
            }
        },
    }
    Ok(())
}
/// Set in the editor process started by [`supervise`].
const SUPERVISED: &str = "DISCOFM_SUPERVISED";

pub fn run() -> ExitCode {
    discofm_log::install_panic_hook();
    run_parsed(Cli::parse(), false)
}
/// Entry point of `discofm-gui.exe`, which has no console window. Arguments
/// are optional files to open: a session (.json) and/or a reference.
pub fn run_editor() -> ExitCode {
    discofm_log::install_panic_hook();
    let (mut session, mut reference) = (None, None);
    for path in std::env::args_os().skip(1).map(PathBuf::from) {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            session = Some(path);
        } else {
            reference = Some(path);
        }
    }
    run_parsed(
        Cli {
            command: Some(Command::Gui {
                session,
                reference,
                sample: None,
                smoke_screenshot: None,
            }),
        },
        true,
    )
}
/// `dialog`: without a console, report a startup failure in a message box.
fn run_parsed(cli: Cli, dialog: bool) -> ExitCode {
    let gui = match &cli.command {
        None => true,
        Some(Command::Gui {
            smoke_screenshot, ..
        }) => smoke_screenshot.is_none(),
        Some(_) => false,
    };
    if gui && let Some(code) = supervise() {
        return code;
    }
    discofm_log::info(
        "start",
        format!(
            "DiscoFM {} pid {}: {}",
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            std::env::args().collect::<Vec<_>>().join(" ")
        ),
    );
    match execute(cli) {
        Ok(()) => {
            discofm_log::info("exit", "normal exit");
            ExitCode::SUCCESS
        }
        Err(error) => {
            discofm_log::error("exit", &error);
            eprintln!("DiscoFM: {error}");
            eprintln!("Log: {}", discofm_log::log_path().display());
            if dialog {
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("DiscoFM")
                    .set_description(format!(
                        "{error}

Log: {}",
                        discofm_log::log_path().display()
                    ))
                    .show();
            }
            ExitCode::FAILURE
        }
    }
}

/// Runs the editor in a child process and records abnormal exits. A stack
/// overflow, an access violation in native code or an abort (release builds
/// abort on panic) ends a process without running any more of its code, so
/// only an observer outside it can report that the window closed and why.
/// Returns `None` when this process should run the editor itself.
fn supervise() -> Option<ExitCode> {
    if std::env::var_os(SUPERVISED).is_some() || std::env::var_os("DISCOFM_NO_SUPERVISOR").is_some()
    {
        return None;
    }
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            discofm_log::warn("supervisor", format!("running unsupervised: {error}"));
            return None;
        }
    };
    let started = std::time::Instant::now();
    let status = match std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(SUPERVISED, "1")
        .status()
    {
        Ok(status) => status,
        Err(error) => {
            discofm_log::warn(
                "supervisor",
                format!("cannot start the editor process, running unsupervised: {error}"),
            );
            return None;
        }
    };
    let code = status.code().unwrap_or(-1);
    // 1 is an error the editor already reported; Ctrl+C or closing the console is deliberate.
    if code == 0 || code == 1 || code as u32 == 0xC000_013A {
        return Some(if code == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let meaning = discofm_log::describe_exit_code(code);
    let seconds = started.elapsed().as_secs_f64();
    discofm_log::error(
        "supervisor",
        format!(
            "editor closed unexpectedly: exit 0x{:08X} ({meaning}) after {seconds:.0} s",
            code as u32
        ),
    );
    let report = format!(
        "DiscoFM closed unexpectedly
Time (UTC): {}
{}Exit status: 0x{:08X} ({meaning})
Running time: {seconds:.0} s

A panic, if any, is recorded just above in the log with its location;
a separate crash-*.txt written at the same time holds its backtrace.

Recent log:
{}
",
        discofm_log::timestamp(),
        discofm_log::environment(),
        code as u32,
        discofm_log::tail(120)
    );
    let record = discofm_log::crash_report(&report);
    let location = record
        .as_ref()
        .map_or_else(discofm_log::log_path, Clone::clone);
    eprintln!(
        "DiscoFM closed unexpectedly ({meaning}). Report: {}",
        location.display()
    );
    let answer = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("DiscoFM closed unexpectedly")
        .set_description(format!(
            "The editor stopped: {meaning} (exit 0x{:08X}).

A report was saved to:
{}

Please send this file and discofm.log from the same folder to the developer.

Open the log folder now?",
            code as u32,
            location.display()
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if answer == rfd::MessageDialogResult::Yes
        && let Err(error) = discofm_log::reveal()
    {
        discofm_log::warn("supervisor", format!("cannot open the log folder: {error}"));
    }
    Some(ExitCode::FAILURE)
}
