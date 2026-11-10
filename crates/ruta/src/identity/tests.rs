use std::io::BufRead as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::*;
use crate::test_support::{ChildGuard, stop_process_group};

fn spawn_sleep() -> ChildGuard {
    let mut command = Command::new("sleep");
    command.arg("30");

    ChildGuard::spawn(&mut command)
}

pub(crate) fn spawn_ready_sleep() -> ChildGuard {
    let child = spawn_sleep();
    child.wait_until_ready("sleep", Duration::from_secs(2));

    child
}

pub(crate) fn spawn_without_marker() -> ChildGuard {
    let mut command = Command::new("sleep");
    command.arg("30").env_remove("RUTA_ROOT");
    let child = ChildGuard::spawn(&mut command);
    child.wait_until_ready("sleep", Duration::from_secs(2));

    child
}

pub(crate) fn spawn_with_marker(root: &Path) -> ChildGuard {
    let mut command = Command::new("sleep");
    command.arg("30").env("RUTA_ROOT", root.as_os_str());
    let child = ChildGuard::spawn_group(&mut command);
    child.wait_until_ready("sleep", Duration::from_secs(2));

    child
}

fn spawn_fake_weston(child_name: &str) -> (ChildGuard, u32) {
    let mut command = Command::new("bash");
    command.args(["-c", &format!("exec -a \"{child_name}\" sleep 30 & wait")]);
    let parent = ChildGuard::spawn_group(&mut command);
    let child = wait_for_child_named(parent.id(), child_name, Duration::from_secs(2));

    (parent, child)
}

fn wait_for_child_named(parent: u32, name: &str, timeout: Duration) -> u32 {
    let found = poll_until(timeout, || {
        for pid in child_pids(parent) {
            let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
                continue;
            };
            let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");

            if cmdline.contains(name) {
                return Ok(Some(pid));
            }
        }

        Ok(None)
    })
    .expect("the child scan cannot fail");

    found.expect("the named child appeared in time")
}

fn write_weston_record(root: &Path, pid: u32) {
    let started = process_start(pid).expect("the fake weston has a start time");

    fs::write(root.join("weston.pid"), format!("{pid} {started}\n")).expect("write weston.pid");
}

fn spawn_term_ignorer() -> ChildGuard {
    let mut command = Command::new("sh");
    command.args(["-c", "trap \"\" TERM; echo ready; exec sleep 30"]);
    command.stdout(Stdio::piped());

    let mut child = ChildGuard::spawn(&mut command);
    let stdout = child.take_stdout();
    let mut reader = std::io::BufReader::new(stdout);
    let mut ready = String::new();
    reader.read_line(&mut ready).expect("read the ready line");

    assert_eq!(ready.trim(), "ready", "the child installed its TERM trap");

    child.wait_until_ready("sleep", Duration::from_secs(2));

    child
}

fn spawn_exiting() -> ChildGuard {
    ChildGuard::spawn(&mut Command::new("true"))
}

pub(crate) fn record_child(path: &Path, child: &ChildGuard) -> u64 {
    let started = process_start(child.id()).expect("the child has a start time");

    fs::write(path, format!("{} {started}\n", child.id())).expect("write the record");

    started
}

fn wait_for_zombie(pid: u32, timeout: Duration) {
    let ready = poll_until(timeout, || Ok(process_is_zombie(pid).then_some(())))
        .expect("the zombie check cannot fail");

    assert!(ready.is_some(), "the child did not become a zombie in time");
}

pub(crate) mod stop;
mod sweep;

#[test]
fn process_start_is_none_for_a_missing_pid() {
    assert_eq!(process_start(u32::MAX), None);
}

#[test]
fn the_stat_parent_names_the_test_process() {
    crate::test_support::in_test_namespace!();

    let child = spawn_sleep();

    assert_eq!(
        ProcessStat::read(child.id()).map(|stat| stat.parent),
        Some(std::process::id())
    );
}

#[test]
fn child_pids_finds_a_child_and_not_a_stranger() {
    crate::test_support::in_test_namespace!();

    let child = spawn_sleep();

    assert!(child_pids(std::process::id()).contains(&child.id()));
    assert!(!child_pids(u32::MAX).contains(&child.id()));
}

#[test]
fn record_process_writes_the_pid_and_start_time() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let child = spawn_ready_sleep();

    record_process(&path, child.id(), "sleep", Duration::from_secs(1)).expect("record_process");

    let (pid, started) = read_record(&path).expect("the record exists");
    assert_eq!(pid, child.id());
    assert_eq!(
        started,
        process_start(child.id()).expect("the child has a start time")
    );
}

#[test]
fn record_process_rejects_a_wrong_name_and_writes_no_file() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let child = spawn_ready_sleep();

    let error = record_process(
        &path,
        child.id(),
        "not-the-name",
        Duration::from_millis(200),
    )
    .expect_err("a wrong name must fail");

    assert!(
        matches!(
            error.downcast_ref::<IdentityError>(),
            Some(IdentityError::UnexpectedProcess { pid, expected, actual })
                if *pid == child.id()
                    && expected == "not-the-name"
                    && actual.starts_with("sleep")
        ),
        "unexpected error: {error:#}"
    );
    assert!(!path.exists(), "no record is written");
}

#[test]
fn record_process_rejects_a_missing_pid_and_writes_no_file() {
    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");

    let error = record_process(&path, u32::MAX, "sleep", Duration::from_secs(1))
        .expect_err("a missing pid must fail");

    assert!(
        matches!(
            error.downcast_ref::<IdentityError>(),
            Some(IdentityError::MissingProcess { pid, expected })
                if *pid == u32::MAX && expected == "sleep"
        ),
        "unexpected error: {error:#}"
    );
    assert!(!path.exists(), "no record is written");
}

#[test]
fn record_process_reports_an_empty_name_in_time() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let child = spawn_exiting();
    let pid = child.id();

    wait_for_zombie(pid, Duration::from_secs(3));

    let error = record_process(&path, pid, "sleep", Duration::from_millis(200))
        .expect_err("an unnamed process must time out");

    assert!(
        matches!(
            error.downcast_ref::<IdentityError>(),
            Some(IdentityError::DidNotNameItself { pid: recorded, expected })
                if *recorded == pid && expected == "sleep"
        ),
        "unexpected error: {error:#}"
    );
    assert!(!path.exists(), "no record is written");
}

#[test]
fn record_xwayland_records_a_matching_child() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("xwayland-match");
    let path = root.path().join("xwayland.pid");
    let (weston, child) = spawn_fake_weston("Xwayland :97");
    write_weston_record(root.path(), weston.id());

    record_xwayland(root.path(), ":97", Duration::from_secs(1)).expect("record_xwayland");

    let (pid, _) = read_record(&path).expect("the xwayland record");
    assert_eq!(pid, child);
}

fn assert_xwayland_not_recorded(error: &anyhow::Error, display: &str) {
    assert!(
        matches!(
            error.downcast_ref::<IdentityError>(),
            Some(IdentityError::XwaylandNotRecorded { display: recorded, timeout })
                if recorded == display && *timeout == Duration::from_millis(200)
        ),
        "unexpected error: {error:#}"
    );
}

#[test]
fn record_xwayland_ignores_a_different_display() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("xwayland-display");
    let path = root.path().join("xwayland.pid");
    let (weston, _child) = spawn_fake_weston("Xwayland :97");
    write_weston_record(root.path(), weston.id());

    let error = record_xwayland(root.path(), ":98", Duration::from_millis(200))
        .expect_err("a different display is not Xwayland for :98");

    assert_xwayland_not_recorded(&error, ":98");
    assert!(!path.exists(), "a different display is not recorded");
}

#[test]
fn record_xwayland_ignores_a_child_with_another_name() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("xwayland-name");
    let path = root.path().join("xwayland.pid");
    let (weston, _child) = spawn_fake_weston("OtherProcess");
    write_weston_record(root.path(), weston.id());

    let error = record_xwayland(root.path(), ":97", Duration::from_millis(200))
        .expect_err("a child with another name is not Xwayland");

    assert_xwayland_not_recorded(&error, ":97");
    assert!(!path.exists(), "a child with another name is not recorded");
}

#[test]
fn record_process_waits_for_the_command_line_to_change() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let mut command = Command::new("bash");
    command.args(["-c", "sleep 0.3; exec -a \"$EXPECTED\" sleep 30"]);
    command.env("EXPECTED", "expected-name");
    let child = ChildGuard::spawn(&mut command);

    record_process(&path, child.id(), "expected-name", Duration::from_secs(2))
        .expect("record_process waits for the exec");

    let (pid, _) = read_record(&path).expect("the record exists");
    assert_eq!(pid, child.id());
}

#[test]
fn the_identity_errors_render_their_messages() {
    let cases = [
        (
            IdentityError::DidNotStop {
                pid: 7,
                path: PathBuf::from("/tmp/x"),
            },
            "The process 7 did not stop; keeping /tmp/x",
        ),
        (
            IdentityError::UnexpectedProcess {
                pid: 7,
                expected: "weston".to_string(),
                actual: "other".to_string(),
            },
            "The process 7 is not the expected weston. Actual: other",
        ),
        (
            IdentityError::MissingProcess {
                pid: 7,
                expected: "weston".to_string(),
            },
            "The weston process 7 is gone before it could be recorded.",
        ),
        (
            IdentityError::DidNotNameItself {
                pid: 7,
                expected: "weston".to_string(),
            },
            "The weston process 7 did not name itself in time.",
        ),
        (
            IdentityError::Unmarked { pid: 7 },
            "The app process 7 does not hold this sandbox's RUTA_ROOT.",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

#[test]
fn record_matches_rejects_a_changed_start_time() {
    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let pid = std::process::id();
    let started = process_start(pid).expect("the test process has a start time");

    fs::write(&path, format!("{pid} {started}\n")).expect("write the matching record");
    assert!(record_matches(&path));

    fs::write(&path, format!("{pid} {}\n", started + 1)).expect("write the changed record");
    assert!(!record_matches(&path));
}

#[test]
fn record_marked_process_records_a_marked_child_whatever_its_name() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("record-marked");
    let path = root.path().join("pid");
    let child = spawn_with_marker(root.path());

    record_marked_process(&path, child.id(), root.path(), Duration::from_secs(1))
        .expect("the marked child is recorded");

    assert_eq!(
        read_record(&path),
        Some((
            child.id(),
            process_start(child.id()).expect("the child has a start time")
        ))
    );
}

#[test]
fn record_marked_process_refuses_a_child_without_the_marker_and_writes_no_file() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("record-unmarked");
    let other = crate::test_support::temporary_sandbox_root("record-other");
    let path = root.path().join("pid");

    for (case, child) in [
        ("no marker", spawn_without_marker()),
        ("another root", spawn_with_marker(other.path())),
    ] {
        let error =
            record_marked_process(&path, child.id(), root.path(), Duration::from_millis(200))
                .expect_err("an unmarked child is refused");

        assert!(
            matches!(
                error.downcast_ref::<IdentityError>(),
                Some(IdentityError::Unmarked { pid }) if *pid == child.id()
            ),
            "{case}: unexpected error: {error:#}"
        );
        assert!(!path.exists(), "{case}: no record is written");
    }
}

#[test]
fn record_marked_process_reports_a_missing_or_exited_process_at_once() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("record-gone");
    let path = root.path().join("pid");
    let exited = spawn_exiting();
    wait_for_zombie(exited.id(), Duration::from_secs(3));

    for (case, pid) in [("missing", u32::MAX), ("zombie", exited.id())] {
        let started = std::time::Instant::now();

        let error = record_marked_process(&path, pid, root.path(), Duration::from_secs(5))
            .expect_err("a gone process is refused");

        assert!(
            matches!(
                error.downcast_ref::<IdentityError>(),
                Some(IdentityError::MissingProcess { pid: recorded, .. }) if *recorded == pid
            ),
            "{case}: unexpected error: {error:#}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{case}: reported after {:?}",
            started.elapsed()
        );
        assert!(!path.exists(), "{case}: no record is written");
    }
}
