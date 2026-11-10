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

#[expect(
    clippy::print_stderr,
    reason = "usage and failure messages are the command's output"
)]
fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    let command = match command::parse(&arguments) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    match load_config() {
        Ok(config) => orchestration::run(&config, command),
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn load_config() -> anyhow::Result<config::Config> {
    let folder = std::env::current_dir().context("Could not read the current folder")?;
    let name = std::env::var("RUTA_NAME").ok();

    config::load(&folder, name.as_deref())
}
