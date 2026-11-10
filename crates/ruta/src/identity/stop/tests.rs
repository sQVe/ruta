use std::fs;
use std::process::Command;

use super::*;
use crate::identity::tests::{
    record_changed_start, record_child, spawn_exiting, spawn_ready_sleep, spawn_term_ignorer,
    spawn_with_marker, spawn_without_marker, wait_for_child_named, wait_for_zombie,
};
use crate::identity::{child_pids, environment_holds_marker};
use crate::process_stat::command_line;
use crate::test_support::{ChildGuard, stop_process_group};

mod sweep;

#[test]
fn stop_window_leaves_a_child_without_the_marker_running() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-plain");
    let path = root.path().join("pid");
    let mut child = spawn_without_marker();
    record_child(&path, &child);

    stop_window(root.path()).expect("stop_window");

    assert!(
        child.is_running(),
        "a child without the marker must keep running"
    );
    assert!(!path.exists(), "the record is removed");
}

#[test]
fn stop_window_stops_a_child_with_the_matching_marker() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-ours");
    let path = root.path().join("pid");
    let mut child = spawn_with_marker(root.path());
    record_child(&path, &child);

    stop_window(root.path()).expect("stop_window");

    assert!(
        child.wait_for_exit(Duration::from_secs(2)),
        "the marked child is stopped"
    );
    assert!(!path.exists(), "the record is removed");
}

#[test]
fn stop_window_leaves_a_leader_whose_start_time_changed_running() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-reused");
    let path = root.path().join("pid");
    let mut child = spawn_with_marker(root.path());
    record_changed_start(&path, child.id());

    stop_window(root.path()).expect("stop_window");

    assert!(
        !child.wait_for_exit(Duration::from_millis(500)),
        "a reused PID in the leader's place keeps running"
    );
    assert!(!path.exists(), "the stale record is removed");
}

#[test]
fn stop_window_leaves_group_members_without_this_roots_marker_running() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-root");
    let other = crate::test_support::temporary_sandbox_root("stop-window-other");
    let path = root.path().join("pid");
    let other_marker = marker(other.path());
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            r#"env -u RUTA_ROOT RUTA_TEST_MEMBER=plain sleep 30 & env "$OTHER" sleep 30 & wait"#,
        ])
        .env("RUTA_ROOT", root.path().as_os_str())
        .env("OTHER", &other_marker);
    let mut leader = ChildGuard::spawn_group(&mut command);
    let members = poll_until(Duration::from_secs(2), || {
        let plain = crate::test_support::live_processes_with("RUTA_TEST_MEMBER=plain");
        let foreign = crate::test_support::live_processes_with(&other_marker);
        let started = !plain.is_empty() && !foreign.is_empty();

        Ok(started.then_some([plain, foreign].concat()))
    })
    .expect("the member scan cannot fail")
    .expect("both members started");
    record_child(&path, &leader);

    stop_window(root.path()).expect("stop_window");

    assert!(
        leader.wait_for_exit(Duration::from_secs(2)),
        "the recorded leader is stopped"
    );
    for member in members {
        assert!(
            ProcessStat::read_live(member).is_some(),
            "the member {member} without this root's marker keeps running"
        );
    }
    assert!(!path.exists(), "the record is removed");

    stop_process_group(leader.id());
}

#[test]
fn stop_recorded_removes_a_malformed_record() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    fs::write(&path, "not an identity").expect("write the malformed record");

    stop_recorded(&path).expect("the malformed record is discarded");

    assert!(!path.exists());
}

#[test]
fn stop_recorded_stops_a_recorded_child() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let mut child = spawn_ready_sleep();
    record_child(&path, &child);

    stop_recorded(&path).expect("stop_recorded");

    assert!(
        child.wait_for_exit(Duration::from_secs(2)),
        "the recorded child is stopped"
    );
    assert!(!path.exists(), "the record is removed");
}

#[test]
fn stop_recorded_leaves_a_child_with_a_changed_start_time_running() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let mut child = spawn_ready_sleep();
    record_changed_start(&path, child.id());

    stop_recorded(&path).expect("stop_recorded");

    assert!(
        child.is_running(),
        "a changed start time must not stop the child"
    );
    assert!(!path.exists(), "the stale record is removed");
}

#[test]
fn stop_process_stops_a_child() {
    crate::test_support::in_test_namespace!();

    let mut child = spawn_ready_sleep();
    let identity = ProcessIdentity::current(child.id()).expect("the child has an identity");

    let stopped = stop_process(identity, Duration::from_secs(2), open_pidfd).expect("stop_process");

    assert!(stopped);
    assert!(
        child.wait_for_exit(Duration::from_secs(2)),
        "a plain child is stopped by TERM"
    );
}

#[test]
fn stop_process_reports_a_child_that_ignores_term() {
    crate::test_support::in_test_namespace!();

    let mut child = spawn_term_ignorer();
    let identity = ProcessIdentity::current(child.id()).expect("the child has an identity");

    let stopped =
        stop_process(identity, Duration::from_millis(300), open_pidfd).expect("the wait succeeds");

    assert!(!stopped);
    assert!(child.is_running(), "the child keeps running");
}

#[test]
fn stop_recorded_keeps_the_record_when_the_child_ignores_term() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let mut child = spawn_term_ignorer();
    let identity = record_child(&path, &child);

    let error = stop_recorded(&path).expect_err("the child ignores TERM");

    assert!(matches!(error.downcast_ref::<IdentityError>(),
        Some(IdentityError::DidNotStop { pid, path: recorded })
            if *pid == child.id() && recorded == &path));
    assert_eq!(ProcessIdentity::read(&path), Some(identity));
    assert!(child.is_running(), "the child keeps running");
}

#[test]
fn stop_recorded_treats_a_zombie_as_stopped() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let child = spawn_exiting();
    record_child(&path, &child);
    wait_for_zombie(child.id(), Duration::from_secs(3));

    stop_recorded(&path).expect("the zombie is stopped");

    assert!(!path.exists(), "a zombie's record is removed");
}

#[test]
fn stop_recorded_removes_the_record_after_a_failed_open_with_changed_identity() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let mut child = spawn_ready_sleep();
    record_child(&path, &child);

    stop_recorded_with(&path, |_| {
        child.reap();
        Err(std::io::Error::from(rustix::io::Errno::NOSYS))
    })
    .expect("a changed identity is treated as stopped");

    assert!(
        !path.exists(),
        "the stale record is removed after the failed open"
    );
}

#[test]
fn stop_recorded_keeps_the_record_after_a_failed_open_with_unchanged_identity() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let mut child = spawn_ready_sleep();
    record_child(&path, &child);
    let before = fs::read(&path).expect("read the record before stopping");

    let error = stop_recorded_with(&path, |_| {
        Err(std::io::Error::from(rustix::io::Errno::NOSYS))
    })
    .expect_err("an open failure with a live process is an error");

    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::raw_os_error),
        Some(rustix::io::Errno::NOSYS.raw_os_error())
    );
    assert_eq!(fs::read(&path).expect("the record remains"), before);
    assert!(child.is_running(), "an open failure sends no signal");
}

#[test]
fn stop_process_skips_a_mismatched_start_time() {
    crate::test_support::in_test_namespace!();

    let mut identity =
        ProcessIdentity::current(std::process::id()).expect("the test has an identity");
    identity.started += 1;

    assert!(
        stop_process(identity, Duration::from_secs(1), open_pidfd).expect("the identity changed")
    );
}

#[test]
fn stop_recorded_removes_the_record_after_a_successful_open_with_changed_identity() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let mut child = spawn_ready_sleep();
    let mut other = spawn_ready_sleep();
    record_child(&path, &child);
    let other_pid =
        rustix::process::Pid::from_raw(other.id() as i32).expect("the child PID is positive");

    stop_recorded_with(&path, |_| {
        let pidfd = open_pidfd(other_pid)?;
        child.reap();
        Ok(pidfd)
    })
    .expect("the original process is gone");

    assert!(
        !path.exists(),
        "the stale record is removed after the successful open"
    );
    assert!(
        !other.wait_for_exit(Duration::from_millis(300)),
        "the opened process must not be signaled"
    );
}

fn spawn_marked_group(root: &Path) -> (ChildGuard, u32) {
    let mut command = Command::new("bash");
    command
        .args(["-c", "exec -a ruta-group-member sleep 30 & wait"])
        .env("RUTA_ROOT", root.as_os_str());
    let leader = ChildGuard::spawn_group(&mut command);
    let member = wait_for_child_named(leader.id(), "ruta-group-member", Duration::from_secs(2));

    (leader, member)
}

#[test]
fn stop_window_stops_the_whole_process_group() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-group");
    let path = root.path().join("pid");
    let (mut leader, member) = spawn_marked_group(root.path());
    record_child(&path, &leader);

    stop_window(root.path()).expect("stop_window");

    assert!(
        leader.wait_for_exit(Duration::from_secs(2)),
        "the group leader is stopped"
    );
    assert_eq!(
        crate::test_support::live_processes_with(&marker(root.path())),
        Vec::<u32>::new(),
        "the group member {member} is stopped"
    );
    assert!(!path.exists(), "the record is removed");
}

#[test]
fn stop_recorded_signals_only_the_recorded_process() {
    crate::test_support::in_test_namespace!();

    let directory = crate::test_support::temporary_directory();
    let path = directory.path().join("record");
    let (mut leader, member) = spawn_marked_group(directory.path());
    record_child(&path, &leader);

    stop_recorded(&path).expect("stop_recorded");

    assert!(
        leader.wait_for_exit(Duration::from_secs(2)),
        "the recorded process is stopped"
    );
    assert!(
        ProcessIdentity::current(member).is_some(),
        "the other group member keeps running"
    );

    stop_process_group(leader.id());
}

const STUBBORN_NAME: &str = "ruta-stubborn";

pub(crate) fn spawn_stubborn_group(root: &Path) -> ChildGuard {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            r#"bash -c 'trap "" TERM; exec -a ruta-stubborn sleep 30' & wait"#,
        ])
        .env("RUTA_ROOT", root.as_os_str());
    let leader = ChildGuard::spawn_group(&mut command);

    // The member names itself only after exec, so its TERM trap is in place by then.
    let named = poll_until(Duration::from_secs(2), || {
        Ok(child_pids(leader.id())
            .into_iter()
            .find(|pid| command_line(*pid).is_ok_and(|cmdline| cmdline.starts_with(STUBBORN_NAME))))
    })
    .expect("the member scan cannot fail");
    assert!(named.is_some(), "the stubborn member started");

    leader
}

fn marker(root: &Path) -> String {
    format!("RUTA_ROOT={}", root.display())
}

#[test]
fn stop_window_kills_a_group_member_that_ignores_term() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-stubborn");
    let path = root.path().join("pid");
    let leader = spawn_stubborn_group(root.path());
    record_child(&path, &leader);

    stop_window(root.path()).expect("stop_window");

    assert_eq!(
        crate::test_support::live_processes_with(&marker(root.path())),
        Vec::<u32>::new(),
        "no member of the group is alive"
    );
    assert!(!path.exists(), "the record is removed");
}

#[test]
fn stop_window_stops_the_members_of_a_leader_that_already_exited() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-orphans");
    let path = root.path().join("pid");
    let mut command = Command::new("sh");
    command
        .args(["-c", "sleep 30 & exit 0"])
        .env("RUTA_ROOT", root.path().as_os_str());
    let leader = ChildGuard::spawn_group(&mut command);
    record_child(&path, &leader);
    wait_for_zombie(leader.id(), Duration::from_secs(3));
    let members = poll_until(Duration::from_secs(2), || {
        let alive = crate::test_support::live_processes_with(&marker(root.path()));

        Ok((!alive.is_empty()).then_some(alive))
    })
    .expect("the member scan cannot fail");
    assert!(members.is_some(), "the member outlives its leader");

    stop_window(root.path()).expect("stop_window");

    assert_eq!(
        crate::test_support::live_processes_with(&marker(root.path())),
        Vec::<u32>::new(),
        "the members of an exited leader are stopped"
    );
    assert!(!path.exists(), "the record is removed");
}
