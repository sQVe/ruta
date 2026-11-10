use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Child;

pub(crate) fn cleanup_warning(context: &str, error: &std::io::Error) -> Option<String> {
    if cleanup_failure_is_expected(error) {
        return None;
    }

    Some(format!("Could not {context}: {error}"))
}

fn cleanup_failure_is_expected(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::NotFound
        || error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error())
}

#[expect(
    clippy::print_stderr,
    reason = "a cleanup failure must not hide the caller's result"
)]
pub(crate) fn warn(context: &str, result: std::io::Result<()>) {
    if let Err(error) = result
        && let Some(warning) = cleanup_warning(context, &error)
    {
        eprintln!("{warning}");
    }
}

pub(crate) fn remove_file(path: &Path) {
    warn(&format!("remove {}", path.display()), fs::remove_file(path));
}

pub(crate) fn stop_child(child: &mut Child) {
    stop_child_with(child, warn);
}

pub(crate) fn stop_child_with(child: &mut Child, report: impl Fn(&str, std::io::Result<()>)) {
    report("stop the process", child.kill());
    report("reap the process", child.wait().map(|_| ()));
}

// Kills the process group that `child` leads, then reaps `child`. Call it before `child` is
// reaped, so its ID cannot name another group yet.
pub(crate) fn stop_child_group(child: &mut Child) {
    stop_group(child);
    stop_child(child);
}

// Kills the process group that `child` leads. `child` must not be reaped yet.
pub(crate) fn stop_group(child: &Child) {
    warn("stop the process group", stop_process_group(child.id()));
}

pub(crate) fn stop_process_group(child_id: u32) -> std::io::Result<()> {
    let Some(leader) = rustix::process::Pid::from_raw(child_id as i32) else {
        return Ok(());
    };

    rustix::process::kill_process_group(leader, rustix::process::Signal::KILL)
        .map_err(std::io::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_target_is_ignored_and_other_failures_are_reported() {
        let missing = std::io::Error::from(ErrorKind::NotFound);
        assert_eq!(cleanup_warning("remove /tmp/x", &missing), None);

        let denied = std::io::Error::from(ErrorKind::PermissionDenied);
        let reported = cleanup_warning("remove /tmp/x", &denied)
            .expect("an unexpected cleanup failure is reported");
        assert!(
            reported.starts_with("Could not remove /tmp/x: "),
            "unexpected warning: {reported}"
        );
    }

    #[test]
    fn a_process_that_is_gone_is_an_expected_failure() {
        let gone = std::io::Error::from(rustix::io::Errno::SRCH);

        assert_eq!(cleanup_warning("stop the process 1", &gone), None);
    }
}
