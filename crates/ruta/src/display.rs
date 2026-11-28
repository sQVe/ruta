use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rustix::fs::OFlags;
use thiserror::Error;

use crate::cleanup;
use crate::environment::{nested_weston_environment, weston_environment};
use crate::process::{TOOL_TIMEOUT, poll_until, run_with_timeout, sandboxed_command};
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
        .arg("--no-xwm-decorations")
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

const MINIMUM_WESTON: u32 = 15;

#[derive(Debug, Error, PartialEq, Eq)]
enum WestonVersionError {
    #[error(
        "The installed weston is version {found}. ruta needs weston {minimum} or newer, because a \
         sandbox window must hold the X input focus."
    )]
    TooOld { found: String, minimum: u32 },
    #[error("Could not read the weston version from \"{answer}\".")]
    Unreadable { answer: String },
}

pub(crate) fn require_weston_version(environment: &[(String, String)]) -> anyhow::Result<()> {
    let mut command = sandboxed_command(Tool::Weston.name(), environment);
    command.arg("--version");

    let label = format!("{} --version", Tool::Weston.name());
    let output = run_with_timeout(command, TOOL_TIMEOUT)?;

    if !output.status.success() {
        anyhow::bail!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let answer = String::from_utf8_lossy(&output.stdout);

    Ok(weston_version(&answer)?)
}

fn weston_version(answer: &str) -> Result<(), WestonVersionError> {
    let answer = answer.trim();
    let Some(version) = answer.strip_prefix("weston ") else {
        return Err(WestonVersionError::Unreadable {
            answer: answer.to_string(),
        });
    };
    let major = version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok());

    match major {
        Some(major) if major >= MINIMUM_WESTON => Ok(()),
        Some(_) => Err(WestonVersionError::TooOld {
            found: version.to_string(),
            minimum: MINIMUM_WESTON,
        }),
        None => Err(WestonVersionError::Unreadable {
            answer: answer.to_string(),
        }),
    }
}

pub(crate) fn clean_display_files(display: &str) -> std::io::Result<()> {
    clean_display_files_with(display, &mut cleanup::report)
}

pub(crate) fn clean_display_files_with(
    display: &str,
    report: &mut impl FnMut(&str) -> std::io::Result<()>,
) -> std::io::Result<()> {
    clean_display_files_in(Path::new("/tmp"), display, DISPLAY_CLEANUP_TIMEOUT, report)
}

pub(crate) fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn clean_display_files_in(
    folder: &Path,
    display: &str,
    timeout: Duration,
    report: &mut impl FnMut(&str) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let Some(number) = display.strip_prefix(':') else {
        return Ok(());
    };

    if !is_decimal(number) {
        return Ok(());
    }

    let lock = folder.join(format!(".X{number}-lock"));
    let socket = folder.join(format!(".X11-unix/X{number}"));
    let owner = lock_owner(&lock);

    // A lock that is there but cannot be read may still name a live owner, so only an absent lock
    // lets the cleanup go on.
    if owner == LockOwner::Unknown {
        return Ok(());
    }

    if let LockOwner::Owner(pid) = owner {
        let stopped = poll_until(timeout, || Ok((!lock_owner_is_alive(pid)).then_some(())))
            .expect("the wait for the lock owner cannot fail");

        if stopped.is_none() {
            return Ok(());
        }
    }

    if lock_owner(&lock) != owner {
        return Ok(());
    }

    cleanup::remove_file_with(report, &lock).and(cleanup::remove_file_with(report, &socket))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockOwner {
    Absent,
    Unknown,
    Owner(u32),
}

fn lock_owner(lock: &Path) -> LockOwner {
    // Open without following a link or blocking on a FIFO, then check the opened handle, so the
    // type check and the read use the same object.
    let fd = match rustix::fs::open(
        lock,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(error) if error == rustix::io::Errno::NOENT => return LockOwner::Absent,
        Err(_) => return LockOwner::Unknown,
    };

    let is_file = rustix::fs::fstat(&fd)
        .is_ok_and(|stat| rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file());

    if !is_file {
        return LockOwner::Unknown;
    }

    let mut contents = String::new();
    if fs::File::from(fd).read_to_string(&mut contents).is_err() {
        return LockOwner::Unknown;
    }

    match contents.trim().parse::<i32>() {
        Ok(owner) if owner > 0 => LockOwner::Owner(owner.unsigned_abs()),
        _ => LockOwner::Unknown,
    }
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
