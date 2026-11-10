use std::ffi::OsStr;
use std::io::Read;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use rustix::process::{Pid, WaitId, WaitIdOptions};
use thiserror::Error;

use crate::cleanup;

pub(crate) const POLL: Duration = Duration::from_millis(100);
pub(crate) const TOOL_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) fn poll_until<T>(
    timeout: Duration,
    mut attempt: impl FnMut() -> anyhow::Result<Option<T>>,
) -> anyhow::Result<Option<T>> {
    let deadline = Instant::now() + timeout;

    loop {
        if let Some(value) = attempt()? {
            return Ok(Some(value));
        }

        if Instant::now() >= deadline {
            return Ok(None);
        }

        std::thread::sleep(POLL);
    }
}

#[derive(Debug, Error)]
pub(crate) enum ProcessError {
    #[error("{program} timed out after {} seconds.", duration.as_secs())]
    Timeout { program: String, duration: Duration },
    #[error("{program} was stopped before it finished.")]
    Stopped { program: String },
}

pub(crate) fn sandboxed_command(
    program: impl AsRef<OsStr>,
    environment: &[(String, String)],
) -> Command {
    let mut command = Command::new(program);
    command.env_clear().envs(environment.iter().cloned());

    command
}

pub(crate) fn run_tool(command: Command, label: &str, timeout: Duration) -> anyhow::Result<()> {
    let output = run_with_timeout(command, timeout)?;

    if !output.status.success() {
        anyhow::bail!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(())
}

pub(crate) fn run_with_timeout(command: Command, timeout: Duration) -> anyhow::Result<Output> {
    run_until(command, timeout, || false)
}

pub(crate) fn run_until(
    mut command: Command,
    timeout: Duration,
    mut stop_early: impl FnMut() -> bool,
) -> anyhow::Result<Output> {
    let program = command.get_program().to_string_lossy().into_owned();

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let mut child = command
        .spawn()
        .with_context(|| format!("Could not start {program}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || read_all(stdout));
    let stderr_reader = std::thread::spawn(move || read_all(stderr));
    let deadline = Instant::now() + timeout;

    let exited = wait_for(&program, timeout, deadline, || {
        if has_exited(&child).with_context(|| format!("Could not wait for {program}"))? {
            return Ok(Some(()));
        }

        check_stopped(&program, &mut stop_early)?;
        Ok(None)
    });

    if let Err(error) = exited {
        cleanup::stop_child_group(&mut child);
        return Err(error);
    }

    // The leader is a zombie until it is reaped, so its group ID still names this group.
    cleanup::stop_group(&child);
    let status = child
        .wait()
        .with_context(|| format!("Could not wait for {program}"))?;

    wait_for(&program, timeout, deadline, || {
        check_stopped(&program, &mut stop_early)?;

        Ok((stdout_reader.is_finished() && stderr_reader.is_finished()).then_some(()))
    })?;

    let stdout = collect(stdout_reader, &program)?;
    let stderr = collect(stderr_reader, &program)?;

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn wait_for(
    program: &str,
    timeout: Duration,
    deadline: Instant,
    attempt: impl FnMut() -> anyhow::Result<Option<()>>,
) -> anyhow::Result<()> {
    poll_until(deadline.saturating_duration_since(Instant::now()), attempt)?.ok_or_else(|| {
        ProcessError::Timeout {
            program: program.to_string(),
            duration: timeout,
        }
        .into()
    })
}

fn check_stopped(program: &str, stop_early: &mut impl FnMut() -> bool) -> anyhow::Result<()> {
    if stop_early() {
        return Err(ProcessError::Stopped {
            program: program.to_string(),
        }
        .into());
    }

    Ok(())
}

fn has_exited(child: &Child) -> std::io::Result<bool> {
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
    let status = rustix::process::waitid(WaitId::Pid(Pid::from_child(child)), options)?;

    Ok(status.is_some())
}

fn collect(
    reader: std::thread::JoinHandle<std::io::Result<Vec<u8>>>,
    program: &str,
) -> anyhow::Result<Vec<u8>> {
    let output = reader
        .join()
        .map_err(|_| anyhow::anyhow!("the output reader stopped"))
        .with_context(|| format!("Could not read the output of {program}"))?;

    output.with_context(|| format!("Could not read the output of {program}"))
}

fn read_all(reader: Option<impl Read>) -> std::io::Result<Vec<u8>> {
    let Some(mut reader) = reader else {
        return Ok(Vec::new());
    };
    let mut buffer = Vec::new();
    reader.read_to_end(&mut buffer)?;

    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandboxed_command_keeps_only_the_supplied_environment() {
        crate::test_support::in_test_namespace!();

        let environment = vec![("KEPT".to_string(), "value with spaces".to_string())];
        let command = sandboxed_command("env", &environment);

        let output = run_with_timeout(command, Duration::from_secs(2)).expect("env finishes");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"KEPT=value with spaces\n");
    }

    #[test]
    fn run_tool_keeps_the_failure_label_and_trims_stderr() {
        crate::test_support::in_test_namespace!();

        for label in ["import", "xdotool click 1"] {
            let mut command = sandboxed_command("sh", &[]);
            command.args(["-c", "printf '  refused \\n' >&2; exit 7"]);

            let error = run_tool(command, label, Duration::from_secs(2))
                .expect_err("the tool exits unsuccessfully");

            assert_eq!(error.to_string(), format!("{label} failed: refused"));
        }
    }

    #[test]
    fn run_tool_accepts_a_successful_exit() {
        crate::test_support::in_test_namespace!();

        let mut command = sandboxed_command("sh", &[]);
        command.args(["-c", "printf warning >&2; exit 0"]);

        run_tool(command, "import", Duration::from_secs(2))
            .expect("stderr does not turn success into failure");
    }

    #[test]
    fn run_tool_keeps_the_timeout_error() {
        crate::test_support::in_test_namespace!();

        let mut command = sandboxed_command("sleep", &[]);
        command.arg("30");

        let error =
            run_tool(command, "xdotool click 1", Duration::ZERO).expect_err("the tool times out");

        assert!(
            matches!(error.downcast_ref::<ProcessError>(), Some(ProcessError::Timeout { program, duration }) if program == "sleep" && *duration == Duration::ZERO)
        );
    }

    #[test]
    fn run_with_timeout_collects_output_larger_than_a_pipe() {
        crate::test_support::in_test_namespace!();

        let mut command = Command::new("sh");
        command.args(["-c", "seq 1 200000"]);

        let output = run_with_timeout(command, Duration::from_secs(10))
            .expect("large output must finish within the deadline");

        assert!(output.status.success());
        assert!(
            output.stdout.len() > 1_000_000,
            "stdout was {} bytes",
            output.stdout.len()
        );
    }

    #[test]
    fn run_with_timeout_kills_and_reaps_a_hung_child() {
        crate::test_support::in_test_namespace!();

        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30"]);

        let error = run_with_timeout(command, Duration::from_millis(200)).expect_err("timeout");
        let timeout = error
            .downcast_ref::<ProcessError>()
            .expect("a process timeout");

        assert!(
            matches!(
                timeout,
                ProcessError::Timeout { program, duration }
                    if program == "sh" && *duration == Duration::from_millis(200)
            ),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn run_with_timeout_stops_waiting_for_output_a_descendant_outside_the_group_holds() {
        crate::test_support::in_test_namespace!();

        for (held, script) in [
            ("stdout", "setsid sleep 3 2>/dev/null &"),
            ("stderr", "setsid sleep 3 >/dev/null &"),
        ] {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            let limit = Duration::from_millis(300);
            let started = Instant::now();

            let error = run_with_timeout(command, limit).expect_err("the output stays open");
            let elapsed = started.elapsed();

            assert!(
                matches!(
                    error.downcast_ref::<ProcessError>(),
                    Some(ProcessError::Timeout { program, duration })
                        if program == "sh" && *duration == limit
                ),
                "{held}: unexpected error: {error:#}"
            );
            assert!(
                elapsed < Duration::from_secs(1),
                "{held}: the timeout came after {elapsed:?}"
            );
        }
    }

    #[test]
    fn run_with_timeout_stops_the_whole_group_of_a_command_that_times_out() {
        crate::test_support::in_test_namespace!();

        let marker = "RUTA_TEST_GROUP=process-timeout";
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & sleep 30"])
            .env("RUTA_TEST_GROUP", "process-timeout");

        let error =
            run_with_timeout(command, Duration::from_millis(300)).expect_err("the command hangs");

        assert!(
            error.downcast_ref::<ProcessError>().is_some(),
            "unexpected error: {error:#}"
        );
        assert_eq!(
            crate::test_support::wait_until_gone(marker, Duration::from_secs(2)),
            Vec::<u32>::new(),
            "no process of the command's group is alive"
        );
    }

    #[test]
    fn a_timeout_names_the_program_and_seconds() {
        let timeout = ProcessError::Timeout {
            program: "weston".to_string(),
            duration: Duration::from_secs(30),
        };

        assert_eq!(timeout.to_string(), "weston timed out after 30 seconds.");
    }

    #[test]
    fn run_with_timeout_prints_a_start_failure_once() {
        crate::test_support::in_test_namespace!();

        let missing = "ruta-no-such-program";
        let os_error = Command::new(missing)
            .spawn()
            .expect_err("the program is missing")
            .to_string();

        let error = run_with_timeout(Command::new(missing), Duration::from_secs(1))
            .expect_err("the program is missing");
        let rendered = format!("{error:#}");

        assert_eq!(
            rendered.matches(&os_error).count(),
            1,
            "rendered: {rendered}"
        );
    }

    #[test]
    fn poll_until_returns_the_value_of_a_later_attempt() {
        let mut attempts = 0;
        let value = poll_until(Duration::from_secs(1), || {
            attempts += 1;

            Ok((attempts == 3).then_some(attempts))
        })
        .expect("the attempt cannot fail");

        assert_eq!(value, Some(3));
    }

    #[test]
    fn poll_until_passes_an_attempt_error_through_at_once() {
        let mut attempts = 0;
        let error = poll_until::<()>(Duration::from_secs(1), || {
            attempts += 1;

            Err(anyhow::anyhow!("attempt failed"))
        })
        .expect_err("the attempt error is returned");

        assert_eq!(error.to_string(), "attempt failed");
        assert_eq!(attempts, 1, "poll_until stops at the first error");
    }

    #[test]
    fn poll_until_returns_none_after_a_short_timeout() {
        let mut attempts = 0;
        let value = poll_until(Duration::from_millis(250), || {
            attempts += 1;

            Ok(None::<()>)
        })
        .expect("the attempt cannot fail");

        assert_eq!(value, None);
        assert!(attempts >= 2, "poll_until keeps trying until the deadline");
    }
}
