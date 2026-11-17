use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use anyhow::Context as _;

use crate::USAGE;

const READY_POLL: Duration = Duration::from_millis(50);
const READY_TIMEOUT: Duration = Duration::from_secs(30);
const READY_SETTLE: Duration = Duration::from_secs(2);

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

pub(crate) fn launch_child(mut arguments: Vec<String>) -> anyhow::Result<()> {
    arguments.retain(|argument| argument != "--launch-child");
    let ready = std::env::temp_dir().join(format!("ruta-test-app-ready-{}", std::process::id()));
    let executable = std::env::current_exe().context("Could not find the test app executable")?;
    let mut child = Command::new(executable)
        .args(&arguments)
        .arg("--child-ready")
        .arg(&ready)
        .spawn()
        .context("Could not start the child test app")?;

    wait_until_ready(&ready, &mut child, READY_TIMEOUT)?;

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

fn wait_until_ready(ready: &Path, child: &mut Child, timeout: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    let ruta_ready = ruta_root().map(|root| root.join("ready.json"));
    let mut settle_deadline = None;

    loop {
        if !ready.exists() {
            if Instant::now() >= deadline {
                stop_child(child)?;

                anyhow::bail!(
                    "The child test app was not ready within {} seconds; still missing {}",
                    timeout.as_secs(),
                    ready.display()
                );
            }

            std::thread::sleep(READY_POLL);
            continue;
        }

        let Some(ruta_ready) = ruta_ready.as_ref() else {
            return Ok(());
        };

        if ruta_ready.exists() {
            return Ok(());
        }

        let settle_deadline = *settle_deadline.get_or_insert_with(|| Instant::now() + READY_SETTLE);

        if Instant::now() >= settle_deadline {
            return Ok(());
        }

        std::thread::sleep(READY_POLL);
    }
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

        let error = wait_until_ready(&ready, &mut child, Duration::from_millis(100))
            .expect_err("the child never writes its ready file");

        assert!(
            error.to_string().contains(&ready.display().to_string()),
            "{error:#}"
        );
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "the child was stopped"
        );
    }
}
