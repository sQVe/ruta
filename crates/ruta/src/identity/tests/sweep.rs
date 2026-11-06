use super::*;
use crate::root::record_root_creation;

// A signaled process can look alive for a moment while it exits, so a spared process must stay
// alive for a while.
fn assert_keeps_running(child: &mut ChildGuard, message: &str) {
    assert!(
        !child.wait_for_exit(Duration::from_millis(500)),
        "{message}"
    );
}

fn spawn_session_leader(marker: Option<&Path>) -> ChildGuard {
    let mut command = Command::new("setsid");
    command
        .args(["sleep", "30"])
        .env_remove("RUTA_ROOT")
        .env("RUTA_TEST_ESCAPE", "1");

    if let Some(root) = marker {
        command.env("RUTA_ROOT", root.as_os_str());
    }

    let child = ChildGuard::spawn(&mut command);
    wait_until_exec_of_sleep(&child);

    child
}

// `setsid sleep 30` holds "sleep" in its command line before it execs, so wait for `sleep` itself.
fn wait_until_exec_of_sleep(child: &ChildGuard) {
    let named = poll_until(Duration::from_secs(2), || {
        let cmdline = fs::read(format!("/proc/{}/cmdline", child.id())).unwrap_or_default();

        Ok(cmdline.starts_with(b"sleep\0").then_some(()))
    })
    .expect("the name check cannot fail");

    assert!(named.is_some(), "the child execs sleep");
}

#[test]
fn stop_window_stops_a_marked_process_outside_the_app_group() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-escape");
    let other = crate::test_support::temporary_sandbox_root("stop-window-escape-other");
    record_root_creation(root.path()).expect("record the root creation");
    let mut escaped = spawn_session_leader(Some(root.path()));
    let mut foreign = spawn_session_leader(Some(other.path()));
    let mut unmarked = spawn_session_leader(None);

    stop_window(root.path()).expect("stop_window");

    assert!(
        escaped.wait_for_exit(Duration::from_secs(2)),
        "a process with this root's marker is stopped"
    );
    assert_keeps_running(&mut foreign, "another root's process keeps running");
    assert_keeps_running(&mut unmarked, "an unmarked process keeps running");
}

#[test]
fn stop_window_leaves_a_marked_process_that_started_before_the_root() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-older");
    let mut older = spawn_session_leader(Some(root.path()));
    // The start-time clock counts in ticks of at least a millisecond.
    std::thread::sleep(Duration::from_millis(50));
    record_root_creation(root.path()).expect("record the root creation");
    let mut app = spawn_with_marker(root.path());
    record_child(&root.path().join("pid"), &app);

    stop_window(root.path()).expect("stop_window");

    assert!(
        app.wait_for_exit(Duration::from_secs(2)),
        "the recorded app is stopped"
    );

    assert_keeps_running(
        &mut older,
        "a marked process older than the root keeps running",
    );
}

#[test]
fn stop_window_sweeps_nothing_without_a_root_creation_record() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-no-created");
    let mut marked = spawn_session_leader(Some(root.path()));

    stop_window(root.path()).expect("stop_window");

    assert_keeps_running(&mut marked, "without a record, nothing is swept");
}

#[test]
fn the_sweep_spares_ruta_and_its_ancestors() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("sweep-ancestors");
    record_root_creation(root.path()).expect("record the root creation");
    let mut command = Command::new("sh");
    command
        .args(["-c", "sleep 30 & sleep 30 & wait"])
        .env("RUTA_ROOT", root.path().as_os_str());
    let mut parent = ChildGuard::spawn_group(&mut command);
    let children = poll_until(Duration::from_secs(2), || {
        let children = child_pids(parent.id());

        Ok((children.len() == 2).then_some(children))
    })
    .expect("the child scan cannot fail")
    .expect("both children started");
    let (ruta, sibling) = (children[0], children[1]);

    stop_marked(root.path(), ruta, STOP_TIMEOUT).expect("stop_marked");

    assert_keeps_running(&mut parent, "an ancestor of ruta keeps running");
    assert!(
        process_start(ruta).is_some() && !process_is_zombie(ruta),
        "ruta itself keeps running"
    );
    let sibling_gone = poll_until(Duration::from_secs(2), || {
        Ok((process_start(sibling).is_none() || process_is_zombie(sibling)).then_some(()))
    })
    .expect("the exit check cannot fail");
    assert!(
        sibling_gone.is_some(),
        "a marked process that is not an ancestor is stopped"
    );

    stop_process_group(parent.id());
}

#[test]
fn stop_window_leaves_a_marked_process_that_started_in_the_boundary_tick() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-same-tick");
    let mut older = spawn_session_leader(Some(root.path()));
    let boundary = process_start(older.id()).expect("the older process has a start time");
    fs::write(root.path().join("created"), format!("{boundary}\n")).expect("write the boundary");
    let moved = poll_until(Duration::from_secs(2), || {
        Ok((crate::process_stat::boot_ticks_now() > boundary).then_some(()))
    })
    .expect("the clock check cannot fail");
    assert!(moved.is_some(), "the clock moved past the boundary");
    let mut newer = spawn_session_leader(Some(root.path()));

    stop_window(root.path()).expect("stop_window");

    assert_keeps_running(
        &mut older,
        "a marked process from the boundary tick keeps running",
    );
    assert!(
        newer.wait_for_exit(Duration::from_secs(2)),
        "a marked process from after the boundary is stopped"
    );
}

#[test]
fn stop_window_leaves_a_process_whose_marker_hides_in_another_value() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("stop-window-hidden-marker");
    record_root_creation(root.path()).expect("record the root creation");
    let hidden = format!("x\nRUTA_ROOT={}\n", root.path().display());
    let mut command = Command::new("setsid");
    command
        .args(["sleep", "30"])
        .env_remove("RUTA_ROOT")
        .env("RUTA_TEST_OTHER", hidden);
    let mut impostor = ChildGuard::spawn(&mut command);
    wait_until_exec_of_sleep(&impostor);

    stop_window(root.path()).expect("stop_window");

    assert_keeps_running(
        &mut impostor,
        "a marker inside another variable's value does not count",
    );
}

fn candidate(pid: u32, owner: u32, start: u64, state: &str) -> SweepCandidate {
    SweepCandidate {
        pid,
        owner,
        stat: crate::process_stat::ProcessStat {
            state: state.to_string(),
            parent: 1,
            group: pid,
            start,
        },
    }
}

#[test]
fn may_sweep_takes_only_new_live_processes_of_this_user() {
    crate::test_support::in_test_namespace!();

    let (user, created, spared) = (1000, 500, [7, 8]);
    let cases = [
        (
            "a new process of this user",
            candidate(20, 1000, 501, "S"),
            true,
        ),
        (
            "another user's process",
            candidate(21, 1001, 501, "S"),
            false,
        ),
        ("root's process", candidate(22, 0, 501, "S"), false),
        (
            "a process from the creation tick",
            candidate(23, 1000, 500, "S"),
            false,
        ),
        (
            "a process from before the root",
            candidate(24, 1000, 499, "S"),
            false,
        ),
        ("ruta or an ancestor", candidate(7, 1000, 501, "S"), false),
        ("a zombie", candidate(25, 1000, 501, "Z"), false),
    ];

    for (case, candidate, expected) in cases {
        assert_eq!(
            may_sweep(&candidate, user, created, &spared),
            expected,
            "{case}"
        );
    }
}

#[test]
fn environment_holds_marker_matches_only_a_whole_entry() {
    crate::test_support::in_test_namespace!();

    let root = Path::new("/var/tmp/ruta-x");
    let cases: [(&str, &[u8], bool); 5] = [
        ("the entry", b"A=1\0RUTA_ROOT=/var/tmp/ruta-x\0B=2\0", true),
        (
            "the last entry without a NUL",
            b"A=1\0RUTA_ROOT=/var/tmp/ruta-x",
            true,
        ),
        (
            "inside another value",
            b"A=1\nRUTA_ROOT=/var/tmp/ruta-x\nB\0",
            false,
        ),
        ("another root", b"RUTA_ROOT=/var/tmp/ruta-x2\0", false),
        ("a longer value", b"RUTA_ROOT=/var/tmp/ruta-x/\0", false),
    ];

    for (case, environment, expected) in cases {
        assert_eq!(
            environment_holds_marker(environment, root),
            expected,
            "{case}"
        );
    }
}
