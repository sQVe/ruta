use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::time::Duration;

use anyhow::Context as _;

use super::{IdentityError, ProcessIdentity, process_is_ours};
use crate::cleanup;
use crate::process::poll_until;
use crate::process_stat::ProcessStat;
use crate::root::root_created;

pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn stop_window(root: &Path) -> anyhow::Result<()> {
    stop_group(root, STOP_TIMEOUT)?;
    stop_marked(root, std::process::id(), STOP_TIMEOUT)
}

pub(crate) fn stop_recorded(path: &Path) -> anyhow::Result<()> {
    stop_recorded_with(path, open_pidfd)
}

fn stop_recorded_with(
    path: &Path,
    open: impl FnOnce(rustix::process::Pid) -> std::io::Result<rustix::fd::OwnedFd>,
) -> anyhow::Result<()> {
    if let Some(identity) = ProcessIdentity::read(path)
        && !stop_process(identity, STOP_TIMEOUT, open)?
    {
        return Err(IdentityError::DidNotStop {
            pid: identity.pid,
            path: path.to_path_buf(),
        }
        .into());
    }

    cleanup::remove_file(path);

    Ok(())
}

fn stop_group(root: &Path, timeout: Duration) -> anyhow::Result<()> {
    let path = root.join(crate::root::record::PID);

    let Some(leader) = ProcessIdentity::read(&path) else {
        cleanup::remove_file(&path);
        return Ok(());
    };

    if stop_all(|| group_members(leader, root), timeout)?.is_some() {
        return Err(IdentityError::DidNotStop {
            pid: leader.pid,
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
    members: impl Fn() -> Vec<ProcessIdentity>,
    timeout: Duration,
) -> anyhow::Result<Option<u32>> {
    let alive = members();
    if alive.is_empty() {
        return Ok(None);
    }

    signal_members(&alive, rustix::process::Signal::TERM);

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

    Ok(members().first().map(|identity| identity.pid))
}

// The live processes of the current user that hold this root's marker and started after the root
// was made, with their start times.
fn marked_processes(root: &Path, created: u64, spared: &[u32]) -> Vec<ProcessIdentity> {
    let user = rustix::process::getuid().as_raw();
    let Ok(entries) = crate::process_stat::process_entries() else {
        return Vec::new();
    };

    entries
        .filter_map(|(pid, entry)| {
            Some(SweepCandidate {
                pid,
                owner: entry.metadata().ok()?.uid(),
                stat: ProcessStat::read_live(pid)?,
            })
        })
        .filter(|candidate| may_sweep(candidate, user, created, spared))
        .filter(|candidate| process_is_ours(candidate.pid, root))
        .map(|candidate| ProcessIdentity {
            pid: candidate.pid,
            started: candidate.stat.start,
        })
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

    candidate.owner == user && is_new && !is_spared
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

fn group_members(leader: ProcessIdentity, root: &Path) -> Vec<ProcessIdentity> {
    let Ok(entries) = crate::process_stat::process_entries() else {
        return Vec::new();
    };

    entries
        .filter_map(|(pid, _)| {
            let stat = ProcessStat::read_live(pid)?;
            let in_group = stat.group == leader.pid;
            let is_leader = pid == leader.pid;
            let is_recorded_leader = is_leader && stat.start == leader.started;
            let is_marked_member = !is_leader && process_is_ours(pid, root);
            let belongs_to_sandbox = is_recorded_leader || is_marked_member;

            (in_group && belongs_to_sandbox).then_some(ProcessIdentity {
                pid,
                started: stat.start,
            })
        })
        .collect()
}

fn signal_members(members: &[ProcessIdentity], signal: rustix::process::Signal) {
    for &identity in members {
        let Some(raw) = rustix::process::Pid::from_raw(identity.pid as i32) else {
            continue;
        };
        let Ok(pidfd) = open_pidfd(raw) else {
            continue;
        };

        if !identity.is_current() {
            continue;
        }

        send_signal(identity.pid, &pidfd, signal);
    }
}

fn open_pidfd(raw: rustix::process::Pid) -> std::io::Result<rustix::fd::OwnedFd> {
    rustix::process::pidfd_open(raw, rustix::process::PidfdFlags::empty())
        .map_err(std::io::Error::from)
}

fn stop_process(
    identity: ProcessIdentity,
    timeout: Duration,
    open: impl FnOnce(rustix::process::Pid) -> std::io::Result<rustix::fd::OwnedFd>,
) -> anyhow::Result<bool> {
    let Some(raw) = rustix::process::Pid::from_raw(identity.pid as i32) else {
        return Ok(true);
    };
    let opened = open(raw);

    if !identity.is_current() {
        return Ok(true);
    }

    let pidfd = opened.with_context(|| format!("Could not open the process {}", identity.pid))?;
    send_signal(identity.pid, &pidfd, rustix::process::Signal::TERM);
    let stopped = poll_until(timeout, || {
        let alive =
            ProcessStat::read_live(identity.pid).is_some_and(|stat| stat.start == identity.started);

        Ok((!alive).then_some(()))
    })?;

    Ok(stopped.is_some())
}

fn send_signal(pid: u32, pidfd: &rustix::fd::OwnedFd, signal: rustix::process::Signal) {
    cleanup::warn(
        &format!("stop the process {pid}"),
        rustix::process::pidfd_send_signal(pidfd, signal).map_err(std::io::Error::from),
    );
}

#[cfg(test)]
pub(crate) mod tests;
