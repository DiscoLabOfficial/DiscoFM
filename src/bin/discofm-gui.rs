//! The editor without a console window. Errors and crashes still reach
//! logs/discofm.log and the supervisor's dialog; `discofm.exe` is the CLI.
#![windows_subsystem = "windows"]
#![forbid(unsafe_code)]

fn main() -> std::process::ExitCode {
    discofm_cli::run_editor()
}
