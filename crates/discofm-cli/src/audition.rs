use crate::{Clock, Profile};
use clap::Args;
use crossterm::{
    event::{
        self, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute, terminal,
};
use discofm_audio::{DeviceOptions, Playback};
use discofm_chip::Value;
use discofm_engine::{ChipProfile, EngineError, LiveConfig};
use discofm_formats::{Session, read_session};
use std::{
    collections::HashMap,
    error::Error,
    fs::File,
    io::{self, BufRead, IsTerminal},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Args)]
pub(crate) struct PlayArgs {
    #[arg(long)]
    session: Option<PathBuf>,
    /// Stable output ID from `devices`; omit for the system default.
    #[arg(long)]
    device: Option<String>,
    #[arg(long)]
    sample_rate: Option<u32>,
    /// Maximum prepared PCM lead, excluding device buffers and filter lookahead.
    #[arg(long, default_value_t = 40)]
    queue_ms: u32,
    #[arg(long,default_value_t=4,allow_hyphen_values=true,value_parser=clap::value_parser!(i8).range(-1..=8))]
    octave: i8,
    #[arg(long,value_parser=clap::value_parser!(u8).range(0..=127))]
    velocity: Option<u8>,
    #[arg(long, value_enum)]
    profile: Option<Profile>,
    #[arg(long, value_enum)]
    clock: Option<Clock>,
    /// Read line commands from stdin instead of terminal key events.
    #[arg(long)]
    commands: bool,
}
const HELP: &str = "Piano: Z S X D C V G B H N J M , (C..C). Enter: A. Release key: note-off.\nSpace: release. Esc: Stop. [ / ]: octave. Tab: YM2612/YM3438. R: reload session. Q/Ctrl+C: quit.";

pub(crate) fn run(args: PlayArgs) -> Result<(), Box<dyn Error>> {
    let session = match &args.session {
        Some(path) => read_session(File::open(path)?)?,
        None => Session::default(),
    };
    let config = LiveConfig {
        channel: session.audition.channel,
        profile: args.profile.map_or(session.audition.profile, Into::into),
        clock: args.clock.map_or(session.audition.clock, Into::into),
        tuning_cents: session.audition.tuning_cents,
    };
    if config.profile == ChipProfile::RawDiagnostic {
        return Err(EngineError::UnsupportedRawProfile.into());
    }
    let options = DeviceOptions {
        device_id: args.device,
        sample_rate: args.sample_rate,
        queue_ms: args.queue_ms,
    };
    options.validate()?;
    if !args.commands && !io::stdin().is_terminal() {
        return Err(
            "keyboard playback requires an interactive terminal; use --commands for stdin".into(),
        );
    }
    let mut playback = Playback::open(session.instrument, config, options)?;
    playback.controls().set_octave(args.octave)?;
    playback.controls().set_velocity(match args.velocity {
        Some(raw) => Value::new(raw)?,
        None => session.audition.velocity,
    });
    println!("Playback: {}", serde_json::to_string(playback.info())?);
    if args.commands {
        command_loop(&mut playback, args.session.as_ref())?;
    } else {
        keyboard_loop(&mut playback, args.session.as_ref())?;
    }
    playback.controls().stop();
    println!(
        "Playback stats: {}",
        serde_json::to_string(&playback.stats())?
    );
    Ok(())
}
struct RawTerminal {
    enhanced: bool,
}
impl RawTerminal {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let enhanced = !cfg!(windows) && terminal::supports_keyboard_enhancement().unwrap_or(false);
        let guard = Self { enhanced };
        execute!(io::stdout(), EnableFocusChange)?;
        if enhanced {
            execute!(
                io::stdout(),
                event::PushKeyboardEnhancementFlags(
                    event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | event::KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                        | event::KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                )
            )?;
        } else if !cfg!(windows) {
            println!(
                "This terminal has no key-release events; press Space to release or Esc to Stop.\r"
            );
        }
        Ok(guard)
    }
}
impl Drop for RawTerminal {
    fn drop(&mut self) {
        if self.enhanced {
            let _ = execute!(io::stdout(), event::PopKeyboardEnhancementFlags);
        }
        let _ = execute!(io::stdout(), DisableFocusChange);
        let _ = terminal::disable_raw_mode();
    }
}
fn toggle(playback: &mut Playback) -> Result<(), Box<dyn Error>> {
    let profile = match playback.controls().profile() {
        ChipProfile::Ym2612Discrete => ChipProfile::Ym3438,
        _ => ChipProfile::Ym2612Discrete,
    };
    playback.controls().set_profile(profile)?;
    println!("Profile: {profile:?}\r");
    Ok(())
}
fn reload(playback: &mut Playback, path: Option<&PathBuf>) -> Result<(), Box<dyn Error>> {
    let path = path.ok_or("reload requires playback started with --session")?;
    let session = read_session(File::open(path)?)?;
    playback.controls().set_instrument(session.instrument)?;
    println!("Session reloaded\r");
    Ok(())
}
fn keyboard_offset(code: KeyCode) -> Option<u8> {
    match code {
        KeyCode::Char(character) => "zsxdcvgbhnjm,"
            .find(character.to_ascii_lowercase())
            .map(|index| index as u8),
        KeyCode::Enter => Some(9),
        _ => None,
    }
}
fn normalized(code: KeyCode) -> KeyCode {
    match code {
        KeyCode::Char(character) => KeyCode::Char(character.to_ascii_lowercase()),
        _ => code,
    }
}
fn keyboard_loop(playback: &mut Playback, path: Option<&PathBuf>) -> Result<(), Box<dyn Error>> {
    println!("{HELP}");
    let _terminal = RawTerminal::enter()?;
    let mut held = HashMap::new();
    loop {
        playback.check_health()?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::FocusLost => {
                playback.controls().stop();
                held.clear();
            }
            Event::Key(key) => {
                let code = normalized(key.code);
                if key.kind == KeyEventKind::Release {
                    if let Some(id) = held.remove(&code) {
                        playback.controls().note_off(id);
                    }
                    continue;
                }
                if key.kind == KeyEventKind::Repeat {
                    continue;
                }
                if code == KeyCode::Char('q')
                    || (code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    break;
                }
                let result: Result<(), Box<dyn Error>> = match code {
                    KeyCode::Esc => {
                        playback.controls().stop();
                        held.clear();
                        Ok(())
                    }
                    KeyCode::Char(' ') => {
                        playback.controls().release();
                        held.clear();
                        Ok(())
                    }
                    KeyCode::Tab => toggle(playback),
                    KeyCode::Char('r') => reload(playback, path),
                    KeyCode::Char('[') | KeyCode::Char(']') => {
                        let octave = playback.controls().octave()
                            + if code == KeyCode::Char('[') { -1 } else { 1 };
                        playback
                            .controls()
                            .set_octave(octave)
                            .map(|()| println!("Octave: {octave}\r"))
                            .map_err(Into::into)
                    }
                    _ => {
                        if let Some(offset) = keyboard_offset(code) {
                            if let std::collections::hash_map::Entry::Vacant(entry) =
                                held.entry(code)
                            {
                                playback
                                    .controls()
                                    .key_on(offset)
                                    .map(|id| {
                                        entry.insert(id);
                                    })
                                    .map_err(Into::into)
                            } else {
                                Ok(())
                            }
                        } else {
                            Ok(())
                        }
                    }
                };
                if let Err(error) = result {
                    eprintln!("{error}\r");
                }
            }
            _ => {}
        }
    }
    Ok(())
}
#[derive(Debug, PartialEq)]
enum Action {
    On(u8),
    Key(u8),
    Off,
    Stop,
    Octave(i8),
    Profile(ChipProfile),
    Toggle,
    Reload,
    Wait(u64),
    Status,
    Quit,
}
fn parse(line: &str) -> Result<Option<Action>, Box<dyn Error>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let words = line.split_whitespace().collect::<Vec<_>>();
    let action=match words.as_slice() {
        ["on",note]=>match note.parse::<u8>() {Ok(note)=>Action::On(note),Err(_)=>{
            let index=["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"].iter().position(|name|name.eq_ignore_ascii_case(note)).ok_or("note must be MIDI 0..127 or C/C#/D/.../B")?;
            Action::Key(index as u8)
        }},
        ["off"]=>Action::Off,["stop"]=>Action::Stop,["octave",octave]=>Action::Octave(octave.parse()?),
        ["profile","ym2612"]=>Action::Profile(ChipProfile::Ym2612Discrete),["profile","ym3438"]=>Action::Profile(ChipProfile::Ym3438),
        ["profile","toggle"]=>Action::Toggle,["reload"]=>Action::Reload,["wait",millis]=>{let millis=millis.parse::<u64>()?;if millis>5000 {return Err("wait must be 0..5000 ms".into());}Action::Wait(millis)},
        ["status"]=>Action::Status,["quit"]=>Action::Quit,
        _=>return Err("commands: on <MIDI|name>, off, stop, octave <-1..8>, profile <ym2612|ym3438|toggle>, reload, wait <0..5000 ms>, status, quit".into()),
    };
    Ok(Some(action))
}
fn command_loop(playback: &mut Playback, path: Option<&PathBuf>) -> Result<(), Box<dyn Error>> {
    let (sender, receiver) = mpsc::sync_channel(32);
    thread::Builder::new()
        .name("discofm-stdin".into())
        .spawn(move || {
            for line in io::stdin().lock().lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        })?;
    let mut waiting = None;
    loop {
        playback.check_health()?;
        if waiting.is_some_and(|deadline| Instant::now() < deadline) {
            thread::sleep(Duration::from_millis(10));
            continue;
        }
        waiting = None;
        let line = match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => line?,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let Some(action) = parse(&line)? else {
            continue;
        };
        match action {
            Action::On(note) => {
                playback.controls().trigger(note)?;
            }
            Action::Key(offset) => {
                playback.controls().key_on(offset)?;
            }
            Action::Off => playback.controls().release(),
            Action::Stop => playback.controls().stop(),
            Action::Octave(octave) => {
                playback.controls().set_octave(octave)?;
                println!("Octave: {octave}");
            }
            Action::Profile(profile) => {
                playback.controls().set_profile(profile)?;
                println!("Profile: {profile:?}");
            }
            Action::Toggle => toggle(playback)?,
            Action::Reload => reload(playback, path)?,
            Action::Wait(millis) => waiting = Some(Instant::now() + Duration::from_millis(millis)),
            Action::Status => println!(
                "Playback stats: {}",
                serde_json::to_string(&playback.stats())?
            ),
            Action::Quit => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_and_line_commands_select_notes_octaves_profiles_and_release() {
        for (index, key) in "zsxdcvgbhnjm,".chars().enumerate() {
            assert_eq!(keyboard_offset(KeyCode::Char(key)), Some(index as u8));
        }
        assert_eq!(parse("on C#").unwrap(), Some(Action::Key(1)));
        assert_eq!(parse("on 69").unwrap(), Some(Action::On(69)));
        assert_eq!(parse("octave -1").unwrap(), Some(Action::Octave(-1)));
        assert_eq!(
            parse("profile ym3438").unwrap(),
            Some(Action::Profile(ChipProfile::Ym3438))
        );
        assert_eq!(parse("off").unwrap(), Some(Action::Off));
        assert_eq!(parse("stop").unwrap(), Some(Action::Stop));
        assert!(parse("wait 5001").is_err());
        assert!(parse("on H").is_err());
        assert!(parse("off extra").is_err());
        assert_eq!(parse("# ignore").unwrap(), None);
    }
}
