use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use thiserror::Error;

use crate::cleanup;
use crate::environment::ROOT_MARKER;
use crate::process::poll_until;
use crate::process_stat::ProcessStat;
use crate::root::root_created;

pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(5);

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
    let Some((weston, _)) = read_record(&root.join("weston.pid")) else {
        return Ok(());
    };

    let recorded = poll_until(timeout, || {
        for pid in child_pids(weston) {
            let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
                continue;
            };
            let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");

            if cmdline.contains("Xwayland") && cmdline.contains(display) {
                record_process(&root.join("xwayland.pid"), pid, "Xwayland", timeout)?;
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
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
        })
        .filter(|pid| ProcessStat::read(*pid).is_some_and(|stat| stat.parent == parent))
        .collect()
}

pub(crate) fn process_is_zombie(pid: u32) -> bool {
    ProcessStat::read(pid).is_some_and(|stat| stat.is_zombie())
}

pub(crate) fn process_start(pid: u32) -> Option<u64> {
    ProcessStat::read(pid).map(|stat| stat.start)
}

// The recorded identity still names the same process. Read again after the pidfd is open so a
// reused PID is detected before any signal is sent.
fn identity_still_matches(pid: u32, started: u64) -> bool {
    process_start(pid) == Some(started)
}

pub(crate) fn record_marked_process(
    path: &Path,
    pid: u32,
    root: &Path,
    timeout: Duration,
) -> anyhow::Result<()> {
    let recorded = poll_until(timeout, || {
        let live = ProcessStat::read(pid).filter(|stat| !stat.is_zombie());
        let Some(started) = live.map(|stat| stat.start) else {
            return Err(IdentityError::MissingProcess {
                pid,
                expected: "app".to_string(),
            }
            .into());
        };

        if !process_is_ours(pid, root) {
            return Ok(None);
        }

        fs::write(path, format!("{pid} {started}\n"))
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
        let Some(started) = process_start(pid) else {
            return Err(IdentityError::MissingProcess {
                pid,
                expected: expected.to_string(),
            }
            .into());
        };

        let cmdline = fs::read(format!("/proc/{pid}/cmdline"))
            .with_context(|| format!("Could not read the process {pid}"))?;
        let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");

        if cmdline.contains(expected) {
            fs::write(path, format!("{pid} {started}\n"))
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

fn read_record(path: &Path) -> Option<(u32, u64)> {
    let text = fs::read_to_string(path).ok()?;
    let mut fields = text.split_whitespace();

    Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}

pub(crate) fn record_matches(path: &Path) -> bool {
    let Some((pid, started)) = read_record(path) else {
        return false;
    };

    process_start(pid) == Some(started)
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

pub(crate) fn stop_window(root: &Path) -> anyhow::Result<()> {
    stop_group(root, STOP_TIMEOUT)?;
    stop_marked(root, std::process::id(), STOP_TIMEOUT)
}

fn stop_group(root: &Path, timeout: Duration) -> anyhow::Result<()> {
    let path = root.join("pid");

    let Some(leader) = read_record(&path) else {
        cleanup::remove_file(&path);
        return Ok(());
    };

    if stop_all(|| group_members(leader, root), timeout)?.is_some() {
        return Err(IdentityError::DidNotStop {
            pid: leader.0,
            path,
        }
        .into());
    }

    cleanup::remove_file(&path);

    Ok(())
}

// A process can leave the app's group with `setsid`, such as a daemon that the app or a ready
// check starts. It still holds this root's marker, so the marker alone finds it.
fn stop_marked(root: &Path, ruta: u32, timeout: Duration) -> anyhow::Result<()> {
    let Some(created) = root_created(root) else {
        return Ok(());
    };
    let spared = self_and_ancestors(ruta);

    match stop_all(|| marked_processes(root, created, &spared), timeout)? {
        Some(pid) => Err(IdentityError::DidNotStop {
            pid,
            path: root.to_path_buf(),
        }
        .into()),
        None => Ok(()),
    }
}

// Sends TERM, then KILL after `timeout`, until `members` finds no process. Returns a process that
// is still alive after both waits.
fn stop_all(
    members: impl Fn() -> Vec<(u32, u64)>,
    timeout: Duration,
) -> anyhow::Result<Option<u32>> {
    if members().is_empty() {
        return Ok(None);
    }

    signal_members(&members(), rustix::process::Signal::TERM);

    if poll_until(timeout, || Ok(members().is_empty().then_some(())))?.is_some() {
        return Ok(None);
    }

    let killed = poll_until(timeout, || {
        let alive = members();
        signal_members(&alive, rustix::process::Signal::KILL);

        Ok(alive.is_empty().then_some(()))
    })?;

    if killed.is_some() {
        return Ok(None);
    }

    Ok(members().first().map(|&(pid, _)| pid))
}

// The live processes of the current user that hold this root's marker and started after the root
// was made, with their start times.
fn marked_processes(root: &Path, created: u64, spared: &[u32]) -> Vec<(u32, u64)> {
    let user = rustix::process::getuid().as_raw();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;

            Some(SweepCandidate {
                pid,
                owner: entry.metadata().ok()?.uid(),
                stat: ProcessStat::read(pid)?,
            })
        })
        .filter(|candidate| may_sweep(candidate, user, created, spared))
        .filter(|candidate| process_is_ours(candidate.pid, root))
        .map(|candidate| (candidate.pid, candidate.stat.start))
        .collect()
}

struct SweepCandidate {
    pid: u32,
    owner: u32,
    stat: ProcessStat,
}

// A marker is only an environment value that anyone can copy, so the sweep also needs the process
// to belong to this user and to have started strictly after the root's creation tick, which
// `record_root_creation` leaves before ruta starts anything. It never takes ruta or an ancestor.
fn may_sweep(candidate: &SweepCandidate, user: u32, created: u64, spared: &[u32]) -> bool {
    let is_new = candidate.stat.start > created;
    let is_spared = spared.contains(&candidate.pid);

    candidate.owner == user && is_new && !is_spared && !candidate.stat.is_zombie()
}

fn self_and_ancestors(pid: u32) -> Vec<u32> {
    let mut chain = vec![pid];

    while let Some(parent) = ProcessStat::read(*chain.last().expect("the chain starts with pid"))
        .map(|stat| stat.parent)
        .filter(|parent| !chain.contains(parent))
    {
        chain.push(parent);
    }

    chain
}

fn group_members((leader, started): (u32, u64), root: &Path) -> Vec<(u32, u64)> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| {
            let stat = ProcessStat::read(pid)?;
            let in_group = stat.group == leader && !stat.is_zombie();
            let is_leader = pid == leader;
            let is_recorded_leader = is_leader && stat.start == started;
            let is_marked_member = !is_leader && process_is_ours(pid, root);

            (in_group && (is_recorded_leader || is_marked_member)).then_some((pid, stat.start))
        })
        .collect()
}

#[expect(
    clippy::print_stderr,
    reason = "a cleanup failure must not hide the original error"
)]
fn signal_members(members: &[(u32, u64)], signal: rustix::process::Signal) {
    for &(pid, started) in members {
        let Some(raw) = rustix::process::Pid::from_raw(pid as i32) else {
            continue;
        };
        let Ok(pidfd) = open_pidfd(raw) else {
            continue;
        };

        if !identity_still_matches(pid, started) {
            continue;
        }

        if let Err(error) = rustix::process::pidfd_send_signal(&pidfd, signal) {
            let error = std::io::Error::from(error);

            if let Some(warning) =
                cleanup::cleanup_warning(&format!("stop the process {pid}"), &error)
            {
                eprintln!("{warning}");
            }
        }
    }
}

pub(crate) fn stop_recorded(path: &Path) -> anyhow::Result<()> {
    let Some((pid, started)) = read_record(path) else {
        cleanup::remove_file(path);
        return Ok(());
    };

    if process_start(pid) == Some(started) {
        return stop_process(path, (pid, started), STOP_TIMEOUT, open_pidfd);
    }

    cleanup::remove_file(path);

    Ok(())
}

fn open_pidfd(raw: rustix::process::Pid) -> std::io::Result<rustix::fd::OwnedFd> {
    rustix::process::pidfd_open(raw, rustix::process::PidfdFlags::empty())
        .map_err(std::io::Error::from)
}

fn stop_process(
    path: &Path,
    (pid, started): (u32, u64),
    timeout: Duration,
    open: impl FnOnce(rustix::process::Pid) -> std::io::Result<rustix::fd::OwnedFd>,
) -> anyhow::Result<()> {
    if !identity_still_matches(pid, started) {
        cleanup::remove_file(path);
        return Ok(());
    }

    let Some(raw) = rustix::process::Pid::from_raw(pid as i32) else {
        cleanup::remove_file(path);
        return Ok(());
    };
    let pidfd = match open(raw) {
        Ok(pidfd) => pidfd,
        Err(error) => {
            if !identity_still_matches(pid, started) {
                cleanup::remove_file(path);
                return Ok(());
            }

            return Err(error).with_context(|| format!("Could not open the process {pid}"));
        }
    };

    stop_after_open(path, (pid, started), process_start(pid), timeout, || {
        send_term(pid, &pidfd)
    })
}

#[expect(
    clippy::print_stderr,
    reason = "a cleanup failure must not hide the original error"
)]
fn send_term(pid: u32, pidfd: &rustix::fd::OwnedFd) {
    if let Err(error) = rustix::process::pidfd_send_signal(pidfd, rustix::process::Signal::TERM) {
        let error = std::io::Error::from(error);

        if let Some(warning) = cleanup::cleanup_warning(&format!("stop the process {pid}"), &error)
        {
            eprintln!("{warning}");
        }
    }
}

fn stop_after_open(
    path: &Path,
    (pid, started): (u32, u64),
    after_open: Option<u64>,
    timeout: Duration,
    signal: impl FnOnce(),
) -> anyhow::Result<()> {
    if after_open != Some(started) {
        cleanup::remove_file(path);
        return Ok(());
    }

    signal();
    stop_loop(path, (pid, started), timeout)
}

fn stop_loop(path: &Path, (pid, started): (u32, u64), timeout: Duration) -> anyhow::Result<()> {
    let stopped = poll_until(timeout, || {
        if !identity_still_matches(pid, started) || process_is_zombie(pid) {
            cleanup::remove_file(path);
            return Ok(Some(()));
        }

        Ok(None)
    })?;

    if stopped.is_none() {
        return Err(IdentityError::DidNotStop {
            pid,
            path: path.to_path_buf(),
        }
        .into());
    }

    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
