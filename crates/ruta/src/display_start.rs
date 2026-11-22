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

    if let Some(display) = display_up(root)
        && records_match(root, mode, size)
    {
        return Ok(display);
    }

    stop_unusable_display(root)?;

    prepare_root(root)?;

    let log = root.join(record::WESTON_LOG);
    cleanup::first_failure([
        cleanup::remove_file(&log),
        cleanup::remove_file(&root.join(record::DISPLAY)),
        cleanup::remove_file(&root.join(record::SIZE)),
        cleanup::remove_file(&root.join(record::XWAYLAND_PID)),
    ])?;

    let environment = session.environment(&caller_environment(), root);
    let command = display::weston_command(session, &socket_name(name), size, &log, &environment);
    spawn_weston(command, root)?;

    let display = display::wait_for_display(&log, display::WESTON_TIMEOUT)?;
    write_display_records(root, &display, mode, size)?;

    Ok(display)
}

// A recorded Weston is stopped even when no usable display record names it: the new Weston would
// overwrite its PID record and leave it running.
fn stop_unusable_display(root: &Path) -> anyhow::Result<()> {
    let display = display_up(root);

    stop_recorded(&root.join(record::WESTON_PID))?;

    if let Some(display) = display {
        display::clean_display_files(&display)?;
    }

    Ok(())
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
        let stopped = cleanup::stop_child_group(&mut child);

        return Err(cleanup::with_report_failure(error, stopped));
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

pub(crate) fn clean_recorded_display_files(
    root: &Path,
    report: &mut impl FnMut(&str) -> std::io::Result<()>,
) -> std::io::Result<()> {
    match recorded_display(root) {
        Some(display) => display::clean_display_files_with(&display, report),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;
    use std::time::Duration;

    use super::*;
    use crate::identity::tests::{record_child, spawn_ready_sleep};
    use crate::test_support::display::{
        record_display_settings, record_live_display, stale_display_cases,
    };

    #[test]
    fn ensure_display_stops_a_recorded_weston_without_a_display_record() {
        crate::test_support::in_test_namespace!();

        let root = crate::test_support::temporary_sandbox_root("display-orphan");
        let mut recorded = spawn_ready_sleep();
        record_child(&root.path().join("weston.pid"), &recorded);
        remember_old_process(root.path(), recorded.id());
        write_fake_weston(root.path());

        let display = ensure_display("test", root.path(), &Session::Headless, (840, 680))
            .expect("the display starts");

        assert_eq!(display, ":9");
        assert_eq!(
            fs::read_to_string(root.path().join("old.state"))
                .expect("the fake Weston saw the old process"),
            "stopped\n",
            "the recorded Weston is stopped before the new one starts"
        );
        assert!(
            recorded.wait_for_exit(Duration::from_secs(2)),
            "the recorded Weston is stopped"
        );
        assert!(
            record_matches(&root.path().join("weston.pid")),
            "the new Weston is recorded"
        );

        stop_recorded(&root.path().join("weston.pid")).expect("stop the fake Weston");
    }

    #[test]
    fn ensure_display_reuses_a_matching_display() {
        crate::test_support::in_test_namespace!();

        let root = crate::test_support::temporary_sandbox_root("display-reuse");
        let mut recorded = spawn_ready_sleep();
        record_child(&root.path().join("weston.pid"), &recorded);
        fs::write(root.path().join("display"), ":7\n").expect("write the display");
        record_display_settings(root.path(), "headless", (840, 680));
        remember_old_process(root.path(), recorded.id());
        write_fake_weston(root.path());

        let display = ensure_display("test", root.path(), &Session::Headless, (840, 680))
            .expect("the display is reused");

        assert_eq!(display, ":7");
        assert!(recorded.is_running(), "the recorded Weston is reused");
        assert!(
            !root.path().join("old.state").exists(),
            "no new Weston is started"
        );
    }

    #[test]
    fn ensure_display_replaces_a_display_with_another_size() {
        crate::test_support::in_test_namespace!();

        let root = crate::test_support::temporary_sandbox_root("display-changed");
        let mut recorded = spawn_ready_sleep();
        record_child(&root.path().join("weston.pid"), &recorded);
        fs::write(root.path().join("display"), ":42\n").expect("write the display");
        record_display_settings(root.path(), "headless", (640, 680));
        remember_old_process(root.path(), recorded.id());
        write_fake_weston(root.path());
        let lock = Path::new("/tmp/.X42-lock");
        let socket = Path::new("/tmp/.X11-unix/X42");
        fs::write(lock, format!("{}\n", u32::MAX)).expect("write the old lock");
        fs::write(socket, "").expect("write the old socket");

        let display = ensure_display("test", root.path(), &Session::Headless, (840, 680))
            .expect("the display is replaced");

        assert_eq!(display, ":9");
        assert_eq!(
            fs::read_to_string(root.path().join("old.state"))
                .expect("the fake Weston saw the old process"),
            "stopped\n",
            "the recorded Weston is stopped before the new one starts"
        );
        assert!(!lock.exists(), "the old lock is removed");
        assert!(!socket.exists(), "the old socket is removed");
        assert!(
            recorded.wait_for_exit(Duration::from_secs(2)),
            "the recorded Weston is stopped"
        );

        stop_recorded(&root.path().join("weston.pid")).expect("stop the fake Weston");
    }

    fn remember_old_process(root: &Path, pid: u32) {
        fs::write(root.join("old.pid"), pid.to_string()).expect("write the old PID");
    }

    fn write_fake_weston(root: &Path) {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("create the bin folder");
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o700))
            .expect("make the bin folder private");
        let weston = bin.join("weston");
        fs::write(
            &weston,
            r#"#!/bin/bash
for argument in "$@"; do
  case "$argument" in
    --log=*) log="${argument#--log=}" ;;
  esac
done
root="$(dirname "$log")"
if [ -f "$root/old.pid" ]; then
  pid="$(cat "$root/old.pid")"
  state="$(cut -d' ' -f3 "/proc/$pid/stat" 2>/dev/null)"
  if [ -n "$state" ] && [ "$state" != "Z" ]; then
    printf 'alive\n' > "$root/old.state"
  else
    printf 'stopped\n' > "$root/old.state"
  fi
fi
printf 'xserver listening on display :9\n' > "$log"
exec -a ruta-fake-weston sleep 30
"#,
        )
        .expect("write the fake weston");
        fs::set_permissions(&weston, fs::Permissions::from_mode(0o755))
            .expect("make the fake weston runnable");
    }

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
