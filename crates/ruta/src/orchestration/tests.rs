use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use thiserror::Error;

use super::*;
use crate::config::{App, CommandLine, Ready};
use crate::identity::tests::{
    record_child, spawn_ready_sleep, spawn_with_marker, spawn_without_marker,
};
use crate::root::{RootError, RootReason};
use crate::test_support::display::{
    record_display_settings, record_live_display, stale_display_cases,
};
use crate::test_support::strings;

mod capture;
mod down;
mod failed_start;
mod restart;
mod status;

#[derive(Debug, Error)]
#[error("the process will not stop")]
struct WillNotStop;

fn test_config(name: &str) -> Config {
    Config {
        name: name.to_string(),
        folder: PathBuf::from("/nonexistent"),
        app: App {
            command: CommandLine {
                program: PathBuf::from("/nonexistent/ruta-app"),
                arguments: Vec::new(),
            },
            title: "ruta test".to_string(),
            width: 640,
            height: 480,
        },
        ready: Ready::Line,
        environment: Vec::new(),
    }
}

#[test]
fn failed_capture_stops_every_target_and_returns_capture_error() {
    let root = crate::test_support::temporary_sandbox_root("capture-failed");
    let mut visited = Vec::new();
    let error = finish_capture(
        root.path(),
        Err(crate::capture::CaptureError::PartialGeometry {
            missing: vec!["scale"],
        }
        .into()),
        |root, target| {
            visited.push((root.to_path_buf(), target));
            Ok(())
        },
    )
    .expect_err("capture fails");

    assert!(
        matches!(error.downcast_ref::<crate::capture::CaptureError>(),
        Some(crate::capture::CaptureError::PartialGeometry { missing }) if missing == &["scale"])
    );
    assert_eq!(
        visited,
        [StopTarget::Window, StopTarget::Xwayland, StopTarget::Weston]
            .map(|target| (root.path().to_path_buf(), target))
    );
    assert!(!root.path().exists());
}

#[test]
fn capture_refuses_an_existing_root_before_starting() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("capture-existing");
    let out = crate::test_support::temporary_directory();
    fs::write(root.path().join("marker"), "keep").expect("write marker");
    let error = capture(
        &test_config("test"),
        root.path(),
        out.path(),
        &[],
        &mut |_| {},
    )
    .expect_err("existing capture root is refused");

    assert!(
        error
            .to_string()
            .contains("RUTA_NAME=test-capture ruta down")
    );
    assert_eq!(
        fs::read_to_string(root.path().join("marker")).expect("marker stays"),
        "keep"
    );
    assert_eq!(fs::read_dir(root.path()).expect("root stays").count(), 1);
    assert_eq!(fs::read_dir(out.path()).expect("out stays").count(), 0);
}

#[test]
fn cleanup_keeps_the_root_when_a_recorded_process_will_not_stop() {
    for failing in 0..3 {
        let root = crate::test_support::temporary_sandbox_root("will-not-stop");
        let mut visited = Vec::new();

        let error = cleanup(root.path(), |_, target| {
            visited.push(target);

            if visited.len() == failing + 1 {
                return Err(WillNotStop.into());
            }

            Ok(())
        })
        .expect_err("a failed stop fails the cleanup");

        assert!(
            error.downcast_ref::<WillNotStop>().is_some(),
            "{failing}: {error:#}"
        );
        assert_eq!(
            visited.as_slice(),
            &[StopTarget::Window, StopTarget::Xwayland, StopTarget::Weston][..=failing],
            "{failing}"
        );
        assert!(root.path().is_dir(), "{failing}: the root must stay");
    }
}

#[test]
fn cleanup_stops_every_record_in_order_and_removes_the_root() {
    let root = crate::test_support::temporary_sandbox_root("all-stopped");
    let mut visited = Vec::new();

    cleanup(root.path(), |_, target| {
        visited.push(target);
        Ok(())
    })
    .expect("the cleanup succeeds");

    assert_eq!(
        visited,
        [StopTarget::Window, StopTarget::Xwayland, StopTarget::Weston]
    );
    assert!(!root.path().exists(), "the root must be removed");
}

#[test]
fn cleanup_refuses_a_symlinked_root_and_leaves_its_target() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let target = crate::test_support::temporary_directory();
    let marker = target.path().join("marker");
    fs::write(&marker, "keep").expect("write the marker");
    let root = folder.path().join("root");
    std::os::unix::fs::symlink(target.path(), &root).expect("link the root");

    let error = cleanup(&root, stop_target).expect_err("the symlink is refused");
    let root_error = error
        .downcast_ref::<RootError>()
        .expect("the refusal is a root error");

    assert!(
        matches!(
            root_error,
            RootError::Unsafe {
                path,
                reason: RootReason::Symlink,
            } if path == &root
        ),
        "unexpected error: {error:#}"
    );
    assert!(marker.is_file(), "cleanup must not touch the link target");
    assert!(
        fs::symlink_metadata(&root)
            .expect("the root")
            .file_type()
            .is_symlink()
    );
}

#[test]
fn cleanup_refuses_a_root_that_is_not_private() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("mode");
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).expect("set the mode");

    let error = cleanup(root.path(), stop_target).expect_err("the mode is refused");
    let root_error = error
        .downcast_ref::<RootError>()
        .expect("the refusal is a root error");

    assert!(
        matches!(
            root_error,
            RootError::Unsafe {
                path,
                reason: RootReason::WrongMode { mode: 0o755 },
            } if path.as_path() == root.path()
        ),
        "unexpected error: {error:#}"
    );
    let metadata = fs::symlink_metadata(root.path()).expect("the root");
    assert!(metadata.is_dir());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o755);
}

#[test]
fn require_up_refuses_a_symlinked_root() {
    let folder = crate::test_support::temporary_directory();
    let target = crate::test_support::temporary_directory();
    let marker = target.path().join("marker");
    fs::write(&marker, "keep").expect("write the marker");
    let root = folder.path().join("root");
    std::os::unix::fs::symlink(target.path(), &root).expect("link the root");

    let error = require_up("test", &root).expect_err("the symlink is refused");
    let root_error = error
        .downcast_ref::<RootError>()
        .expect("the refusal is a root error");

    assert!(
        matches!(
            root_error,
            RootError::Unsafe {
                path,
                reason: RootReason::Symlink,
            } if path == &root
        ),
        "unexpected error: {error:#}"
    );
    assert!(marker.is_file(), "the target must not change");
    assert!(
        fs::symlink_metadata(&root)
            .expect("the root")
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_failed_start_keeps_its_error_when_cleanup_fails() {
    crate::test_support::in_test_namespace!();

    let root = live_sandbox("failed-start");
    let config = test_config("test");
    let (width, height) = display_size((config.app.width, config.app.height));
    // The live display matches the config, so the start reuses it and stops nothing.
    record_display_settings(root.path(), "headless", (width, height));
    prepare_root(root.path()).expect("the root is prepared");
    let target = crate::test_support::temporary_directory();
    let marker = target.path().join("marker");
    fs::write(&marker, "keep").expect("write the marker");
    let link = root.path().join("bin").join("xdg-open");
    std::os::unix::fs::symlink(&marker, &link).expect("link the opener");
    let mut visited = Vec::new();

    let error =
        start_sandbox(&config, root.path(), &[], &Session::Headless).expect_err("the start fails");
    let error = failed_start(root.path(), error, |_, target| {
        visited.push(target);
        Err(WillNotStop.into())
    });

    assert!(
        matches!(
            error.downcast_ref::<RootError>(),
            Some(RootError::LinkedOpener { path }) if path == &link
        ),
        "unexpected error: {error:#}"
    );
    assert_eq!(visited, [StopTarget::Window]);
    assert!(
        root.path().join("weston.pid").is_file(),
        "the root must stay"
    );
    assert_eq!(fs::read_to_string(&marker).expect("the marker"), "keep");
}

#[test]
fn stop_target_stops_a_recorded_child_and_removes_its_record() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-target");
    let window_record = root.path().join("pid");

    let mut window = spawn_with_marker(root.path());
    record_child(&window_record, &window);
    stop_target(root.path(), StopTarget::Window).expect("the window stops");

    assert!(
        window.wait_for_exit(Duration::from_secs(2)),
        "window: stopped"
    );
    assert!(!window_record.exists(), "window: the record is removed");

    for (target, name) in [
        (StopTarget::Xwayland, "xwayland.pid"),
        (StopTarget::Weston, "weston.pid"),
    ] {
        let recorded = root.path().join(name);
        let other = root.path().join("unrelated.pid");
        fs::write(&other, "keep").expect("write unrelated record");
        let mut child = spawn_ready_sleep();
        record_child(&recorded, &child);

        stop_target(root.path(), target).expect("the recorded child stops");

        assert!(
            child.wait_for_exit(Duration::from_secs(2)),
            "{target:?}: stopped"
        );
        assert!(!recorded.exists(), "{target:?}: the record is removed");
        assert_eq!(
            fs::read_to_string(&other).expect("unrelated record remains"),
            "keep"
        );
    }
}

#[test]
fn stop_target_leaves_a_window_without_the_sandbox_marker_running() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-target-unmarked");
    let window_record = root.path().join("pid");
    let mut window = spawn_without_marker();
    record_child(&window_record, &window);

    stop_target(root.path(), StopTarget::Window).expect("the stale record is dropped");

    assert!(window.is_running(), "an unmarked child must keep running");
}

fn live_sandbox(label: &str) -> tempfile::TempDir {
    let root = crate::test_support::temporary_sandbox_root(label);
    record_live_display(root.path(), ":7\n");
    fs::write(root.path().join("window"), "123\n").expect("write the window");

    root
}

fn assert_not_up(error: &anyhow::Error, case: &str) {
    assert_eq!(
        error.downcast_ref::<SandboxError>(),
        Some(&SandboxError::NotUp {
            name: "test".to_string()
        }),
        "{case}: {error:#}"
    );
}

#[test]
fn require_up_returns_the_display_of_a_live_sandbox() {
    let root = live_sandbox("require-up");

    assert_eq!(
        require_up("test", root.path()).expect("the sandbox is up"),
        ":7"
    );
}

#[test]
fn require_up_refuses_a_stale_display() {
    for (case, make_stale) in stale_display_cases() {
        let root = live_sandbox("require-stale");
        make_stale(root.path());

        let error = require_up("test", root.path()).expect_err("a stale display is refused");

        assert_not_up(&error, case);
    }
}

#[test]
fn require_up_refuses_a_missing_window() {
    let root = live_sandbox("require-window");
    fs::remove_file(root.path().join("window")).expect("remove the window");

    let error = require_up("test", root.path()).expect_err("a missing window is refused");

    assert_not_up(&error, "missing window");
    assert_eq!(display_up(root.path()), Some(":7".to_string()));
}

fn io_kind(error: &anyhow::Error) -> Option<std::io::ErrorKind> {
    error
        .downcast_ref::<std::io::Error>()
        .map(std::io::Error::kind)
}

#[test]
fn opened_accepts_a_missing_or_normal_file_and_fails_on_a_folder() {
    let root = crate::test_support::temporary_sandbox_root("opened");

    assert_eq!(
        opened(root.path()).expect("a missing file means nothing opened"),
        ""
    );

    fs::write(root.path().join("opened"), "https://example.com\n").expect("write opened");
    assert_eq!(
        opened(root.path()).expect("a normal file is returned"),
        "https://example.com\n"
    );

    fs::remove_file(root.path().join("opened")).expect("remove opened");
    fs::create_dir(root.path().join("opened")).expect("create the folder");
    let error = opened(root.path()).expect_err("a folder cannot be read");

    assert_eq!(io_kind(&error), Some(std::io::ErrorKind::IsADirectory));
}

#[test]
fn log_fails_on_a_missing_log_and_prints_a_normal_one() {
    let root = crate::test_support::temporary_sandbox_root("log");

    let error = log(root.path()).expect_err("a missing log fails");

    assert_eq!(io_kind(&error), Some(std::io::ErrorKind::NotFound));

    fs::write(root.path().join("app.log"), "ready\n").expect("write the log");
    assert_eq!(
        log(root.path()).expect("a normal log is returned"),
        "ready\n"
    );
}

#[test]
fn execute_returns_a_command_failure() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("dispatch");
    let base = root
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a named root");
    let name = base.strip_prefix("ruta-").expect("a sandbox root name");
    assert_eq!(root_path(name), root.path());

    let mut output = String::new();
    let error = execute(&test_config(name), Sandbox::Log, &mut |text| {
        output.push_str(text)
    })
    .expect_err("a missing app log fails");

    assert_eq!(io_kind(&error), Some(std::io::ErrorKind::NotFound));
    assert_eq!(output, "");
}

#[test]
fn display_size_leaves_room_around_the_window() {
    assert_eq!(display_size((640, 480)), (840, 680));
    assert_eq!(display_size((u32::MAX, 1)), (u32::MAX, 201));
}
