use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use thiserror::Error;

use crate::cleanup;
use crate::environment::{nested_weston_environment, weston_environment};
use crate::process::{poll_until, sandboxed_command};
use crate::process_stat::ProcessStat;
use crate::tool::Tool;

pub(crate) const WESTON_TIMEOUT: Duration = Duration::from_secs(30);
const DISPLAY_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Headless,
    Headed,
}

impl Mode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Headed => "headed",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Session {
    Headless,
    Headed {
        wayland_display: String,
        runtime: PathBuf,
    },
}

impl Session {
    pub(crate) fn mode(&self) -> Mode {
        match self {
            Self::Headless => Mode::Headless,
            Self::Headed { .. } => Mode::Headed,
        }
    }

    pub(crate) fn environment(
        &self,
        caller: &[(String, String)],
        root: &Path,
    ) -> Vec<(String, String)> {
        match self {
            Self::Headless => weston_environment(caller, root),
            Self::Headed { runtime, .. } => nested_weston_environment(caller, root, runtime),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum HeadedSessionError {
    #[error(
        "Headed mode needs WAYLAND_DISPLAY. Run it from the Wayland session, or leave out --headed."
    )]
    NoWaylandDisplay,
    #[error("Headed mode needs an absolute XDG_RUNTIME_DIR to reach the Wayland socket.")]
    NoAbsoluteRuntimeDir,
}

pub(crate) fn display_session(
    headed: bool,
    wayland: Option<&str>,
    runtime: Option<&str>,
) -> Result<Session, HeadedSessionError> {
    if !headed {
        return Ok(Session::Headless);
    }

    Ok(Session::Headed {
        wayland_display: wayland_display(wayland)?,
        runtime: caller_runtime_dir(runtime)?,
    })
}

fn wayland_display(value: Option<&str>) -> Result<String, HeadedSessionError> {
    match value {
        Some(display) if !display.trim().is_empty() => Ok(display.to_string()),
        _ => Err(HeadedSessionError::NoWaylandDisplay),
    }
}

fn caller_runtime_dir(value: Option<&str>) -> Result<PathBuf, HeadedSessionError> {
    match value {
        Some(directory) if directory.starts_with('/') => Ok(PathBuf::from(directory)),
        _ => Err(HeadedSessionError::NoAbsoluteRuntimeDir),
    }
}

pub(crate) fn weston_command(
    session: &Session,
    socket: &str,
    (width, height): (u32, u32),
    log: &Path,
    environment: &[(String, String)],
) -> Command {
    let mut command = sandboxed_command(Tool::Weston.name(), environment);

    command
        .arg("--xwayland")
        .arg(format!("--socket={socket}"))
        .arg(format!("--width={width}"))
        .arg(format!("--height={height}"))
        .arg(format!("--log={}", log.display()));

    match session {
        Session::Headless => command.arg("--backend=headless"),
        Session::Headed {
            wayland_display, ..
        } => command
            .arg("--backend=wayland")
            .env("WAYLAND_DISPLAY", wayland_display),
    };

    command
}

pub(crate) fn clean_display_files(display: &str) {
    clean_display_files_in(Path::new("/tmp"), display, DISPLAY_CLEANUP_TIMEOUT);
}

pub(crate) fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn clean_display_files_in(folder: &Path, display: &str, timeout: Duration) {
    let Some(number) = display.strip_prefix(':') else {
        return;
    };

    if !is_decimal(number) {
        return;
    }

    let lock = folder.join(format!(".X{number}-lock"));
    let socket = folder.join(format!(".X11-unix/X{number}"));
    let owner = lock_owner(&lock);

    if let Some(pid) = owner {
        let stopped = poll_until(timeout, || Ok((!lock_owner_is_alive(pid)).then_some(())))
            .expect("the wait for the lock owner cannot fail");

        if stopped.is_none() {
            return;
        }
    }

    if lock_owner(&lock) != owner {
        return;
    }

    cleanup::remove_file(&lock);
    cleanup::remove_file(&socket);
}

fn lock_owner(lock: &Path) -> Option<u32> {
    fs::read_to_string(lock).ok()?.trim().parse().ok()
}

fn lock_owner_is_alive(pid: u32) -> bool {
    ProcessStat::read_live(pid).is_some()
}

#[derive(Debug, Error)]
enum DisplayError {
    #[error("weston did not print a display within {} seconds.", timeout.as_secs())]
    DisplayTimeout { timeout: Duration },
}

pub(crate) fn display_from_log(log: &str) -> Option<String> {
    let marker = "xserver listening on display ";

    log.lines().find_map(|line| {
        let rest = line.split_once(marker)?.1;
        let display: String = rest
            .chars()
            .take_while(|character| !character.is_whitespace())
            .collect();

        display.starts_with(':').then_some(display)
    })
}

pub(crate) fn wait_for_display(log_path: &Path, timeout: Duration) -> anyhow::Result<String> {
    let display = poll_until(timeout, || {
        if let Ok(log) = fs::read_to_string(log_path)
            && let Some(display) = display_from_log(&log)
        {
            return Ok(Some(display));
        }

        Ok(None)
    })?;

    display.ok_or_else(|| DisplayError::DisplayTimeout { timeout }.into())
}

#[cfg(test)]
mod tests;
