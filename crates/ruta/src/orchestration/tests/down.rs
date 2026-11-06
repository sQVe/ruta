use super::*;
use crate::app_start::start_app;
use crate::identity::process_start;
use crate::identity::tests::stop::spawn_stubborn_group;
use crate::test_support::live_processes_with;

#[test]
fn down_succeeds_for_an_absent_root() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let root = folder.path().join("ruta-absent");

    down("test", &root).expect("an absent sandbox is already down");

    assert!(!root.exists());
}

#[test]
fn down_removes_a_private_root_without_records() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("down");
    fs::write(root.path().join("arguments"), "[]\n").expect("write the arguments");

    down("test", root.path()).expect("the sandbox goes down");

    assert!(!root.path().exists());
}

#[test]
fn down_stops_a_member_that_ignores_term_before_it_removes_the_root() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("down-stubborn");
    let marker = format!("RUTA_ROOT={}", root.path().display());
    let leader = spawn_stubborn_group(root.path());
    record_child(&root.path().join("pid"), &leader);

    down("test", root.path()).expect("the sandbox goes down");

    assert_eq!(
        live_processes_with(&marker),
        Vec::<u32>::new(),
        "no member of the app's group is alive"
    );
    assert!(!root.path().exists(), "the root is removed");
}

#[test]
fn down_stops_a_recorded_app_that_dropped_the_marker() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("down-unmarked");
    let launcher = root.path().join("launch");
    // The pause lets ruta record the launcher while it still holds the marker.
    fs::write(&launcher, "#!/bin/sh\nsleep 0.3\nexec env -i sleep 30\n")
        .expect("write the launcher");
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755))
        .expect("make the launcher runnable");
    let mut config = test_config("test");
    config.folder = root.path().to_path_buf();
    config.app.command = CommandLine {
        program: launcher,
        arguments: Vec::new(),
    };
    config.ready = Ready::Command(CommandLine {
        program: PathBuf::from("true"),
        arguments: Vec::new(),
    });
    start_app(root.path(), &config, &[], ":999").expect("the app starts");
    let recorded = fs::read_to_string(root.path().join("pid")).expect("the app record");
    let pid: u32 = recorded
        .split_whitespace()
        .next()
        .and_then(|pid| pid.parse().ok())
        .expect("a recorded PID");
    let dropped = crate::process::poll_until(Duration::from_secs(3), || {
        let environment = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();

        Ok(environment.is_empty().then_some(()))
    })
    .expect("the environment check cannot fail");
    assert!(dropped.is_some(), "the app dropped its environment");

    down("test", root.path()).expect("the sandbox goes down");

    let alive = process_start(pid).is_some() && !crate::identity::process_is_zombie(pid);
    assert!(!alive, "the recorded app {pid} is stopped");
    assert!(!root.path().exists(), "the root is removed");
}
