use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thiserror::Error;

use crate::app_start::start_app;
use crate::command::Sandbox;
use crate::config::Config;
use crate::display::{Session, display_session};
use crate::display_start::{
    clean_recorded_display_files, display_up, ensure_display, recorded_headed,
};
use crate::environment::{caller_environment, sandbox_environment};
use crate::identity::stop::{stop_recorded, stop_window};
use crate::identity::{RECORD_TIMEOUT, record_xwayland};
use crate::input::{focus_window, send, window_id};
use crate::log_tail::last_lines;
use crate::root::{
    create_root, install_openers, prepare_root, record_folder, record_root_creation, remove_root,
    require_trusted_root, root_path,
};
use crate::tool::{Tool, require_program};

mod status;

const DISPLAY_MARGIN: u32 = 200;

pub(crate) fn execute(
    config: &Config,
    command: Sandbox,
    emit: &mut impl FnMut(&str),
) -> anyhow::Result<()> {
    let name = &config.name;
    let root = root_path(name);

    let output = match command {
        Sandbox::Up { arguments, headed } => up(config, &root, &arguments, headed),
        Sandbox::Restart {
            arguments,
            force_headed,
        } => restart(config, &root, arguments, force_headed),
        Sandbox::Capture { out, arguments } => {
            return capture(
                config,
                &root_path(&format!("{name}-capture")),
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
        Sandbox::Hold { keys } => {
            with_window(name, &root, |display, _| send::hold(&root, display, &keys))
        }
        Sandbox::Release { keys } => with_window(name, &root, |display, _| {
            send::release(&root, display, &keys)
        }),
        Sandbox::Status => status::status(config, &root),
        Sandbox::Log => log(&root),
        Sandbox::Opened => opened(&root),
        Sandbox::Down => down(name, &root),
    }?;

    if !output.is_empty() {
        emit(&output);
    }

    Ok(())
}

fn capture(
    config: &Config,
    root: &Path,
    out: &Path,
    arguments: &[String],
    emit: &mut impl FnMut(&str),
) -> anyhow::Result<()> {
    let name = format!("{}-capture", config.name);
    crate::config::check_name("RUTA_NAME", &name)?;
    refuse_capture_output_in_root(root, out)?;

    match create_root(root) {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => anyhow::bail!(
            "The capture root already exists; run 'RUTA_NAME={name} ruta down' first."
        ),
        Ok(()) => {}
        Err(error) => return Err(error).context("Could not create the capture root"),
    }

    let captured = capture_inner(config, root, out, arguments, emit);

    finish_capture(root, captured, stop_target)
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("The capture output folder {} must not be the capture root {} or inside it.", out.display(), root.display())]
struct CaptureOutputInsideRoot {
    out: PathBuf,
    root: PathBuf,
}

fn refuse_capture_output_in_root(root: &Path, out: &Path) -> anyhow::Result<()> {
    let root = lexical_absolute_path(root)?;
    let out = lexical_absolute_path(out)?;

    if out.starts_with(&root) {
        return Err(CaptureOutputInsideRoot { out, root }.into());
    }

    Ok(())
}

fn lexical_absolute_path(path: &Path) -> anyhow::Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .with_context(|| format!("Could not make {} absolute", path.display()))?;
    let mut normalized = PathBuf::new();

    for component in absolute.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component),
        }
    }

    Ok(normalized)
}

fn capture_inner(
    config: &Config,
    root: &Path,
    out: &Path,
    arguments: &[String],
    emit: &mut impl FnMut(&str),
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

    emit(&output);

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
    let display = require_up(name, root)?;
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

#[expect(
    clippy::print_stderr,
    reason = "cleanup must not hide the capture error"
)]
fn finish_capture<F>(root: &Path, captured: anyhow::Result<()>, stop: F) -> anyhow::Result<()>
where
    F: FnMut(&Path, StopTarget) -> anyhow::Result<()>,
{
    let cleaned = cleanup(root, stop);

    if let Err(error) = captured {
        if let Err(cleanup_error) = cleaned {
            eprintln!("Could not clean up after capture: {cleanup_error:#}");
        }

        return Err(error);
    }

    cleaned
}

#[mutants::skip]
fn up(config: &Config, root: &Path, arguments: &[String], headed: bool) -> anyhow::Result<String> {
    require_display_tools()?;
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
    require_display_tools()?;

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
    start_sandbox(config, root, arguments, session)
        .map_err(|error| failed_start(root, error, stop_target))
}

#[expect(
    clippy::print_stderr,
    reason = "a cleanup failure after a failed start must not hide the start error"
)]
fn failed_start<F>(root: &Path, error: anyhow::Error, stop: F) -> anyhow::Error
where
    F: FnMut(&Path, StopTarget) -> anyhow::Result<()>,
{
    // The cleanup removes the app log, so its end goes into the error first. A refused root is not
    // read, since its log may be anything, such as a FIFO.
    let log_tail = match require_trusted_root(root) {
        Ok(()) => last_lines(&root.join(crate::root::record::APP_LOG), LOG_TAIL_LINES),
        Err(_) => Vec::new(),
    };

    if let Err(cleanup_error) = cleanup(root, stop) {
        eprintln!("Could not clean up after the failed start: {cleanup_error:#}");
    }

    if log_tail.is_empty() {
        return error;
    }

    FailedStart { error, log_tail }.into()
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
    record_folder(root, &config.folder)?;

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

    let display = require_up(name, root)?;
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

fn require_up(name: &str, root: &Path) -> anyhow::Result<String> {
    let display = up_display(root)?.ok_or_else(|| SandboxError::NotUp {
        name: name.to_string(),
    })?;

    Ok(display)
}

fn up_display(root: &Path) -> anyhow::Result<Option<String>> {
    require_trusted_root(root)?;

    let has_window = root.join(crate::root::record::WINDOW).is_file();

    Ok(display_up(root).filter(|_| has_window))
}

#[mutants::skip]
fn shot(name: &str, root: &Path, file: Option<PathBuf>) -> anyhow::Result<String> {
    require_program(Tool::Import)?;

    let display = require_up(name, root)?;
    let window = window_id(root)?;
    let path = file.unwrap_or_else(|| root.join(crate::root::record::SHOT));
    let environment = sandbox_environment(&caller_environment(), root, &display);

    crate::capture::import_window(&display, &window, &path, &environment)?;
    Ok(format!("{}\n", path.display()))
}

fn log(root: &Path) -> anyhow::Result<String> {
    require_trusted_root(root)?;

    fs::read_to_string(root.join(crate::root::record::APP_LOG))
        .with_context(|| "Could not read the app log")
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

    cleanup(root, stop_target)?;

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

fn cleanup<F>(root: &Path, mut stop: F) -> anyhow::Result<()>
where
    F: FnMut(&Path, StopTarget) -> anyhow::Result<()>,
{
    require_trusted_root(root)?;

    for target in [StopTarget::Window, StopTarget::Xwayland, StopTarget::Weston] {
        stop(root, target)?;
    }
    clean_recorded_display_files(root);

    if root.exists() {
        remove_root(root)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
