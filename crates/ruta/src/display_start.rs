use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Context as _;

use crate::cleanup;
use crate::display::{self, Mode, Session};
use crate::environment::caller_environment;
use crate::identity::stop::stop_recorded;
use crate::identity::{RECORD_TIMEOUT, record_matches, record_process};
use crate::root::{prepare_root, record, socket_name};

pub(crate) fn ensure_display(
    name: &str,
    root: &Path,
    session: &Session,
    size: (u32, u32),
) -> anyhow::Result<String> {
    let mode = session.mode();

    if let Some(display) = display_up(root) {
        if records_match(root, mode, size) {
            return Ok(display);
        }

        stop_recorded(&root.join(record::WESTON_PID))?;
        display::clean_display_files(&display);
    }

    prepare_root(root)?;

    let log = root.join(record::WESTON_LOG);
    cleanup::remove_file(&log);
    cleanup::remove_file(&root.join(record::DISPLAY));
    cleanup::remove_file(&root.join(record::SIZE));
    cleanup::remove_file(&root.join(record::XWAYLAND_PID));

    let environment = session.environment(&caller_environment(), root);
    let command = display::weston_command(session, &socket_name(name), size, &log, &environment);
    spawn_weston(command, root)?;

    let display = display::wait_for_display(&log, display::WESTON_TIMEOUT)?;
    write_display_records(root, &display, mode, size)?;

    Ok(display)
}

#[mutants::skip]
fn spawn_weston(mut command: Command, root: &Path) -> anyhow::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);

    let mut child = command.spawn().with_context(|| "Could not start weston")?;
    let pid = child.id();

    if let Err(error) = record_process(
        &root.join(record::WESTON_PID),
        pid,
        "weston",
        RECORD_TIMEOUT,
    ) {
        cleanup::stop_child_group(&mut child);
        return Err(error);
    }

    drop(child);

    Ok(())
}

fn write_display_records(
    root: &Path,
    display: &str,
    mode: Mode,
    size: (u32, u32),
) -> anyhow::Result<()> {
    fs::write(root.join(record::DISPLAY), format!("{display}\n"))
        .with_context(|| "Could not write the display file")?;
    fs::write(root.join(record::MODE), format!("{}\n", mode.as_str()))
        .with_context(|| "Could not write the sandbox mode")?;
    fs::write(root.join(record::SIZE), size_record(size))
        .with_context(|| "Could not write the display size")?;

    Ok(())
}

// A running display is reused only in the same mode and at the same output size, so a changed
// `width` or `height` in `ruta.toml` gets a new display.
fn records_match(root: &Path, mode: Mode, size: (u32, u32)) -> bool {
    let same_mode = recorded_mode(root) == Some(mode);
    let same_size = fs::read_to_string(root.join(record::SIZE))
        .is_ok_and(|recorded| recorded == size_record(size));

    same_mode && same_size
}

fn size_record((width, height): (u32, u32)) -> String {
    format!("{width}x{height}\n")
}

pub(crate) fn display_up(root: &Path) -> Option<String> {
    if !record_matches(&root.join(record::WESTON_PID)) {
        return None;
    }

    let display = recorded_display(root)?;

    display.starts_with(':').then_some(display)
}

pub(crate) fn recorded_headed(root: &Path) -> bool {
    recorded_mode(root) == Some(Mode::Headed)
}

fn recorded_mode(root: &Path) -> Option<Mode> {
    let recorded = fs::read_to_string(root.join(record::MODE)).ok()?;

    [Mode::Headless, Mode::Headed]
        .into_iter()
        .find(|mode| mode.as_str() == recorded.trim())
}

fn recorded_display(root: &Path) -> Option<String> {
    fs::read_to_string(root.join(record::DISPLAY))
        .ok()
        .map(|display| display.trim().to_string())
}

pub(crate) fn clean_recorded_display_files(root: &Path) {
    if let Some(display) = recorded_display(root) {
        display::clean_display_files(&display);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::display::{record_live_display, stale_display_cases};

    #[test]
    fn display_up_returns_the_display_of_a_live_record() {
        let root = crate::test_support::temporary_sandbox_root("display-up");
        record_live_display(root.path(), ":7\n");

        assert_eq!(display_up(root.path()), Some(":7".to_string()));
    }

    #[test]
    fn records_match_needs_the_same_mode_and_size() {
        let cases = [
            ("same mode and size", "headless", "840x680\n", true),
            ("another width", "headless", "640x680\n", false),
            ("another height", "headless", "840x600\n", false),
            ("another mode", "headed", "840x680\n", false),
            ("unknown mode", "other", "840x680\n", false),
            ("trimmed mode", " headless ", "840x680\n", true),
            ("untrimmed size", "headless", "840x680", false),
            ("no size record", "headless", "", false),
        ];

        for (case, mode, size, reused) in cases {
            let root = crate::test_support::temporary_sandbox_root("display-reuse");
            fs::write(root.path().join("mode"), format!("{mode}\n")).expect("write the mode");

            if !size.is_empty() {
                fs::write(root.path().join("size"), size).expect("write the size");
            }

            assert_eq!(
                records_match(root.path(), Mode::Headless, (840, 680)),
                reused,
                "{case}"
            );
        }
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
    fn recorded_headed_accepts_only_the_trimmed_headed_record() {
        let root = crate::test_support::temporary_sandbox_root("mode");

        for (mode, headed) in [
            ("headed\n", true),
            (" headed \t", true),
            ("headless\n", false),
            ("other", false),
            ("", false),
        ] {
            fs::write(root.path().join("mode"), mode).expect("write the mode");
            assert_eq!(recorded_headed(root.path()), headed, "{mode}");
        }
    }

    #[test]
    fn recorded_headed_is_false_without_a_record() {
        let root = crate::test_support::temporary_sandbox_root("no-mode");

        assert!(!recorded_headed(root.path()));
    }

    #[test]
    fn write_display_records_keeps_the_record_text() {
        let root = crate::test_support::temporary_directory();

        write_display_records(root.path(), ":7", Mode::Headed, (840, 680))
            .expect("write the records");

        assert_eq!(
            fs::read_to_string(root.path().join("display")).expect("display"),
            ":7\n"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("mode")).expect("mode"),
            "headed\n"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("size")).expect("size"),
            "840x680\n"
        );
    }

    #[test]
    fn write_display_records_stops_at_each_failure_in_the_existing_order() {
        let names = ["display", "mode", "size"];
        let cases = [
            (
                "display",
                "Could not write the display file",
                [None, Some("old"), Some("old")],
            ),
            (
                "mode",
                "Could not write the sandbox mode",
                [Some(":7\n"), None, Some("old")],
            ),
            (
                "size",
                "Could not write the display size",
                [Some(":7\n"), Some("headed\n"), None],
            ),
        ];

        for (blocked, message, expected) in cases {
            let root = crate::test_support::temporary_directory();
            for name in names {
                let path = root.path().join(name);
                if name == blocked {
                    fs::create_dir(path).expect("block the write");
                } else {
                    fs::write(path, "old").expect("write the old record");
                }
            }

            let error = write_display_records(root.path(), ":7", Mode::Headed, (840, 680))
                .expect_err("the record path is a directory");

            assert_eq!(error.to_string(), message, "{blocked}");
            for (name, text) in names.into_iter().zip(expected) {
                let path = root.path().join(name);
                match text {
                    Some(text) => assert_eq!(
                        fs::read_to_string(path).expect("the record remains"),
                        text,
                        "{blocked}: {name}"
                    ),
                    None => assert!(
                        path.is_dir(),
                        "{blocked}: the blocked record stays a directory"
                    ),
                }
            }
        }
    }
}
