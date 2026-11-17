use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use anyhow::Context as _;

use crate::USAGE;

const READY_POLL: Duration = Duration::from_millis(50);
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const READY_SETTLE: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadyError {
    #[error(
        "The child test app was not ready within {} seconds; still missing {}",
        .timeout.as_secs(),
        display_paths(.missing)
    )]
    Timeout {
        timeout: Duration,
        missing: Vec<PathBuf>,
    },
}

#[derive(Debug)]
pub(crate) struct Modes {
    pub(crate) panic_after: Vec<String>,
    pub(crate) exit_after: Vec<String>,
    pub(crate) launch_child: bool,
    pub(crate) child_ready: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Panic,
    Exit,
}

impl Modes {
    pub(crate) fn after_press(&self, pressed: &[String]) -> Option<Mode> {
        if reached(pressed, &self.panic_after) {
            return Some(Mode::Panic);
        }

        reached(pressed, &self.exit_after).then_some(Mode::Exit)
    }
}

pub(crate) fn parse(arguments: &[String]) -> anyhow::Result<(Modes, Vec<String>)> {
    let mut modes = Modes {
        panic_after: Vec::new(),
        exit_after: Vec::new(),
        launch_child: false,
        child_ready: None,
    };
    let mut remaining = Vec::new();
    let mut arguments = arguments.iter();

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--panic-after" => modes.panic_after = sequence(arguments.next())?,
            "--exit-after" => modes.exit_after = sequence(arguments.next())?,
            "--launch-child" => modes.launch_child = true,
            "--child-ready" => {
                let path = arguments.next().ok_or_else(|| anyhow::anyhow!(USAGE))?;
                modes.child_ready = Some(PathBuf::from(path));
            }
            _ => remaining.push(argument.clone()),
        }
    }

    Ok((modes, remaining))
}

pub(crate) fn reached(pressed: &[String], after: &[String]) -> bool {
    !after.is_empty() && pressed.ends_with(after)
}

pub(crate) fn launch_child(arguments: Vec<String>) -> anyhow::Result<()> {
    let ready = std::env::temp_dir().join(format!("ruta-test-app-ready-{}", std::process::id()));
    let executable = std::env::current_exe().context("Could not find the test app executable")?;
    let mut child = Command::new(executable)
        .args(&arguments)
        .arg("--child-ready")
        .arg(&ready)
        .spawn()
        .context("Could not start the child test app")?;
    let ruta_ready = ruta_root().map(|root| root.join("ready.json"));

    ReadyWait {
        ready: &ready,
        ruta_ready: ruta_ready.as_deref(),
        timeout: READY_TIMEOUT,
        settle: READY_SETTLE,
    }
    .wait(&mut child)?;

    fs::remove_file(&ready).with_context(|| format!("Could not remove {}", ready.display()))?;

    Ok(())
}

pub(crate) fn announce_ready(path: &Path) -> anyhow::Result<()> {
    fs::write(path, []).with_context(|| format!("Could not write {}", path.display()))
}

fn sequence(value: Option<&String>) -> anyhow::Result<Vec<String>> {
    let value = value.ok_or_else(|| anyhow::anyhow!(USAGE))?;

    Ok(value
        .split(',')
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect())
}

struct ReadyWait<'a> {
    ready: &'a Path,
    ruta_ready: Option<&'a Path>,
    timeout: Duration,
    settle: Duration,
}

impl ReadyWait<'_> {
    fn wait(&self, child: &mut Child) -> anyhow::Result<()> {
        let deadline = Instant::now() + self.timeout;
        let mut settle_deadline = None;

        loop {
            if !self.ready.exists() {
                if Instant::now() >= deadline {
                    stop_child(child)?;

                    let mut missing = vec![self.ready.to_path_buf()];
                    missing.extend(
                        self.ruta_ready
                            .filter(|path| !path.exists())
                            .map(Path::to_path_buf),
                    );

                    return Err(ReadyError::Timeout {
                        timeout: self.timeout,
                        missing,
                    }
                    .into());
                }

                std::thread::sleep(READY_POLL);
                continue;
            }

            let Some(ruta_ready) = self.ruta_ready else {
                return Ok(());
            };

            if ruta_ready.exists() {
                return Ok(());
            }

            let settle_deadline =
                *settle_deadline.get_or_insert_with(|| Instant::now() + self.settle);

            if Instant::now() >= settle_deadline {
                return Ok(());
            }

            std::thread::sleep(READY_POLL);
        }
    }
}

fn display_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn ruta_root() -> Option<PathBuf> {
    std::env::var_os("RUTA_ROOT").map(PathBuf::from)
}

fn stop_child(child: &mut Child) -> anyhow::Result<()> {
    match child.kill() {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
        Err(error) => return Err(error).context("Could not stop the child test app"),
    }

    child.wait().context("Could not reap the child test app")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn reached_needs_a_non_empty_suffix() {
        assert!(!reached(&[], &[]));
        assert!(!reached(&strings(&["alpha"]), &[]));

        let after = strings(&["alpha", "beta"]);

        assert!(reached(&strings(&["alpha", "beta"]), &after));
        assert!(reached(&strings(&["gamma", "alpha", "beta"]), &after));
        assert!(!reached(&strings(&["alpha"]), &after));
        assert!(!reached(&strings(&["alpha", "beta", "gamma"]), &after));
    }

    #[test]
    fn parse_reads_comma_separated_sequences() {
        let arguments = strings(&["--panic-after", "alpha,beta", "--exit-after", "gamma"]);
        let (modes, remaining) = parse(&arguments).expect("parse");

        assert_eq!(modes.panic_after, strings(&["alpha", "beta"]));
        assert_eq!(modes.exit_after, strings(&["gamma"]));
        assert!(remaining.is_empty());
    }

    #[test]
    fn parse_reads_an_empty_sequence() {
        let (modes, _) = parse(&strings(&["--panic-after", ""])).expect("parse");

        assert!(modes.panic_after.is_empty());
    }

    #[test]
    fn parse_keeps_other_arguments_and_drops_child_ready() {
        let arguments = strings(&[
            "--geometry",
            "1,2,3,4,5",
            "--child-ready",
            "/tmp/ready",
            "extra",
            "--launch-child",
        ]);
        let (modes, remaining) = parse(&arguments).expect("parse");

        assert_eq!(remaining, strings(&["--geometry", "1,2,3,4,5", "extra"]));
        assert_eq!(modes.child_ready, Some(PathBuf::from("/tmp/ready")));
        assert!(modes.launch_child);
    }

    #[test]
    fn a_flag_without_its_value_fails_with_the_usage() {
        for flag in ["--panic-after", "--exit-after", "--child-ready"] {
            let error = parse(&strings(&[flag])).expect_err("missing value");

            assert_eq!(error.to_string(), USAGE, "{flag}");
        }
    }

    #[test]
    fn panic_wins_over_exit_after_the_same_sequence() {
        let (modes, _) = parse(&strings(&[
            "--exit-after",
            "alpha",
            "--panic-after",
            "alpha",
        ]))
        .expect("parse");

        assert_eq!(modes.after_press(&strings(&["alpha"])), Some(Mode::Panic));
    }

    #[test]
    fn wait_until_ready_stops_a_child_that_never_writes_the_ready_file() {
        crate::test_support::in_test_namespace!();

        let folder = tempfile::tempdir().expect("create the temp folder");
        let ready = folder.path().join("child-ready");
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("start the sleeping child");
        let timeout = Duration::from_millis(100);

        let error = ReadyWait {
            ready: &ready,
            ruta_ready: None,
            timeout,
            settle: Duration::from_millis(50),
        }
        .wait(&mut child)
        .expect_err("the child never writes its ready file");
        let timeout_error = error
            .downcast_ref::<ReadyError>()
            .expect("the wait reports its typed timeout error");
        let ReadyError::Timeout {
            timeout: reported_timeout,
            missing,
        } = timeout_error;

        assert_eq!(*reported_timeout, timeout);
        assert_eq!(missing, &[ready]);
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "the child was stopped"
        );
    }

    #[test]
    fn wait_until_ready_returns_after_the_child_writes_its_ready_file_without_a_ruta_root() {
        crate::test_support::in_test_namespace!();

        let folder = tempfile::tempdir().expect("create the temp folder");
        let ready = folder.path().join("child-ready");
        let mut child = Command::new("sh")
            .args(["-c", "sleep 0.2; : > \"$1\"", "sh"])
            .arg(&ready)
            .spawn()
            .expect("start the child that writes the ready file");

        ReadyWait {
            ready: &ready,
            ruta_ready: None,
            timeout: Duration::from_secs(5),
            settle: Duration::from_millis(50),
        }
        .wait(&mut child)
        .expect("the child writes its ready file before the timeout");

        assert!(
            ready.exists(),
            "the wait returned only after the child wrote its ready file"
        );

        child.wait().expect("reap the child");
    }

    #[test]
    fn wait_returns_at_once_when_ruta_recorded_the_ready_line() {
        crate::test_support::in_test_namespace!();

        let folder = tempfile::tempdir().expect("create the temp folder");
        let ready = folder.path().join("child-ready");
        let ruta_ready = folder.path().join("ready.json");
        fs::write(&ready, []).expect("write the child's ready file");
        fs::write(&ruta_ready, []).expect("write the ruta ready record");
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("start the sleeping child");
        let started = Instant::now();

        ReadyWait {
            ready: &ready,
            ruta_ready: Some(&ruta_ready),
            timeout: Duration::from_secs(5),
            settle: Duration::from_secs(30),
        }
        .wait(&mut child)
        .expect("the recorded ready line ends the wait");

        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the wait did not run out the settle deadline"
        );

        stop_child(&mut child).expect("stop the sleeping child");
    }

    #[test]
    fn wait_returns_after_the_settle_when_ruta_records_no_ready_line() {
        crate::test_support::in_test_namespace!();

        let folder = tempfile::tempdir().expect("create the temp folder");
        let ready = folder.path().join("child-ready");
        let ruta_ready = folder.path().join("ready.json");
        fs::write(&ready, []).expect("write the child's ready file");
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("start the sleeping child");
        let settle = Duration::from_millis(150);
        let started = Instant::now();

        ReadyWait {
            ready: &ready,
            ruta_ready: Some(&ruta_ready),
            timeout: Duration::from_secs(5),
            settle,
        }
        .wait(&mut child)
        .expect("a missing ruta record is not an error");

        assert!(
            started.elapsed() >= settle,
            "the wait ran out the settle deadline before it returned"
        );

        stop_child(&mut child).expect("stop the sleeping child");
    }

    #[test]
    fn timeout_lists_only_the_ruta_record_it_never_saw() {
        crate::test_support::in_test_namespace!();

        let folder = tempfile::tempdir().expect("create the temp folder");
        let ready = folder.path().join("child-ready");
        let ruta_ready = folder.path().join("ready.json");
        fs::write(&ruta_ready, []).expect("write the ruta ready record");
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("start the sleeping child");

        let error = ReadyWait {
            ready: &ready,
            ruta_ready: Some(&ruta_ready),
            timeout: Duration::from_millis(100),
            settle: Duration::from_millis(50),
        }
        .wait(&mut child)
        .expect_err("the child never writes its ready file");
        let ReadyError::Timeout { missing, .. } = error
            .downcast_ref::<ReadyError>()
            .expect("the wait reports its typed timeout error");

        assert_eq!(missing, &[ready]);
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "the child was stopped"
        );
    }

    #[test]
    fn timeout_lists_the_ruta_record_that_never_appeared() {
        crate::test_support::in_test_namespace!();

        let folder = tempfile::tempdir().expect("create the temp folder");
        let ready = folder.path().join("child-ready");
        let ruta_ready = folder.path().join("ready.json");
        let mut child = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("start the sleeping child");

        let error = ReadyWait {
            ready: &ready,
            ruta_ready: Some(&ruta_ready),
            timeout: Duration::from_millis(100),
            settle: Duration::from_millis(50),
        }
        .wait(&mut child)
        .expect_err("the child never writes its ready file");
        let ReadyError::Timeout { missing, .. } = error
            .downcast_ref::<ReadyError>()
            .expect("the wait reports its typed timeout error");

        assert_eq!(missing, &[ready, ruta_ready]);
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "the child was stopped"
        );
    }

    #[test]
    fn stop_child_returns_no_error_for_a_child_that_already_exited() {
        crate::test_support::in_test_namespace!();

        let mut child = Command::new("true")
            .spawn()
            .expect("start the child that exits at once");

        child.wait().expect("wait for the child to exit");
        stop_child(&mut child).expect("stopping an exited child is not an error");
    }
}
