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
pub(crate) fn remove_file(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && let Some(warning) = cleanup_warning(&format!("remove {}", path.display()), &error)
    {
        eprintln!("{warning}");
    }
}

#[expect(
    clippy::print_stderr,
    reason = "a cleanup failure must not hide the original error"
)]
pub(crate) fn stop_child(child: &mut Child) {
    if let Err(error) = child.kill()
        && let Some(warning) = cleanup_warning("stop the process", &error)
    {
        eprintln!("{warning}");
    }

    if let Err(error) = child.wait()
        && let Some(warning) = cleanup_warning("reap the process", &error)
    {
        eprintln!("{warning}");
    }
}

// Kills the process group that `child` leads, then reaps `child`. Call it before `child` is
// reaped, so its ID cannot name another group yet.
pub(crate) fn stop_child_group(child: &mut Child) {
    stop_group(child);
    stop_child(child);
}

// Kills the process group that `child` leads. `child` must not be reaped yet.
#[expect(
    clippy::print_stderr,
    reason = "a cleanup failure must not hide the original error"
)]
pub(crate) fn stop_group(child: &Child) {
    let leader = rustix::process::Pid::from_child(child);

    if let Err(error) = rustix::process::kill_process_group(leader, rustix::process::Signal::KILL) {
        let error = std::io::Error::from(error);

        if let Some(warning) = cleanup_warning("stop the process group", &error) {
            eprintln!("{warning}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_target_is_ignored_and_other_failures_are_reported() {
        crate::test_support::in_test_namespace!();

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
        crate::test_support::in_test_namespace!();

        let gone = std::io::Error::from(rustix::io::Errno::SRCH);

        assert_eq!(cleanup_warning("stop the process 1", &gone), None);
    }
}
