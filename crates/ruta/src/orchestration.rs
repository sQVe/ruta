use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use thiserror::Error;

use crate::app_start::start_app;
use crate::command::{Sandbox, require_program};
use crate::config::Config;
use crate::display;
use crate::display_start::{
    Session, clean_display_files, display_session, display_up, ensure_display, recorded_mode,
};
use crate::environment::{caller_environment, sandbox_environment};
use crate::identity::{STOP_TIMEOUT, record_xwayland, stop_recorded, stop_window};
use crate::input::{focus_window, send, window_id};
use crate::log_tail::last_lines;
use crate::root::{
    install_openers, prepare_root, refuse_linked_paths, refuse_unsafe_root, remove_root,
    require_trusted_root, root_path,
};

const DISPLAY_MARGIN: u32 = 200;

#[expect(
    clippy::print_stderr,
    reason = "failure messages are the command's output"
)]
pub(crate) fn run(config: &Config, command: Sandbox) -> ExitCode {
    match execute(config, command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn execute(config: &Config, command: Sandbox) -> anyhow::Result<()> {
    let name = &config.name;
    let root = root_path(name);

    match command {
        Sandbox::Up { arguments, headed } => up(config, &root, &arguments, headed),
        Sandbox::Restart { arguments, headed } => restart(config, &root, arguments, headed),
        Sandbox::Shot { file } => shot(name, &root, file),
        Sandbox::Move { from, to } => with_window(name, &root, |display, window| {
            send::glide(&root, display, window, from, to)
        }),
        Sandbox::Click { x, y, button } => with_window(name, &root, |display, window| {
            send::click(&root, display, window, (x, y), button)
        }),
        Sandbox::Drag { from, to } => with_window(name, &root, |display, window| {
            send::drag(&root, display, window, from, to)
        }),
        Sandbox::Type { text } => with_window(name, &root, |display, window| {
            send::type_text(&root, display, window, &text)
        }),
        Sandbox::Key { key, modifiers } => with_window(name, &root, |display, window| {
            send::key(&root, display, window, &key, &modifiers)
        }),
        Sandbox::Scroll { x, y, notches } => with_window(name, &root, |display, window| {
            send::scroll(&root, display, window, (x, y), notches)
        }),
        Sandbox::Hold { keys } => {
            with_window(name, &root, |display, _| send::hold(&root, display, &keys))
        }
        Sandbox::Release { keys } => with_window(name, &root, |display, _| {
            send::release(&root, display, &keys)
        }),
        Sandbox::Log => log(&root),
        Sandbox::Opened => opened(&root),
        Sandbox::Down => down(name, &root),
    }
}

#[mutants::skip]
fn up(config: &Config, root: &Path, arguments: &[String], headed: bool) -> anyhow::Result<()> {
    require_display_tools()?;
    let session = caller_session(headed)?;

    start_window(config, root, arguments, &session)
}

#[mutants::skip]
fn restart(
    config: &Config,
    root: &Path,
    arguments: Option<Vec<String>>,
    headed: Option<bool>,
) -> anyhow::Result<()> {
    require_display_tools()?;

    let (arguments, headed) = restart_plan(root, arguments, headed)?;
    let session = caller_session(headed)?;

    start_window(config, root, &arguments, &session)
}

fn caller_session(headed: bool) -> anyhow::Result<Session> {
    let session = display_session(
        headed,
        std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
    )?;

    Ok(session)
}

fn require_display_tools() -> anyhow::Result<()> {
    require_program("weston")?;
    require_program("Xwayland")?;
    require_program("xdotool")?;
    require_program("cat")?;

    Ok(())
}

fn restart_plan(
    root: &Path,
    arguments: Option<Vec<String>>,
    headed: Option<bool>,
) -> anyhow::Result<(Vec<String>, bool)> {
    require_trusted_root(root)?;

    let headed = headed.unwrap_or_else(|| recorded_mode(root).as_deref() == Some("headed"));
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => recorded_arguments(root)?,
    };

    Ok((arguments, headed))
}

fn recorded_arguments(root: &Path) -> anyhow::Result<Vec<String>> {
    let recorded = fs::read_to_string(root.join("arguments"))
        .map_err(|_| anyhow::anyhow!("The sandbox is not up; run 'ruta up' first."))?;

    serde_json::from_str(&recorded).with_context(|| "Could not read the recorded arguments")
}

fn start_window(
    config: &Config,
    root: &Path,
    arguments: &[String],
    session: &Session,
) -> anyhow::Result<()> {
    let started = start_window_inner(config, root, arguments, session);

    clean_up_failed_start(root, started, stop_target)
}

#[expect(
    clippy::print_stderr,
    reason = "a cleanup failure after a failed start must not hide the start error"
)]
fn clean_up_failed_start<F>(root: &Path, started: anyhow::Result<()>, stop: F) -> anyhow::Result<()>
where
    F: FnMut(StopTarget) -> anyhow::Result<()>,
{
    let Err(error) = started else {
        return Ok(());
    };

    // The cleanup removes the app log, so its end goes into the error first. A refused root is not
    // read, since its log may be anything, such as a FIFO.
    let log_tail = match require_trusted_root(root) {
        Ok(()) => last_lines(&root.join("app.log"), LOG_TAIL_LINES),
        Err(_) => Vec::new(),
    };

    if let Err(cleanup_error) = cleanup(root, stop) {
        eprintln!("Could not clean up after the failed start: {cleanup_error:#}");
    }

    if log_tail.is_empty() {
        return Err(error);
    }

    Err(FailedStart { error, log_tail }.into())
}

#[expect(
    clippy::print_stdout,
    reason = "the display and log path are the command's output"
)]
fn start_window_inner(
    config: &Config,
    root: &Path,
    arguments: &[String],
    session: &Session,
) -> anyhow::Result<()> {
    let name = &config.name;
    let size = (config.app.width, config.app.height);

    prepare_root(root)?;

    let display = ensure_display(name, root, session, display_size(size))?;

    install_openers(root)?;
    start_app(root, config, arguments, &display)?;
    focus_window(root, &display, &config.app.title, size)?;
    record_xwayland(root, &display, STOP_TIMEOUT)?;

    println!(
        "ruta {name} is up on {display} (log: {})",
        root.join("app.log").display()
    );

    Ok(())
}

fn display_size((width, height): (u32, u32)) -> (u32, u32) {
    (
        width.saturating_add(DISPLAY_MARGIN),
        height.saturating_add(DISPLAY_MARGIN),
    )
}

const LOG_TAIL_LINES: usize = 20;

#[derive(Debug, Error)]
#[error("{error:#}\n\nThe app log ended with:\n{}", log_tail.join("\n"))]
struct FailedStart {
    error: anyhow::Error,
    log_tail: Vec<String>,
}

fn with_window<F>(name: &str, root: &Path, action: F) -> anyhow::Result<()>
where
    F: FnOnce(&str, &str) -> anyhow::Result<()>,
{
    require_program("xdotool")?;

    let display = require_up(name, root)?;
    let window = window_id(root)?;

    action(&display, &window)
}

#[derive(Debug, Error, PartialEq, Eq)]
enum SandboxError {
    #[error("ruta {name} is not up; run 'ruta up' first")]
    NotUp { name: String },
}

fn require_up(name: &str, root: &Path) -> anyhow::Result<String> {
    require_trusted_root(root)?;

    let not_up = || SandboxError::NotUp {
        name: name.to_string(),
    };
    let display = display_up(root).ok_or_else(not_up)?;

    if !root.join("window").is_file() {
        return Err(not_up().into());
    }

    Ok(display)
}

#[mutants::skip]
#[expect(clippy::print_stdout, reason = "the shot path is the command's output")]
fn shot(name: &str, root: &Path, file: Option<PathBuf>) -> anyhow::Result<()> {
    require_program("import")?;

    let display = require_up(name, root)?;
    let window = window_id(root)?;
    let path = file.unwrap_or_else(|| root.join("shot.png"));
    let environment = sandbox_environment(&caller_environment(), root, &display);

    display::import_window(&display, &window, &path, &environment)?;
    println!("{}", path.display());

    Ok(())
}

#[expect(clippy::print_stdout, reason = "the app log is the command's output")]
fn log(root: &Path) -> anyhow::Result<()> {
    require_trusted_root(root)?;

    let log =
        fs::read_to_string(root.join("app.log")).with_context(|| "Could not read the app log")?;

    print!("{log}");

    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "the opened links are the command's output"
)]
fn opened(root: &Path) -> anyhow::Result<()> {
    require_trusted_root(root)?;

    match fs::read_to_string(root.join("opened")) {
        Ok(opened) => print!("{opened}"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| "Could not read the opened links");
        }
    }

    Ok(())
}

#[expect(
    clippy::print_stdout,
    reason = "the sandbox state is the command's output"
)]
fn down(name: &str, root: &Path) -> anyhow::Result<()> {
    if !root.exists() {
        println!("ruta {name} is not up");
        return Ok(());
    }

    refuse_unsafe_root(root)?;
    refuse_linked_paths(root)?;
    cleanup(root, stop_target)?;
    println!("ruta {name} is gone");

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum StopTarget {
    Window { root: PathBuf },
    Recorded { path: PathBuf },
}

fn stop_target(target: StopTarget) -> anyhow::Result<()> {
    match target {
        StopTarget::Window { root } => stop_window(&root),
        StopTarget::Recorded { path } => stop_recorded(&path),
    }
}

fn cleanup<F>(root: &Path, mut stop: F) -> anyhow::Result<()>
where
    F: FnMut(StopTarget) -> anyhow::Result<()>,
{
    require_trusted_root(root)?;

    stop(StopTarget::Window {
        root: root.to_path_buf(),
    })?;
    stop(StopTarget::Recorded {
        path: root.join("xwayland.pid"),
    })?;
    stop(StopTarget::Recorded {
        path: root.join("weston.pid"),
    })?;
    clean_display_files(root);

    if root.exists() {
        remove_root(root)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
