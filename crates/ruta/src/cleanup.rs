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

// Reports a cleanup failure. The error it returns is the failure to write the warning, so a
// caller that cannot reach stderr passes it on instead of panicking.
pub(crate) fn warn(context: &str, result: std::io::Result<()>) -> std::io::Result<()> {
    warn_to(&mut std::io::stderr(), context, result)
}

pub(crate) fn warn_to(
    writer: &mut impl std::io::Write,
    context: &str,
    result: std::io::Result<()>,
) -> std::io::Result<()> {
    warn_with(&mut |warning| report_to(writer, warning), context, result)
}

pub(crate) fn warn_with(
    report: &mut impl FnMut(&str) -> std::io::Result<()>,
    context: &str,
    result: std::io::Result<()>,
) -> std::io::Result<()> {
    let Err(error) = result else {
        return Ok(());
    };

    match cleanup_warning(context, &error) {
        Some(warning) => report(&warning),
        None => Ok(()),
    }
}

pub(crate) fn report(warning: &str) -> std::io::Result<()> {
    report_to(&mut std::io::stderr(), warning)
}

fn report_to(writer: &mut impl std::io::Write, warning: &str) -> std::io::Result<()> {
    writeln!(writer, "{warning}")
}

pub(crate) fn with_report_failure(
    error: anyhow::Error,
    report: std::io::Result<()>,
) -> anyhow::Error {
    match report {
        Ok(()) => error,
        Err(report_error) => error.context(format!("Could not report the cleanup: {report_error}")),
    }
}

pub(crate) fn first_failure(
    results: impl IntoIterator<Item = std::io::Result<()>>,
) -> std::io::Result<()> {
    let mut first = Ok(());

    for result in results {
        first = first.and(result);
    }

    first
}

pub(crate) fn remove_file(path: &Path) -> std::io::Result<()> {
    remove_file_with(&mut report, path)
}

pub(crate) fn remove_file_with(
    report: &mut impl FnMut(&str) -> std::io::Result<()>,
    path: &Path,
) -> std::io::Result<()> {
    warn_with(
        report,
        &format!("remove {}", path.display()),
        fs::remove_file(path),
    )
}

pub(crate) fn stop_child(child: &mut Child) -> std::io::Result<()> {
    stop_child_with(child, warn)
}

pub(crate) fn stop_child_with(
    child: &mut Child,
    mut report: impl FnMut(&str, std::io::Result<()>) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let killed = child.kill();
    let waited = child.wait().map(|_| ());

    report("stop the process", killed).and(report("reap the process", waited))
}

// Kills the process group that `child` leads, then reaps `child`. Call it before `child` is
// reaped, so its ID cannot name another group yet.
pub(crate) fn stop_child_group(child: &mut Child) -> std::io::Result<()> {
    stop_group(child).and(stop_child(child))
}

// Kills the process group that `child` leads. `child` must not be reaped yet.
pub(crate) fn stop_group(child: &Child) -> std::io::Result<()> {
    warn("stop the process group", stop_process_group(child.id()))
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
    use crate::test_support::FailingWriter;

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

    #[test]
    fn a_warning_that_cannot_be_written_is_returned() {
        let denied = std::io::Error::from(ErrorKind::PermissionDenied);

        let error = warn_to(&mut FailingWriter, "remove /tmp/x", Err(denied))
            .expect_err("the warning cannot be written");

        assert_eq!(error.kind(), ErrorKind::BrokenPipe);
    }

    #[test]
    fn a_report_failure_is_added_to_the_primary_error() {
        let error = with_report_failure(
            anyhow::anyhow!("the command timed out"),
            Err(std::io::Error::from(ErrorKind::BrokenPipe)),
        );

        assert_eq!(
            format!("{error:#}"),
            "Could not report the cleanup: broken pipe: the command timed out"
        );
    }

    #[test]
    fn an_expected_failure_needs_no_warning() {
        let missing = std::io::Error::from(ErrorKind::NotFound);

        warn_to(&mut FailingWriter, "remove /tmp/x", Err(missing))
            .expect("a missing target writes nothing");
    }

    #[test]
    fn first_failure_keeps_the_first_error_and_runs_every_step() {
        let mut steps = 0;
        let error = first_failure((0..3).map(|_| {
            steps += 1;

            Err(std::io::Error::from(ErrorKind::PermissionDenied))
        }))
        .expect_err("every step fails");

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(steps, 3, "every step ran");
    }

    #[test]
    fn a_stopped_child_is_reaped_before_its_report() {
        crate::test_support::in_test_namespace!();

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn the child");
        let pid = child.id();

        stop_child_with(&mut child, |context, result| {
            warn_to(&mut FailingWriter, context, result)
        })
        .expect("the stop and the reap succeed");

        assert_eq!(
            wait_without_reaping(pid).expect_err("the child is already reaped"),
            rustix::io::Errno::CHILD
        );
    }

    #[test]
    fn a_failed_report_does_not_skip_the_reap_step() {
        crate::test_support::in_test_namespace!();

        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn the child");
        let pid = child.id();
        let mut reports = 0;

        let error = stop_child_with(&mut child, |_, _| {
            reports += 1;

            Err(std::io::Error::from(ErrorKind::BrokenPipe))
        })
        .expect_err("the report fails");

        assert_eq!(error.kind(), ErrorKind::BrokenPipe);
        assert_eq!(reports, 2, "the reap is reported after the failed stop");
        assert_eq!(
            wait_without_reaping(pid).expect_err("the child is already reaped"),
            rustix::io::Errno::CHILD
        );
    }

    fn wait_without_reaping(
        pid: u32,
    ) -> Result<Option<rustix::process::WaitIdStatus>, rustix::io::Errno> {
        let options = rustix::process::WaitIdOptions::EXITED
            | rustix::process::WaitIdOptions::NOHANG
            | rustix::process::WaitIdOptions::NOWAIT;
        let pid = rustix::process::Pid::from_raw(pid as i32).expect("the child PID is positive");

        rustix::process::waitid(rustix::process::WaitId::Pid(pid), options)
    }
}
