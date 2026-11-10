use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use thiserror::Error;

use crate::environment::ROOT_MARKER;
use crate::process::poll_until;
use crate::process_stat::{ProcessStat, command_line};

pub(crate) mod stop;

pub(crate) const RECORD_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) started: u64,
}

impl ProcessIdentity {
    pub(crate) fn current(pid: u32) -> Option<Self> {
        let stat = ProcessStat::read(pid)?;

        Some(Self {
            pid,
            started: stat.start,
        })
    }

    pub(crate) fn read(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        let mut fields = text.split_whitespace();

        Some(Self {
            pid: fields.next()?.parse().ok()?,
            started: fields.next()?.parse().ok()?,
        })
    }

    pub(crate) fn write(self, path: &Path) -> std::io::Result<()> {
        fs::write(path, format!("{} {}\n", self.pid, self.started))
    }

    // Read again after opening a pidfd so a reused PID is detected before sending a signal.
    pub(crate) fn is_current(self) -> bool {
        Self::current(self.pid) == Some(self)
    }
}

#[derive(Debug, Error)]
pub(crate) enum IdentityError {
    #[error("The process {pid} did not stop; keeping {}", path.display())]
    DidNotStop { pid: u32, path: PathBuf },
    #[error("The process {pid} is not the expected {expected}. Actual: {actual}")]
    UnexpectedProcess {
        pid: u32,
        expected: String,
        actual: String,
    },
    #[error("The {expected} process {pid} is gone before it could be recorded.")]
    MissingProcess { pid: u32, expected: String },
    #[error("The {expected} process {pid} did not name itself in time.")]
    DidNotNameItself { pid: u32, expected: String },
    #[error("The app process {pid} does not hold this sandbox's {ROOT_MARKER}.")]
    Unmarked { pid: u32 },
    #[error(
        "Xwayland for {display} did not start within {} seconds.",
        timeout.as_secs()
    )]
    XwaylandNotRecorded { display: String, timeout: Duration },
}

pub(crate) fn record_xwayland(root: &Path, display: &str, timeout: Duration) -> anyhow::Result<()> {
    let Some(weston) = ProcessIdentity::read(&root.join(crate::root::record::WESTON_PID)) else {
        return Ok(());
    };

    let recorded = poll_until(timeout, || {
        for pid in child_pids(weston.pid) {
            let Ok(cmdline) = command_line(pid) else {
                continue;
            };

            if cmdline.contains("Xwayland") && cmdline.contains(display) {
                record_process(
                    &root.join(crate::root::record::XWAYLAND_PID),
                    pid,
                    "Xwayland",
                    timeout,
                )?;
                return Ok(Some(()));
            }
        }

        Ok(None)
    })?;

    recorded.ok_or_else(|| {
        IdentityError::XwaylandNotRecorded {
            display: display.to_string(),
            timeout,
        }
        .into()
    })
}

fn child_pids(parent: u32) -> Vec<u32> {
    let Ok(entries) = crate::process_stat::process_entries() else {
        return Vec::new();
    };

    entries
        .map(|(pid, _)| pid)
        .filter(|pid| ProcessStat::read(*pid).is_some_and(|stat| stat.parent == parent))
        .collect()
}

#[cfg(test)]
pub(crate) fn process_is_zombie(pid: u32) -> bool {
    ProcessStat::read(pid).is_some_and(|stat| stat.is_zombie())
}

pub(crate) fn record_marked_process(
    path: &Path,
    pid: u32,
    root: &Path,
    timeout: Duration,
) -> anyhow::Result<()> {
    let recorded = poll_until(timeout, || {
        let Some(stat) = ProcessStat::read_live(pid) else {
            return Err(IdentityError::MissingProcess {
                pid,
                expected: "app".to_string(),
            }
            .into());
        };

        if !process_is_ours(pid, root) {
            return Ok(None);
        }

        ProcessIdentity {
            pid,
            started: stat.start,
        }
        .write(path)
        .with_context(|| "Could not record the app process")?;

        Ok(Some(()))
    })?;

    recorded.ok_or_else(|| IdentityError::Unmarked { pid }.into())
}

pub(crate) fn record_process(
    path: &Path,
    pid: u32,
    expected: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let mut mismatch = None;
    let recorded = poll_until(timeout, || {
        let Some(identity) = ProcessIdentity::current(pid) else {
            return Err(IdentityError::MissingProcess {
                pid,
                expected: expected.to_string(),
            }
            .into());
        };

        let cmdline =
            command_line(pid).with_context(|| format!("Could not read the process {pid}"))?;

        if cmdline.contains(expected) {
            identity
                .write(path)
                .with_context(|| format!("Could not record the {expected} process"))?;

            return Ok(Some(()));
        }

        if !cmdline.trim().is_empty() {
            mismatch = Some(cmdline);
        }

        Ok(None)
    })?;

    if recorded.is_none() {
        return Err(match mismatch {
            Some(actual) => IdentityError::UnexpectedProcess {
                pid,
                expected: expected.to_string(),
                actual,
            }
            .into(),
            None => IdentityError::DidNotNameItself {
                pid,
                expected: expected.to_string(),
            }
            .into(),
        });
    }

    Ok(())
}

pub(crate) fn record_matches(path: &Path) -> bool {
    ProcessIdentity::read(path).is_some_and(ProcessIdentity::is_current)
}

fn process_is_ours(pid: u32, root: &Path) -> bool {
    fs::read(format!("/proc/{pid}/environ"))
        .is_ok_and(|environment| environment_holds_marker(&environment, root))
}

// The kernel separates environment entries with NUL bytes only. A value may hold a newline, so
// only a whole entry counts as the marker.
fn environment_holds_marker(environment: &[u8], root: &Path) -> bool {
    let marker = format!("{ROOT_MARKER}={}", root.display());

    environment
        .split(|byte| *byte == 0)
        .any(|entry| entry == marker.as_bytes())
}

#[cfg(test)]
pub(crate) mod tests;
