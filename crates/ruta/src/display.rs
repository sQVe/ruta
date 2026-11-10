use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use thiserror::Error;

use crate::cleanup;
use crate::process::{TOOL_TIMEOUT, poll_until, run_with_timeout};
use crate::process_stat::ProcessStat;

pub(crate) const WESTON_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Error)]
enum DisplayError {
    #[error("weston did not print a display within {} seconds.", timeout.as_secs())]
    DisplayTimeout {
        timeout: Duration,
        #[allow(
            dead_code,
            reason = "the timeout is the message contract; the log path is test-visible data"
        )]
        log: PathBuf,
    },
}

const DISPLAY_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn clean_display_files(display: &str) {
    clean_display_files_in(Path::new("/tmp"), display, DISPLAY_CLEANUP_TIMEOUT);
}

fn clean_display_files_in(folder: &Path, display: &str, timeout: Duration) {
    let Some(number) = display.strip_prefix(':') else {
        return;
    };

    if number.is_empty() || !number.chars().all(|character| character.is_ascii_digit()) {
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

fn base_weston_command(
    socket: &str,
    width: u32,
    height: u32,
    log: &Path,
    environment: &[(String, String)],
) -> Command {
    let mut command = Command::new(crate::tool::Tool::Weston.name());

    command
        .arg("--xwayland")
        .arg(format!("--socket={socket}"))
        .arg(format!("--width={width}"))
        .arg(format!("--height={height}"))
        .arg(format!("--log={}", log.display()))
        .env_clear()
        .envs(environment.iter().cloned());

    command
}

pub(crate) fn weston_command(
    socket: &str,
    width: u32,
    height: u32,
    log: &Path,
    environment: &[(String, String)],
) -> Command {
    let mut command = base_weston_command(socket, width, height, log, environment);

    command.arg("--backend=headless");

    command
}

pub(crate) fn nested_weston_command(
    socket: &str,
    width: u32,
    height: u32,
    log: &Path,
    wayland_display: &str,
    environment: &[(String, String)],
) -> Command {
    let mut command = base_weston_command(socket, width, height, log, environment);

    command
        .arg("--backend=wayland")
        .env("WAYLAND_DISPLAY", wayland_display);

    command
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

    display.ok_or_else(|| {
        DisplayError::DisplayTimeout {
            timeout,
            log: log_path.to_path_buf(),
        }
        .into()
    })
}

#[mutants::skip]
pub(crate) fn import_window(
    display: &str,
    window: &str,
    path: &Path,
    environment: &[(String, String)],
) -> anyhow::Result<()> {
    let mut command = Command::new(crate::tool::Tool::Import.name());

    command
        .args(["-display", display, "-window", window])
        .arg(path)
        .env_clear()
        .envs(environment.iter().cloned());

    let output = run_with_timeout(command, TOOL_TIMEOUT)?;

    if !output.status.success() {
        anyhow::bail!(
            "import failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests;
