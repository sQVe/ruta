use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use thiserror::Error;

use super::*;
use crate::config::{App, CommandLine, Ready};
use crate::display_start::tests::{record_live_display, stale_display_cases};
use crate::identity::tests::{
    record_child, spawn_ready_sleep, spawn_with_marker, spawn_without_marker,
};
use crate::root::{RootError, RootReason};

mod down;
mod failed_start;
mod restart;

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

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn cleanup_order(root: &Path) -> [StopTarget; 3] {
    [
        StopTarget::Window {
            root: root.to_path_buf(),
        },
        StopTarget::Recorded {
            path: root.join("xwayland.pid"),
        },
        StopTarget::Recorded {
            path: root.join("weston.pid"),
        },
    ]
}

#[test]
fn cleanup_keeps_the_root_when_a_recorded_process_will_not_stop() {
    crate::test_support::in_test_namespace!();

    for failing in 0..3 {
        let root = crate::test_support::temporary_sandbox_root("will-not-stop");
        let mut visited = Vec::new();

        let error = cleanup(root.path(), |target| {
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
            &cleanup_order(root.path())[..=failing],
            "{failing}"
        );
        assert!(root.path().is_dir(), "{failing}: the root must stay");
    }
}

#[test]
fn cleanup_stops_every_record_in_order_and_removes_the_root() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("all-stopped");
    let mut visited = Vec::new();

    cleanup(root.path(), |target| {
        visited.push(target);
        Ok(())
    })
    .expect("the cleanup succeeds");

    assert_eq!(visited, cleanup_order(root.path()));
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
    crate::test_support::in_test_namespace!();

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
    fs::write(root.path().join("mode"), "headless\n").expect("write the mode");
    fs::write(root.path().join("size"), format!("{width}x{height}\n")).expect("write the size");
    prepare_root(root.path()).expect("the root is prepared");
    let target = crate::test_support::temporary_directory();
    let marker = target.path().join("marker");
    fs::write(&marker, "keep").expect("write the marker");
    let link = root.path().join("bin").join("xdg-open");
    std::os::unix::fs::symlink(&marker, &link).expect("link the opener");
    let mut visited = Vec::new();

    let started = start_window_inner(&config, root.path(), &[], &Session::Headless);
    let error = clean_up_failed_start(root.path(), started, |target| {
        visited.push(target);
        Err(WillNotStop.into())
    })
    .expect_err("the start fails");

    assert!(
        matches!(
            error.downcast_ref::<RootError>(),
            Some(RootError::LinkedOpener { path }) if path == &link
        ),
        "unexpected error: {error:#}"
    );
    assert_eq!(visited, cleanup_order(root.path())[..1]);
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
    let recorded = root.path().join("xwayland.pid");

    let mut window = spawn_with_marker(root.path());
    record_child(&window_record, &window);
    stop_target(StopTarget::Window {
        root: root.path().to_path_buf(),
    })
    .expect("the window stops");

    assert!(
        window.wait_for_exit(Duration::from_secs(2)),
        "window: stopped"
    );
    assert!(!window_record.exists(), "window: the record is removed");

    let mut child = spawn_ready_sleep();
    record_child(&recorded, &child);
    stop_target(StopTarget::Recorded {
        path: recorded.clone(),
    })
    .expect("the recorded child stops");

    assert!(
        child.wait_for_exit(Duration::from_secs(2)),
        "recorded: stopped"
    );
    assert!(!recorded.exists(), "recorded: the record is removed");
}

#[test]
fn stop_target_leaves_a_window_without_the_sandbox_marker_running() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-target-unmarked");
    let window_record = root.path().join("pid");
    let mut window = spawn_without_marker();
    record_child(&window_record, &window);

    stop_target(StopTarget::Window {
        root: root.path().to_path_buf(),
    })
    .expect("the stale record is dropped");

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
    crate::test_support::in_test_namespace!();

    let root = live_sandbox("require-up");

    assert_eq!(
        require_up("test", root.path()).expect("the sandbox is up"),
        ":7"
    );
}

#[test]
fn require_up_refuses_a_stale_display() {
    crate::test_support::in_test_namespace!();

    for (case, make_stale) in stale_display_cases() {
        let root = live_sandbox("require-stale");
        make_stale(root.path());

        let error = require_up("test", root.path()).expect_err("a stale display is refused");

        assert_not_up(&error, case);
    }
}

#[test]
fn require_up_refuses_a_missing_window() {
    crate::test_support::in_test_namespace!();

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
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("opened");

    opened(root.path()).expect("a missing file means nothing opened");

    fs::write(root.path().join("opened"), "https://example.com\n").expect("write opened");
    opened(root.path()).expect("a normal file is printed");

    fs::remove_file(root.path().join("opened")).expect("remove opened");
    fs::create_dir(root.path().join("opened")).expect("create the folder");
    let error = opened(root.path()).expect_err("a folder cannot be read");

    assert_eq!(io_kind(&error), Some(std::io::ErrorKind::IsADirectory));
}

#[test]
fn log_fails_on_a_missing_log_and_prints_a_normal_one() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("log");

    let error = log(root.path()).expect_err("a missing log fails");

    assert_eq!(io_kind(&error), Some(std::io::ErrorKind::NotFound));

    fs::write(root.path().join("app.log"), "ready\n").expect("write the log");
    log(root.path()).expect("a normal log is printed");
}

#[test]
fn a_failed_command_exits_with_failure() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("dispatch");
    let base = root
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a named root");
    let name = base.strip_prefix("ruta-").expect("a sandbox root name");
    assert_eq!(root_path(name), root.path());

    let code = run(&test_config(name), Sandbox::Log);

    assert_eq!(code, ExitCode::FAILURE, "a missing app log fails");
}

#[test]
fn display_size_leaves_room_around_the_window() {
    crate::test_support::in_test_namespace!();

    assert_eq!(display_size((640, 480)), (840, 680));
    assert_eq!(display_size((u32::MAX, 1)), (u32::MAX, 201));
}
