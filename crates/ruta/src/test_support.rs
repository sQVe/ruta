use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, ChildStdout, Command};
use std::time::Duration;

use crate::process::poll_until;

use tempfile::{Builder, TempDir};

pub(crate) mod namespace;

// The first statement of every test that sends a signal, or calls code that does. On the host it
// runs the test in a test namespace instead; see `namespace.rs`.
macro_rules! in_test_namespace {
    () => {
        if crate::test_support::namespace::rerun_outside_test_namespace() {
            return;
        }
    };
}

pub(crate) use in_test_namespace;

pub(crate) fn temporary_directory() -> TempDir {
    Builder::new()
        .prefix("ruta-unit-test-")
        .tempdir()
        .expect("create a temporary directory")
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // The body records the PID it ran under in the file that `RUTA_TEST_CANARY` names. Inside the
    // namespace that PID is 1, so any other value means the body ran on the host.
    #[test]
    fn a_guarded_test_runs_its_body_only_inside_the_namespace() {
        in_test_namespace!();

        assert!(namespace::inside_test_namespace());

        if let Some(canary) = std::env::var_os("RUTA_TEST_CANARY") {
            fs::write(canary, std::process::id().to_string()).expect("write the canary");
        }
    }

    #[expect(
        clippy::print_stderr,
        reason = "the process list is the evidence that the namespace hides the host"
    )]
    #[test]
    fn the_namespace_shows_no_process_of_the_host() {
        in_test_namespace!();

        let processes: Vec<(u32, String)> = fs::read_dir("/proc")
            .expect("list /proc")
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .map(|pid| {
                let command = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();

                (pid, String::from_utf8_lossy(&command).replace('\0', " "))
            })
            .collect();
        let test_binary = std::env::current_exe().expect("the test binary has a path");

        for (pid, command) in &processes {
            eprintln!("namespace process {pid}: {command}");
        }

        assert_eq!(
            processes.iter().map(|(pid, _)| *pid).collect::<Vec<_>>(),
            [1],
            "only the test binary is visible"
        );
        assert_eq!(
            fs::read_link("/proc/1/exe").expect("read the init binary"),
            test_binary,
            "PID 1 is the test binary"
        );
    }
}
