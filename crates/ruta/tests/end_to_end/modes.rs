use std::time::Duration;

use crate::support::*;

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn panic_mode_panics_only_after_its_sequence() {
    in_test_namespace!();

    let command = app_command(&["--panic-after", "alpha,beta"]);
    let project = Project::new(&Config {
        command: Some(&command),
        ..Config::default()
    });
    let sandbox = project.sandbox("mode-panic");
    sandbox.succeed(&["up"]);
    let pid = sandbox.recorded_pid("pid");

    sandbox.succeed(&["click", "80", "40"]);
    let log = sandbox.wait_for_log("ruta-test-app: press name=alpha");

    assert!(!log.contains("panicking"), "{log}");
    assert!(process_runs(pid), "the app still runs: {log}");

    sandbox.succeed(&["click", "80", "100"]);
    let log = sandbox.wait_for_log("ruta-test-app: panicking after the button sequence alpha,beta");

    assert!(
        log.contains("ruta-test-app: panicking after the button sequence alpha,beta"),
        "{log}"
    );
    assert!(
        poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
            !process_runs(pid)
        }),
        "the app stopped after the panic: {log}"
    );

    sandbox.succeed(&["down"]);
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn exit_mode_exits_only_after_its_sequence() {
    in_test_namespace!();

    let command = app_command(&["--exit-after", "beta,alpha"]);
    let project = Project::new(&Config {
        command: Some(&command),
        ..Config::default()
    });
    let sandbox = project.sandbox("mode-exit");
    sandbox.succeed(&["up"]);
    let pid = sandbox.recorded_pid("pid");

    sandbox.succeed(&["click", "80", "100"]);
    let log = sandbox.wait_for_log("ruta-test-app: press name=beta");

    assert!(!log.contains("exiting"), "{log}");
    assert!(process_runs(pid), "the app still runs: {log}");

    sandbox.succeed(&["click", "80", "40"]);
    let log = sandbox.wait_for_log("ruta-test-app: exiting after beta,alpha");

    assert!(
        log.contains("ruta-test-app: exiting after beta,alpha"),
        "{log}"
    );
    assert!(
        poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
            !process_runs(pid)
        }),
        "the app stopped after the exit: {log}"
    );

    sandbox.succeed(&["down"]);
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn normal_mode_survives_the_sequence() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("mode-normal");
    sandbox.succeed(&["up"]);
    let pid = sandbox.recorded_pid("pid");

    sandbox.succeed(&["click", "80", "40"]);
    sandbox.wait_for_log("ruta-test-app: press name=alpha");
    sandbox.succeed(&["click", "80", "100"]);
    let log = sandbox.wait_for_log("ruta-test-app: press name=beta");

    assert!(!log.contains("panicking"), "{log}");
    assert!(!log.contains("exiting"), "{log}");
    assert!(process_runs(pid), "the app still runs: {log}");

    sandbox.succeed(&["down"]);
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn child_launcher_leaves_the_window_to_its_child() {
    in_test_namespace!();

    let command = app_command(&["--launch-child"]);
    let project = Project::new(&Config {
        command: Some(&command),
        ..Config::default()
    });
    let sandbox = project.sandbox("mode-child");
    sandbox.succeed(&["up"]);
    sandbox.wait_for_log("ruta-test-app: running");
    let pid = sandbox.recorded_pid("pid");

    assert!(
        poll_until(Duration::from_secs(10), Duration::from_millis(100), || {
            !process_runs(pid)
        }),
        "the launcher stopped"
    );

    sandbox.succeed(&["status"]);
    assert_eq!(
        sandbox.shot_size(),
        WINDOW_SIZE,
        "the child keeps the window"
    );

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}

fn app_command(extra: &[&str]) -> String {
    let app = test_app().to_str().expect("the test app path is UTF-8");
    let mut command = vec![
        format!("\"{app}\""),
        format!("\"{TITLE}\""),
        format!("\"{}\"", WINDOW_SIZE.0),
        format!("\"{}\"", WINDOW_SIZE.1),
    ];

    command.extend(extra.iter().map(|argument| format!("\"{argument}\"")));

    format!("[{}]", command.join(", "))
}

fn process_runs(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some((_, rest)) = stat.rsplit_once(") ") else {
        return false;
    };

    !rest.starts_with('Z')
}
