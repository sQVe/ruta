use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Context as _;
use thiserror::Error;

use crate::cleanup;
use crate::display;

use crate::environment::{caller_environment, nested_weston_environment, weston_environment};
use crate::identity::{STOP_TIMEOUT, record_matches, record_process, stop_recorded};
use crate::root::{prepare_root, socket_name};

// Starts Weston and Xwayland, so it needs a real display server.
#[mutants::skip]
pub(crate) fn ensure_display(
    name: &str,
    root: &Path,
    session: &Session,
    (width, height): (u32, u32),
) -> anyhow::Result<String> {
    let mode = session.mode();

    if let Some(display) = display_up(root) {
        if recorded_mode(root).as_deref() == Some(mode) {
            return Ok(display);
        }

        stop_recorded(&root.join("weston.pid"))?;
        clean_display_files(root);
    }

    prepare_root(root)?;

    let log = root.join("weston.log");
    cleanup::remove_file(&log);
    cleanup::remove_file(&root.join("display"));
    cleanup::remove_file(&root.join("xwayland.pid"));

    let mut command = match session {
        Session::Headed {
            wayland_display,
            runtime,
        } => display::nested_weston_command(
            &socket_name(name),
            width,
            height,
            &log,
            wayland_display,
            &nested_weston_environment(&caller_environment(), root, runtime),
        ),
        Session::Headless => display::weston_command(
            &socket_name(name),
            width,
            height,
            &log,
            &weston_environment(&caller_environment(), root),
        ),
    };

    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);

    let mut child = command.spawn().with_context(|| "Could not start weston")?;
    let pid = child.id();

    if let Err(error) = record_process(&root.join("weston.pid"), pid, "weston", STOP_TIMEOUT) {
        cleanup::stop_child_group(&mut child);
        return Err(error);
    }

    drop(child);

    let display = display::wait_for_display(&log, display::WESTON_TIMEOUT)?;
    fs::write(root.join("display"), format!("{display}\n"))
        .with_context(|| "Could not write the display file")?;
    fs::write(root.join("mode"), format!("{mode}\n"))
        .with_context(|| "Could not write the sandbox mode")?;

    Ok(display)
}

pub(crate) fn display_up(root: &Path) -> Option<String> {
    if !record_matches(&root.join("weston.pid")) {
        return None;
    }

    let display = fs::read_to_string(root.join("display")).ok()?;
    let display = display.trim().to_string();

    display.starts_with(':').then_some(display)
}

pub(crate) fn recorded_mode(root: &Path) -> Option<String> {
    fs::read_to_string(root.join("mode"))
        .ok()
        .map(|mode| mode.trim().to_string())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum HeadedSessionError {
    #[error(
        "Headed mode needs WAYLAND_DISPLAY. Run it from the Wayland session, or leave out --headed."
    )]
    NoWaylandDisplay,
    #[error("Headed mode needs an absolute XDG_RUNTIME_DIR to reach the Wayland socket.")]
    NoAbsoluteRuntimeDir,
}

// Where Weston shows the private display.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Session {
    Headless,
    // One window on the caller's Wayland session.
    Headed {
        wayland_display: String,
        runtime: PathBuf,
    },
}

impl Session {
    // The mode record, which a restart without `--headed` reads back.
    fn mode(&self) -> &'static str {
        match self {
            Session::Headless => "headless",
            Session::Headed { .. } => "headed",
        }
    }
}

pub(crate) fn display_session(
    headed: bool,
    wayland: Option<&str>,
    runtime: Option<&str>,
) -> Result<Session, HeadedSessionError> {
    if !headed {
        return Ok(Session::Headless);
    }

    Ok(Session::Headed {
        wayland_display: wayland_display(wayland)?,
        runtime: caller_runtime_dir(runtime)?,
    })
}

fn wayland_display(value: Option<&str>) -> Result<String, HeadedSessionError> {
    match value {
        Some(display) if !display.trim().is_empty() => Ok(display.to_string()),
        _ => Err(HeadedSessionError::NoWaylandDisplay),
    }
}

fn caller_runtime_dir(value: Option<&str>) -> Result<PathBuf, HeadedSessionError> {
    match value {
        Some(directory) if directory.starts_with('/') => Ok(PathBuf::from(directory)),
        _ => Err(HeadedSessionError::NoAbsoluteRuntimeDir),
    }
}

pub(crate) fn clean_display_files(root: &Path) {
    if let Ok(display) = fs::read_to_string(root.join("display")) {
        display::clean_display_files(display.trim());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::identity::process_start;

    pub(crate) fn record_live_display(root: &Path, display: &str) {
        let pid = std::process::id();
        let started = process_start(pid).expect("the test process has a start time");

        fs::write(root.join("weston.pid"), format!("{pid} {started}\n")).expect("write weston.pid");
        fs::write(root.join("display"), display).expect("write the display");
    }

    #[test]
    fn display_up_returns_the_display_of_a_live_record() {
        let root = crate::test_support::temporary_sandbox_root("display-up");
        record_live_display(root.path(), ":7\n");

        assert_eq!(display_up(root.path()), Some(":7".to_string()));
    }

    pub(crate) type StaleDisplayCase = (&'static str, fn(&Path));

    pub(crate) fn stale_display_cases() -> [StaleDisplayCase; 3] {
        [
            ("changed start", |root| {
                let pid = std::process::id();
                let started = process_start(pid).expect("the test process has a start time");

                fs::write(root.join("weston.pid"), format!("{pid} {}\n", started + 1))
                    .expect("rewrite weston.pid");
            }),
            ("missing pid file", |root| {
                fs::remove_file(root.join("weston.pid")).expect("remove weston.pid");
            }),
            ("no colon", |root| {
                fs::write(root.join("display"), "7\n").expect("rewrite the display");
            }),
        ]
    }

    #[test]
    fn display_up_refuses_a_stale_or_missing_record_and_a_display_without_a_colon() {
        for (case, make_stale) in stale_display_cases() {
            let root = crate::test_support::temporary_sandbox_root("display-down");
            record_live_display(root.path(), ":7\n");
            make_stale(root.path());

            assert_eq!(display_up(root.path()), None, "{case}");
        }
    }

    #[test]
    fn recorded_mode_reads_the_trimmed_record() {
        let root = crate::test_support::temporary_sandbox_root("mode");
        fs::write(root.path().join("mode"), "headed\n").expect("write the mode");

        assert_eq!(recorded_mode(root.path()), Some("headed".to_string()));
    }

    #[test]
    fn recorded_mode_is_none_without_a_record() {
        let root = crate::test_support::temporary_sandbox_root("no-mode");

        assert_eq!(recorded_mode(root.path()), None);
    }

    #[test]
    fn display_session_refuses_a_missing_or_blank_wayland_display() {
        for (case, wayland) in [
            ("missing", None),
            ("empty", Some("")),
            ("blank", Some(" \t")),
        ] {
            assert_eq!(
                display_session(true, wayland, Some("/run/user/1000")),
                Err(HeadedSessionError::NoWaylandDisplay),
                "{case}"
            );
        }
    }

    #[test]
    fn display_session_refuses_a_missing_or_relative_runtime_folder() {
        for (case, runtime) in [("missing", None), ("relative", Some("run/user/1000"))] {
            assert_eq!(
                display_session(true, Some("wayland-1"), runtime),
                Err(HeadedSessionError::NoAbsoluteRuntimeDir),
                "{case}"
            );
        }
    }

    #[test]
    fn display_session_accepts_a_valid_session() {
        assert_eq!(
            display_session(true, Some("wayland-1"), Some("/run/user/1000")),
            Ok(Session::Headed {
                wayland_display: "wayland-1".to_string(),
                runtime: PathBuf::from("/run/user/1000"),
            })
        );
    }

    #[test]
    fn display_session_needs_nothing_when_headless() {
        assert_eq!(display_session(false, None, None), Ok(Session::Headless));
    }

    #[test]
    fn session_modes_name_the_mode_record() {
        assert_eq!(Session::Headless.mode(), "headless");
        assert_eq!(
            Session::Headed {
                wayland_display: "wayland-1".to_string(),
                runtime: PathBuf::from("/run/user/1000"),
            }
            .mode(),
            "headed"
        );
    }

    #[test]
    fn headed_values_are_returned_unchanged() {
        assert_eq!(
            wayland_display(Some(" wayland-1 ")),
            Ok(" wayland-1 ".to_string())
        );
        assert_eq!(
            caller_runtime_dir(Some("/run/user/1000")),
            Ok(PathBuf::from("/run/user/1000"))
        );
    }

    #[test]
    fn headed_session_errors_render_the_refusal_text() {
        assert_eq!(
            HeadedSessionError::NoWaylandDisplay.to_string(),
            "Headed mode needs WAYLAND_DISPLAY. Run it from the Wayland session, or leave out --headed."
        );
        assert_eq!(
            HeadedSessionError::NoAbsoluteRuntimeDir.to_string(),
            "Headed mode needs an absolute XDG_RUNTIME_DIR to reach the Wayland socket."
        );
    }
}
