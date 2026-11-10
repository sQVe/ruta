use std::ffi::OsStr;

use super::*;
use crate::process::POLL;

fn environment() -> Vec<(String, String)> {
    vec![
        ("XDG_RUNTIME_DIR".to_string(), "/tmp/run".to_string()),
        ("HOME".to_string(), "/tmp/home".to_string()),
    ]
}

fn value<'a>(command: &'a Command, key: &str) -> Option<&'a OsStr> {
    command
        .get_envs()
        .find(|(name, _)| *name == OsStr::new(key))
        .and_then(|(_, value)| value)
}

#[test]
fn display_from_log_reads_the_xwayland_display() {
    let log = "some line\nxserver listening on display :7\nanother line\n";

    assert_eq!(display_from_log(log), Some(":7".to_string()));
    assert_eq!(display_from_log("no display here"), None);
    assert_eq!(
        display_from_log("xserver listening on display wayland-0"),
        None
    );
}

#[test]
fn headless_weston_drops_the_wayland_session() {
    let command = weston_command(
        &Session::Headless,
        "ruta-mock",
        (1400, 900),
        Path::new("/tmp/weston.log"),
        &environment(),
    );
    let arguments: Vec<&OsStr> = command.get_args().collect();

    assert_eq!(
        arguments,
        [
            "--xwayland",
            "--socket=ruta-mock",
            "--width=1400",
            "--height=900",
            "--log=/tmp/weston.log",
            "--backend=headless"
        ]
    );
    assert_eq!(value(&command, "WAYLAND_DISPLAY"), None);
    assert_eq!(
        value(&command, "XDG_RUNTIME_DIR"),
        Some(OsStr::new("/tmp/run"))
    );
}

#[test]
fn nested_weston_gets_the_caller_wayland_session() {
    let mut caller = environment();
    caller.push(("WAYLAND_DISPLAY".to_string(), "wrong-session".to_string()));
    let session = Session::Headed {
        wayland_display: "wayland-1".to_string(),
        runtime: PathBuf::from("/tmp/run"),
    };
    let command = weston_command(
        &session,
        "ruta-demo",
        (1400, 900),
        Path::new("/tmp/weston.log"),
        &caller,
    );
    let arguments: Vec<&OsStr> = command.get_args().collect();

    assert_eq!(
        arguments,
        [
            "--xwayland",
            "--socket=ruta-demo",
            "--width=1400",
            "--height=900",
            "--log=/tmp/weston.log",
            "--backend=wayland"
        ]
    );
    assert_eq!(
        value(&command, "WAYLAND_DISPLAY"),
        Some(OsStr::new("wayland-1"))
    );
    assert_eq!(
        value(&command, "XDG_RUNTIME_DIR"),
        Some(OsStr::new("/tmp/run"))
    );
}

fn display_files(folder: &Path, owner: u32) -> (PathBuf, PathBuf) {
    display_files_for(folder, "7", owner)
}

fn display_files_for(folder: &Path, number: &str, owner: u32) -> (PathBuf, PathBuf) {
    let lock = folder.join(format!(".X{number}-lock"));
    let socket = folder.join(format!(".X11-unix/X{number}"));

    fs::create_dir_all(folder.join(".X11-unix")).expect("socket folder");
    fs::write(&lock, format!("{owner:>10}\n")).expect("lock");
    fs::write(&socket, "").expect("socket");

    (lock, socket)
}

#[test]
fn clean_display_files_removes_the_files_of_a_dead_owner() {
    let folder = crate::test_support::temporary_directory();
    let (lock, socket) = display_files(folder.path(), u32::MAX);

    clean_display_files_in(folder.path(), ":7", Duration::from_secs(1));

    assert!(!lock.exists());
    assert!(!socket.exists());
}

#[test]
fn clean_display_files_keeps_the_files_of_a_live_owner() {
    let folder = crate::test_support::temporary_directory();
    let (lock, socket) = display_files(folder.path(), std::process::id());

    clean_display_files_in(folder.path(), ":7", Duration::ZERO);

    assert!(lock.exists());
    assert!(socket.exists());
}

#[test]
fn clean_display_files_keeps_the_files_of_a_new_owner() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let mut dying = Command::new("sleep").arg("30").spawn().expect("sleep");
    let (lock, socket) = display_files(folder.path(), dying.id());
    let replacement = lock.clone();
    let new_owner = std::thread::spawn(move || {
        std::thread::sleep(POLL);
        fs::write(&replacement, format!("{:>10}\n", std::process::id())).expect("new lock");
        crate::test_support::stop_child(&mut dying);
    });

    clean_display_files_in(folder.path(), ":7", Duration::from_secs(10));
    new_owner.join().expect("new owner thread");

    assert!(lock.exists());
    assert!(socket.exists());
}

#[test]
fn clean_display_files_ignores_a_display_that_is_not_a_number() {
    let folder = crate::test_support::temporary_directory();
    let (lock, socket) = display_files(folder.path(), u32::MAX);

    clean_display_files_in(folder.path(), ":7x", Duration::ZERO);

    assert!(lock.exists());
    assert!(socket.exists());
}

#[test]
fn clean_display_files_ignores_a_display_without_a_number() {
    let folder = crate::test_support::temporary_directory();
    let (lock, socket) = display_files_for(folder.path(), "", u32::MAX);

    clean_display_files_in(folder.path(), ":", Duration::ZERO);

    assert!(lock.exists());
    assert!(socket.exists());
}

#[test]
fn clean_display_files_waits_for_an_owner_that_exits_later() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let mut owner = Command::new("sleep").arg("30").spawn().expect("sleep");
    let (lock, socket) = display_files(folder.path(), owner.id());
    let exit = std::thread::spawn(move || {
        std::thread::sleep(POLL * 3);
        crate::test_support::stop_child(&mut owner);
    });

    clean_display_files_in(folder.path(), ":7", Duration::from_secs(10));
    exit.join().expect("owner thread");

    assert!(!lock.exists());
    assert!(!socket.exists());
}

#[test]
fn wait_for_display_returns_a_display_that_appears_later() {
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("weston.log");
    let writer_path = log.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        fs::write(&writer_path, "xserver listening on display :7\n").expect("write the log");
    });

    let display = wait_for_display(&log, Duration::from_secs(2)).expect("the display appears");

    writer.join().expect("the writer thread");
    assert_eq!(display, ":7");
}

#[test]
fn wait_for_display_reports_a_timeout() {
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("weston.log");
    let timeout = Duration::from_millis(150);

    let error = wait_for_display(&log, timeout).expect_err("a missing display times out");

    assert!(
        matches!(
            error.downcast_ref::<DisplayError>(),
            Some(DisplayError::DisplayTimeout { timeout: recorded })
                if *recorded == timeout
        ),
        "unexpected error: {error:#}"
    );
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
            runtime: PathBuf::from("/run/user/1000")
        })
    );
}

#[test]
fn display_session_needs_nothing_when_headless() {
    assert_eq!(display_session(false, None, None), Ok(Session::Headless));
}

#[test]
fn session_modes_name_the_mode_record() {
    assert_eq!(Session::Headless.mode().as_str(), "headless");
    assert_eq!(
        Session::Headed {
            wayland_display: "wayland-1".to_string(),
            runtime: PathBuf::from("/run/user/1000")
        }
        .mode()
        .as_str(),
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

#[test]
fn session_environment_uses_the_selected_runtime_without_the_host_display() {
    let root = crate::test_support::temporary_directory();
    let caller = vec![
        ("DISPLAY".to_string(), ":0".to_string()),
        ("WAYLAND_DISPLAY".to_string(), "wayland-host".to_string()),
    ];
    let cases = [
        (Session::Headless, root.path().join("run")),
        (
            Session::Headed {
                wayland_display: "wayland-1".to_string(),
                runtime: PathBuf::from("/run/user/1000"),
            },
            PathBuf::from("/run/user/1000"),
        ),
    ];

    for (session, runtime) in cases {
        let environment = session.environment(&caller, root.path());
        assert_eq!(
            environment
                .iter()
                .find(|(name, _)| name == "XDG_RUNTIME_DIR")
                .map(|(_, value)| value.as_str()),
            runtime.to_str()
        );
        assert!(
            !environment
                .iter()
                .any(|(name, _)| name == "DISPLAY" || name == "WAYLAND_DISPLAY")
        );
    }
}

#[test]
fn decimal_identifiers_require_nonempty_ascii_digits() {
    for (value, expected) in [
        ("", false),
        ("7", true),
        ("007", true),
        ("-7", false),
        ("7 ", false),
        ("٧", false),
        ("7x", false),
    ] {
        assert_eq!(is_decimal(value), expected, "{value}");
    }
}

#[test]
fn the_display_errors_render_their_messages() {
    let error = DisplayError::DisplayTimeout {
        timeout: Duration::from_secs(3),
    };

    assert_eq!(
        error.to_string(),
        "weston did not print a display within 3 seconds."
    );
}
