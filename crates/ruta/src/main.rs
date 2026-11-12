mod app_start;
mod capture;
mod cleanup;
mod command;
mod config;
mod display;
mod display_start;
mod environment;
mod identity;
mod input;
mod log_tail;
mod orchestration;
mod process;
mod process_stat;
mod root;
#[cfg(test)]
mod test_support;
#[cfg(test)]
use test_support::namespace;
mod tool;

use std::process::ExitCode;

use anyhow::Context as _;

#[expect(
    clippy::print_stderr,
    reason = "usage and failure messages are the command's output"
)]
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

#[expect(
    clippy::print_stdout,
    reason = "main owns command output, including capture startup progress"
)]
fn run() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = match command::parse(&arguments)? {
        command::Command::Guide => {
            println!("{}", command::GUIDE);

            return Ok(());
        }
        command::Command::Version => {
            println!("ruta {}", env!("CARGO_PKG_VERSION"));

            return Ok(());
        }
        command::Command::Sandbox(command) => command,
    };
    let config = load_config()?;

    // Capture's startup line needs its newline so stdout flushes before capture continues.
    orchestration::execute(&config, command, &mut |text| print!("{text}"))
}

fn load_config() -> anyhow::Result<config::Config> {
    let folder = std::env::current_dir().context("Could not read the current folder")?;
    let name = std::env::var("RUTA_NAME").ok();

    config::load(&folder, name.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_error_format_keeps_every_parse_error_text() {
        for error in [
            command::ParseError::Usage,
            command::ParseError::NoNotches,
            command::ParseError::NotANumber {
                argument: "x",
                value: "word".to_string(),
            },
        ] {
            let expected = error.to_string();
            let error = anyhow::Error::from(error);

            assert_eq!(format!("{error:#}"), expected);
        }
    }
}
