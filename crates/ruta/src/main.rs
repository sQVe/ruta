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
mod tool;

use std::process::ExitCode;

use anyhow::Context as _;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => failure(&error),
    }
}

fn failure(error: &anyhow::Error) -> ExitCode {
    let warning = format!("{error:#}");

    match cleanup::report(&warning) {
        Ok(()) => ExitCode::FAILURE,
        // A broken stderr has no stream left to report the write on; the exit status still fails.
        Err(_) => ExitCode::FAILURE,
    }
}

#[expect(
    clippy::print_stdout,
    reason = "the guide and version lines are the command's output"
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
    let mut stdout = std::io::stdout().lock();

    orchestration::execute(&config, command, &mut |text| {
        write_output(&mut stdout, text)
    })
}

fn write_output(writer: &mut impl std::io::Write, text: &str) -> anyhow::Result<()> {
    writer
        .write_all(text.as_bytes())
        .and_then(|()| writer.flush())
        .context("Could not write the output")
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
    fn a_failed_output_write_is_an_error() {
        let error = write_output(&mut crate::test_support::FailingWriter, "ruta test is up\n")
            .expect_err("the output cannot be written");

        assert_eq!(error.to_string(), "Could not write the output");
    }

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
