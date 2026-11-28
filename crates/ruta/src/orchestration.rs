use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thiserror::Error;

use crate::app_start::start_app;
use crate::cleanup;
use crate::command::Sandbox;
use crate::config::{Config, check_name};
use crate::display::{Session, display_session, require_weston_version};
use crate::display_start::{
    clean_recorded_display_files, display_up, ensure_display, recorded_headed,
};
use crate::environment::{caller_environment, sandbox_environment};
use crate::identity::stop::{stop_recorded, stop_window};
use crate::identity::{RECORD_TIMEOUT, error_is_survivor, record_matches, record_xwayland};
use crate::input::{focus_window, send, window_exists, window_id};
use crate::lock::{self, LockMode};
use crate::log_tail::{last_lines, open_app_log};
use crate::root::{
    install_openers, prepare_root, record, record_root_creation, remove_root, require_trusted_root,
    root_path,
};
use crate::tool::{Tool, require_program};

use one_shot::{refuse_capture_output_in_root, run_one_shot};

mod one_shot;
mod status;

#[cfg(test)]
use one_shot::{CaptureOutputInsideRoot, finish_capture};

const DISPLAY_MARGIN: u32 = 200;

pub(crate) fn execute(
    config: &Config,
    command: Sandbox,
    emit: &mut impl FnMut(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let name = &config.name;
    let root = root_path(name);
    let capture_name = format!("{name}-capture");

    if matches!(&command, Sandbox::Capture { .. }) {
        check_name("RUTA_NAME", &capture_name)?;
    }

    let _lock = match command_lock(&command, name, &capture_name) {
        Some((lock_name, mode)) => Some(lock::take(lock_name, mode)?),
        None => None,
    };

    let output = match command {
        Sandbox::Up { arguments, headed } => up(config, &root, &arguments, headed),
        Sandbox::Restart {
            arguments,
            force_headed,
        } => restart(config, &root, arguments, force_headed),
        Sandbox::Capture { out, arguments } => {
            return capture(
                config,
                &capture_name,
                &root_path(&capture_name),
                &out,
                &arguments,
                emit,
            );
        }
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
        Sandbox::Hold { keys } => with_window(name, &root, |display, window| {
            send::hold(&root, display, window, &keys)
        }),
        Sandbox::Release { keys } => with_window(name, &root, |display, window| {
            send::release(&root, display, window, &keys)
        }),
        Sandbox::Status => status::status(config, &root, recorded_window_exists),
        Sandbox::Log => log(&root),
        Sandbox::Opened => opened(&root),
        Sandbox::Down => down(name, &root),
    }?;

    if !output.is_empty() {
        emit(&output)?;
    }

    Ok(())
}

fn command_lock<'a>(
    command: &Sandbox,
    name: &'a str,
    capture_name: &'a str,
) -> Option<(&'a str, LockMode)> {
    match command {
        Sandbox::Up { .. } | Sandbox::Restart { .. } | Sandbox::Down => {
            Some((name, LockMode::Exclusive))
        }
        Sandbox::Capture { .. } => Some((capture_name, LockMode::Exclusive)),
        Sandbox::Status
        | Sandbox::Shot { .. }
        | Sandbox::Move { .. }
        | Sandbox::Click { .. }
        | Sandbox::Drag { .. }
        | Sandbox::Type { .. }
        | Sandbox::Key { .. }
        | Sandbox::Scroll { .. }
        | Sandbox::Hold { .. }
        | Sandbox::Release { .. } => Some((name, LockMode::Shared)),
        Sandbox::Log | Sandbox::Opened => None,
    }
}

fn capture(
    config: &Config,
    name: &str,
    root: &Path,
    out: &Path,
    arguments: &[String],
    emit: &mut impl FnMut(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    run_one_shot(root, name, out, || {
        capture_inner(config, root, out, arguments, emit)
    })
}

fn capture_inner(
    config: &Config,
    root: &Path,
    out: &Path,
    arguments: &[String],
    emit: &mut impl FnMut(&str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    record_root_creation(root)?;
    prepare_capture_folder(out)?;

    let resolved_root = fs::canonicalize(root).context("Could not resolve the capture root")?;
    let resolved_out =
        fs::canonicalize(out).context("Could not resolve the capture output folder")?;
    refuse_capture_output_in_root(&resolved_root, &resolved_out)?;
    require_display_tools()?;
    require_program(Tool::Import)?;

    let output = start_window(config, root, arguments, &Session::Headless)?;

    emit(&output)?;

    capture_window(&config.name, root, out)
}

fn prepare_capture_folder(folder: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(folder).with_context(|| format!("Could not create {}", folder.display()))?;

    for name in [crate::root::record::SHOT, "crop.png", "capture.json"] {
        let path = folder.join(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("Could not remove {}", path.display()));
            }
        }
    }

    Ok(())
}

fn capture_window(name: &str, root: &Path, out: &Path) -> anyhow::Result<()> {
    let display = require_up(name, root, recorded_window_exists)?;
    let window = window_id(root)?;
    let environment = sandbox_environment(&caller_environment(), root, &display);
    let ready_line = match fs::read_to_string(root.join(crate::root::record::READY)) {
        Ok(line) => Some(line),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("Could not read the saved ready line"),
    };

    crate::capture::import_window(
        &display,
        &window,
        &out.join(crate::root::record::SHOT),
        &environment,
    )?;
    crate::capture::write_capture(out, ready_line.as_deref(), &environment)
}

#[mutants::skip]
fn up(config: &Config, root: &Path, arguments: &[String], headed: bool) -> anyhow::Result<String> {
    let session = caller_session(headed)?;

    start_window(config, root, arguments, &session)
}

#[mutants::skip]
fn restart(
    config: &Config,
    root: &Path,
    arguments: Option<Vec<String>>,
    force_headed: bool,
) -> anyhow::Result<String> {
    let (arguments, headed) = restart_plan(root, arguments, force_headed)?;
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
    require_program(Tool::Weston)?;
    require_program(Tool::Xwayland)?;
    require_program(Tool::Xdotool)?;
    require_program(Tool::Cat)?;

    Ok(())
}

fn restart_plan(
    root: &Path,
    arguments: Option<Vec<String>>,
    force_headed: bool,
) -> anyhow::Result<(Vec<String>, bool)> {
    require_trusted_root(root)?;

    let headed = force_headed || recorded_headed(root);
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => recorded_arguments(root)?,
    };

    Ok((arguments, headed))
}

fn recorded_arguments(root: &Path) -> anyhow::Result<Vec<String>> {
    let recorded = fs::read_to_string(root.join(crate::root::record::ARGUMENTS))
        .map_err(|_| SandboxError::RestartNotUp)?;

    serde_json::from_str(&recorded).with_context(|| "Could not read the recorded arguments")
}

fn start_window(
    config: &Config,
    root: &Path,
    arguments: &[String],
    session: &Session,
) -> anyhow::Result<String> {
    // The probe runs before start_window_with stages the folder record, so a refused weston leaves
    // a running sandbox and its records untouched.
    require_trusted_root(root)?;
    let environment = session.environment(&caller_environment(), root);
    require_weston_version(&environment)?;
    require_display_tools()?;

    start_window_with(root, &config.folder, || {
        start_sandbox(config, root, arguments, session)
    })
}

fn start_window_with(
    root: &Path,
    folder: &Path,
    start: impl FnOnce() -> anyhow::Result<String>,
) -> anyhow::Result<String> {
    prepare_root(root)?;
    let staged = crate::root::stage_folder(root, folder)?;
    let output = start().map_err(|error| failed_start(root, error, stop_target))?;

    crate::root::publish_folder(root, &staged)
        .map_err(|error| failed_start(root, error, stop_target))?;

    Ok(output)
}

fn failed_start<F>(root: &Path, error: anyhow::Error, stop: F) -> anyhow::Error
where
    F: FnMut(&Path, StopTarget) -> anyhow::Result<()>,
{
    failed_start_with(root, error, stop, cleanup::report)
}

fn failed_start_with<F, R>(
    root: &Path,
    error: anyhow::Error,
    stop: F,
    mut report: R,
) -> anyhow::Error
where
    F: FnMut(&Path, StopTarget) -> anyhow::Result<()>,
    R: FnMut(&str) -> std::io::Result<()>,
{
    // The cleanup removes the app log, so its end goes into the error first. A refused root is not
    // read, since its log may be anything, such as a FIFO.
    let log_tail = match require_trusted_root(root) {
        Ok(()) => last_lines(&root.join(crate::root::record::APP_LOG), LOG_TAIL_LINES),
        Err(_) => Vec::new(),
    };
    let error = if log_tail.is_empty() {
        error
    } else {
        FailedStart { error, log_tail }.into()
    };

    if let Err(cleanup_error) = cleanup(root, stop, &mut report) {
        let warning = format!("Could not clean up after the failed start: {cleanup_error:#}");

        return cleanup::with_report_failure(error, report(&warning));
    }

    error
}

fn start_sandbox(
    config: &Config,
    root: &Path,
    arguments: &[String],
    session: &Session,
) -> anyhow::Result<String> {
    let name = &config.name;
    let size = (config.app.width, config.app.height);

    prepare_root(root)?;
    stop_window(root)?;

    let display = ensure_display(name, root, session, display_size(size))?;

    install_openers(root)?;
    start_app(root, config, arguments, &display)?;
    focus_window(root, &display, &config.app.title, size)?;
    record_xwayland(root, &display, RECORD_TIMEOUT)?;

    Ok(format!(
        "ruta {name} is up on {display} (log: {})\n",
        root.join(crate::root::record::APP_LOG).display()
    ))
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

fn with_window<F>(name: &str, root: &Path, action: F) -> anyhow::Result<String>
where
    F: FnOnce(&str, &str) -> anyhow::Result<()>,
{
    require_program(Tool::Xdotool)?;

    let display = require_up(name, root, recorded_window_exists)?;
    let window = window_id(root)?;

    action(&display, &window)?;

    Ok(String::new())
}

#[derive(Debug, Error, PartialEq, Eq)]
enum SandboxError {
    #[error("ruta {name} is not up; run 'ruta up' first")]
    NotUp { name: String },
    #[error("The sandbox is not up; run 'ruta up' first.")]
    RestartNotUp,
}

fn require_up(
    name: &str,
    root: &Path,
    window_exists: impl FnOnce(&Path, &str) -> anyhow::Result<bool>,
) -> anyhow::Result<String> {
    let display = up_display(root, window_exists)?.ok_or_else(|| SandboxError::NotUp {
        name: name.to_string(),
    })?;

    Ok(display)
}

fn up_display(
    root: &Path,
    window_exists: impl FnOnce(&Path, &str) -> anyhow::Result<bool>,
) -> anyhow::Result<Option<String>> {
    require_trusted_root(root)?;

    let Some(display) = display_up(root) else {
        return Ok(None);
    };

    if !root.join(crate::root::record::WINDOW).is_file() {
        return Ok(None);
    }

    // The window record outlives the app, so only the display can say that the window is there.
    if !window_exists(root, &display)? {
        return Ok(None);
    }

    Ok(Some(display))
}

#[mutants::skip]
fn recorded_window_exists(root: &Path, display: &str) -> anyhow::Result<bool> {
    require_program(Tool::Xdotool)?;

    window_exists(root, display)
}

#[mutants::skip]
fn shot(name: &str, root: &Path, file: Option<PathBuf>) -> anyhow::Result<String> {
    require_program(Tool::Import)?;

    let display = require_up(name, root, recorded_window_exists)?;
    let window = window_id(root)?;
    let path = file.unwrap_or_else(|| root.join(crate::root::record::SHOT));
    let environment = sandbox_environment(&caller_environment(), root, &display);

    crate::capture::import_window(&display, &window, &path, &environment)?;
    Ok(format!("{}\n", path.display()))
}

fn log(root: &Path) -> anyhow::Result<String> {
    require_trusted_root(root)?;

    let mut file = open_app_log(&root.join(crate::root::record::APP_LOG))
        .with_context(|| "Could not read the app log")?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| "Could not read the app log")?;

    Ok(String::from_utf8_lossy_owned(bytes))
}

fn opened(root: &Path) -> anyhow::Result<String> {
    require_trusted_root(root)?;

    match fs::read_to_string(root.join(crate::root::record::OPENED)) {
        Ok(opened) => Ok(opened),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| "Could not read the opened links"),
    }
}

fn down(name: &str, root: &Path) -> anyhow::Result<String> {
    if !root.exists() {
        return Ok(format!("ruta {name} is not up\n"));
    }

    cleanup(root, stop_target, cleanup::report)?;

    Ok(format!("ruta {name} is gone\n"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopTarget {
    Window,
    Xwayland,
    Weston,
}

fn stop_target(root: &Path, target: StopTarget) -> anyhow::Result<()> {
    match target {
        StopTarget::Window => stop_window(root),
        StopTarget::Xwayland => stop_recorded(&root.join(crate::root::record::XWAYLAND_PID)),
        StopTarget::Weston => stop_recorded(&root.join(crate::root::record::WESTON_PID)),
    }
}

fn cleanup<F, R>(root: &Path, mut stop: F, mut report: R) -> anyhow::Result<()>
where
    F: FnMut(&Path, StopTarget) -> anyhow::Result<()>,
    R: FnMut(&str) -> std::io::Result<()>,
{
    require_trusted_root(root)?;

    let mut failure: Option<anyhow::Error> = None;
    let mut survivor = false;

    for target in [StopTarget::Window, StopTarget::Xwayland, StopTarget::Weston] {
        match stop(root, target) {
            Ok(()) => {}
            Err(error) => {
                survivor |= error_is_survivor(&error);
                failure.get_or_insert(error);
            }
        }
    }

    // A report failure and a failed record unlink leave no process alive, so only a survivor or a
    // live record keeps the root and its display files.
    let xwayland_alive = record_matches(&root.join(crate::root::record::XWAYLAND_PID));
    let all_stopped = !survivor && !recorded_process_alive(root);

    if !xwayland_alive && let Err(error) = clean_recorded_display_files(root, &mut report) {
        failure.get_or_insert(error.into());
    }

    let removed = if all_stopped && root.exists() {
        remove_root(root)
    } else {
        Ok(())
    };

    match (failure, removed) {
        (Some(error), Err(remove_error)) => {
            Err(error.context(format!("Could not remove the root: {remove_error:#}")))
        }
        (Some(error), Ok(())) => Err(error),
        (None, removed) => removed,
    }
}

fn recorded_process_alive(root: &Path) -> bool {
    [record::PID, record::XWAYLAND_PID, record::WESTON_PID]
        .into_iter()
        .any(|name| record_matches(&root.join(name)))
}

#[cfg(test)]
mod tests;
