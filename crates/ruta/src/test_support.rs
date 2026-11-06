use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, ChildStdout, Command};
use std::time::Duration;

use crate::process::poll_until;

use tempfile::{Builder, TempDir};

pub(crate) fn temporary_directory() -> TempDir {
    Builder::new()
        .prefix("ruta-unit-test-")
        .tempdir()
        .expect("create a temporary directory")
}

// A private folder under `/var/tmp` whose name passes the root checks, so cleanup may remove it.
pub(crate) fn temporary_sandbox_root(label: &str) -> TempDir {
    let prefix = format!("ruta-test-{label}-");
    let root = Builder::new()
        .prefix(&prefix)
        .tempdir_in("/var/tmp")
        .expect("create a temporary sandbox root");

    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
        .expect("secure the sandbox root");

    root
}

// Lists live processes whose environment holds `variable`, such as `RUTA_ROOT=/var/tmp/ruta-x`.
// A zombie has no readable environment, so it is not listed.
pub(crate) fn live_processes_with(variable: &str) -> Vec<u32> {
    let processes = fs::read_dir("/proc").expect("list /proc");

    processes
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            fs::read(format!("/proc/{pid}/environ")).is_ok_and(|bytes| {
                bytes
                    .split(|byte| *byte == 0)
                    .any(|entry| entry == variable.as_bytes())
            })
        })
        .collect()
}

// Waits until no live process holds `variable`, and returns the ones still alive.
pub(crate) fn wait_until_gone(variable: &str, timeout: Duration) -> Vec<u32> {
    poll_until(timeout, || {
        let alive = live_processes_with(variable);

        Ok(alive.is_empty().then_some(()))
    })
    .expect("the process scan cannot fail");

    live_processes_with(variable)
}

pub(crate) fn stop_child(child: &mut Child) {
    if let Err(error) = child.kill()
        && let Some(warning) = crate::cleanup::cleanup_warning("stop the process", &error)
    {
        panic!("{warning}");
    }

    if let Err(error) = child.wait()
        && let Some(warning) = crate::cleanup::cleanup_warning("reap the process", &error)
    {
        panic!("{warning}");
    }
}

// A test child that is stopped and reaped on drop. A group leader takes its whole group along.
pub(crate) struct ChildGuard {
    child: Child,
    reaped: bool,
    group: bool,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self {
            child,
            reaped: false,
            group: false,
        }
    }

    pub(crate) fn spawn(command: &mut Command) -> Self {
        Self::new(command.spawn().expect("spawn the child"))
    }

    pub(crate) fn spawn_group(command: &mut Command) -> Self {
        command.process_group(0);
        let mut guard = Self::new(command.spawn().expect("spawn the child"));
        guard.group = true;

        guard
    }

    pub(crate) fn take_stdout(&mut self) -> ChildStdout {
        self.child
            .stdout
            .take()
            .expect("the child's stdout is piped")
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn is_running(&mut self) -> bool {
        !self.reaped && self.child.try_wait().expect("check the child").is_none()
    }

    pub(crate) fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        if self.reaped {
            return true;
        }

        let exited = poll_until(timeout, || {
            Ok(self.child.try_wait().expect("check the child"))
        })
        .expect("the exit wait cannot fail")
        .is_some();
        self.reaped = exited;

        exited
    }

    pub(crate) fn wait_until_ready(&self, fragment: &str, timeout: Duration) {
        let ready = poll_until(timeout, || {
            let Ok(cmdline) = fs::read(format!("/proc/{}/cmdline", self.id())) else {
                return Ok(None);
            };
            let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");

            Ok(cmdline.contains(fragment).then_some(()))
        })
        .expect("the readiness check cannot fail");

        assert!(ready.is_some(), "the child never named itself {fragment}");
    }

    pub(crate) fn reap(&mut self) {
        if self.reaped {
            return;
        }

        if self.group {
            stop_process_group(self.child.id());
        }

        stop_child(&mut self.child);
        self.reaped = true;
    }
}

pub(crate) fn stop_process_group(child_id: u32) {
    let Some(pgid) = rustix::process::Pid::from_raw(child_id as i32) else {
        return;
    };

    if let Err(error) = rustix::process::kill_process_group(pgid, rustix::process::Signal::KILL) {
        let error = std::io::Error::from(error);

        if let Some(warning) = crate::cleanup::cleanup_warning("stop the process group", &error) {
            panic!("{warning}");
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.reap();
    }
}
