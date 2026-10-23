//! Application log and crash records the user can find and send to the
//! developer. Files live in `logs` next to the executable; when that folder
//! is not writable (for example under Program Files) they go to
//! `%LOCALAPPDATA%\DiscoFM\logs`, then to the temporary directory.
//! Logging never panics and never fails the caller: a lost line is better
//! than a second crash while reporting the first.
#![forbid(unsafe_code)]
use std::{
    fmt::Display,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

const FILE: &str = "discofm.log";
const PREVIOUS: &str = "discofm.previous.log";
/// The active log is rotated once it exceeds this size.
const LIMIT: u64 = 4 * 1024 * 1024;

static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static WRITER: Mutex<()> = Mutex::new(());

fn writable(directory: &Path) -> bool {
    fs::create_dir_all(directory).is_ok()
        && OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join(FILE))
            .is_ok()
}

/// Folder holding `discofm.log` and `crash-*.txt` records.
pub fn directory() -> &'static Path {
    DIRECTORY.get_or_init(|| {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("logs")));
        let local = std::env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join("DiscoFM").join("logs"));
        let temp = std::env::temp_dir().join("DiscoFM").join("logs");
        beside
            .into_iter()
            .chain(local)
            .find(|dir| writable(dir))
            .unwrap_or(temp)
    })
}

/// Full path of the active application log.
pub fn log_path() -> PathBuf {
    directory().join(FILE)
}

/// UTC wall-clock time as `YYYY-MM-DD HH:MM:SS.mmm`, without a date library.
pub fn timestamp() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = elapsed.as_secs();
    let (year, month, day) = civil(seconds / 86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        elapsed.subsec_millis()
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Hinnant's algorithm).
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

fn append(level: &str, context: &str, message: &str) {
    let _guard = WRITER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = log_path();
    if fs::metadata(&path).is_ok_and(|m| m.len() > LIMIT) {
        let _ = fs::rename(&path, directory().join(PREVIOUS));
    }
    let thread = std::thread::current();
    let line = format!(
        "{} {level:<5} [{}] {context}: {}\n",
        timestamp(),
        thread.name().unwrap_or("unnamed"),
        message.replace('\n', "\n    ")
    );
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Records normal activity, so the last lines before a crash show what ran.
pub fn info(context: &str, message: impl Display) {
    append("INFO", context, &message.to_string());
}
pub fn warn(context: &str, message: impl Display) {
    append("WARN", context, &message.to_string());
}
pub fn error(context: &str, message: impl Display) {
    append("ERROR", context, &message.to_string());
}

/// Shows the log folder in the system file manager.
pub fn reveal() -> std::io::Result<()> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(directory())
        .spawn()
        .map(drop)
}

/// Last `lines` lines of the active log (empty when it cannot be read).
pub fn tail(lines: usize) -> String {
    let text = fs::read_to_string(log_path()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Writes a new `crash-<time>.txt` record and returns its path.
pub fn crash_report(text: &str) -> Option<PathBuf> {
    let stamp = timestamp().replace([' ', ':'], "-").replace('.', "-");
    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("crash-{stamp}.txt")
        } else {
            format!("crash-{stamp}-{attempt}.txt")
        };
        let path = directory().join(name);
        if let Ok(mut file) = File::create_new(&path) {
            let _ = file.write_all(text.as_bytes());
            return Some(path);
        }
    }
    None
}

/// Header shared by crash records: version, platform and command line.
pub fn environment() -> String {
    format!(
        "DiscoFM {} ({} {})\nCommand line: {}\nLog: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::env::args().collect::<Vec<_>>().join(" "),
        log_path().display()
    )
}

/// Logs every panic with its location, thread and backtrace, and writes a
/// crash record. Release builds abort on panic, so this runs before the
/// process ends; the previous hook still prints to the console afterwards.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-text panic payload".into());
        let location = info
            .location()
            .map_or_else(|| "unknown location".into(), |l| l.to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        error("panic", format!("{message} at {location}"));
        let report = format!(
            "DiscoFM crashed (panic)\nTime (UTC): {}\n{}Thread: {}\nMessage: {message}\nLocation: {location}\n\nBacktrace:\n{backtrace}\n\nRecent log:\n{}\n",
            timestamp(),
            environment(),
            std::thread::current().name().unwrap_or("unnamed"),
            tail(80)
        );
        if let Some(path) = crash_report(&report) {
            error("panic", format!("crash record {}", path.display()));
            eprintln!("DiscoFM crash record: {}", path.display());
        }
        previous(info);
    }));
}

/// Meaning of a Windows process exit status, for crash records.
pub fn describe_exit_code(code: i32) -> &'static str {
    match code as u32 {
        0 => "normal exit",
        1 => "error reported by the application",
        3 => "abort() in native code",
        101 => "Rust panic (development build)",
        0x8000_0003 => "breakpoint (debug assertion in native code)",
        0xC000_0005 => "access violation (invalid memory access, usually native code)",
        0xC000_0017 | 0xC000_012D => "out of memory",
        0xC000_001D => "illegal instruction",
        0xC000_00FD => "stack overflow",
        0xC000_0374 => "heap corruption",
        0xC000_0409 => {
            "fast-fail abort (Rust panic with panic=abort, failed allocation, or security check)"
        }
        0xC000_013A => "terminated by Ctrl+C or console close",
        0xC000_0142 => "DLL initialization failed",
        _ => "unexpected exit status",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn civil_dates_and_exit_codes() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(20_748), (2026, 10, 22));
        assert_eq!(timestamp().len(), 23);
        assert_eq!(describe_exit_code(0xC00000FD_u32 as i32), "stack overflow");
        assert!(describe_exit_code(0xC0000409_u32 as i32).contains("panic"));
    }
}
