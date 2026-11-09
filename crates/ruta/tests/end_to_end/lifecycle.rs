use std::fs;

use crate::support::*;

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn up_shoots_logs_and_goes_down() {
    in_test_namespace!();

    let project = Project::new(TITLE);
    let sandbox = project.sandbox("up");

    sandbox.succeed(&["up", "--", "--headed", "extra"]);

    let ready = fs::read_to_string(sandbox.root().join("ready.json")).expect("read ready.json");
    let ready: serde_json::Value = serde_json::from_str(&ready).expect("ready.json is JSON");
    assert_eq!(
        ready,
        serde_json::json!({
            "title": TITLE,
            "width": WINDOW_SIZE.0,
            "height": WINDOW_SIZE.1,
            "arguments": ["--headed", "extra"],
        }),
        "ready.json holds the app's ready line"
    );

    assert_eq!(
        sandbox.shot_size(),
        WINDOW_SIZE,
        "the shot has the window size"
    );

    let log = wait_for_log(&sandbox, "ruta-test-app: running");
    for expected in [
        "ruta-test-app: starting\n",
        "\"title\":\"ruta test app\"",
        "ruta-test-app: running\n",
    ] {
        assert!(log.contains(expected), "the log holds {expected:?}: {log}");
    }

    assert_eq!(sandbox.succeed(&["opened"]), "", "no link was opened");

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn failed_up_leaves_no_root_and_no_process() {
    in_test_namespace!();

    let project = Project::new("no window has this title");
    let sandbox = project.sandbox("bad");

    let output = sandbox.run(&["up"]);

    assert!(
        !output.status.success(),
        "up without a matching window fails"
    );
    sandbox.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn two_sandboxes_run_side_by_side() {
    in_test_namespace!();

    let project = Project::new(TITLE);
    let first = project.sandbox("one");
    let second = project.sandbox("two");

    first.succeed(&["up"]);
    second.succeed(&["up"]);

    assert_ne!(first.root(), second.root());
    assert_eq!(first.shot_size(), WINDOW_SIZE, "the first shot");
    assert_eq!(second.shot_size(), WINDOW_SIZE, "the second shot");

    first.succeed(&["down"]);
    first.assert_gone();
    assert_eq!(second.shot_size(), WINDOW_SIZE, "the second stays up");

    second.succeed(&["down"]);
    second.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn command_ready_up_writes_no_ready_json() {
    in_test_namespace!();

    let project = Project::with_ready(TITLE, COMMAND_READY);
    let sandbox = project.sandbox("command");

    sandbox.succeed(&["up"]);

    assert!(
        !sandbox.root().join("ready.json").exists(),
        "command-ready mode writes no ready.json"
    );
    assert_eq!(
        sandbox.shot_size(),
        WINDOW_SIZE,
        "the shot has the window size"
    );

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_failed_up_reports_the_last_line_the_app_printed() {
    in_test_namespace!();

    let project = Project::with_config(&config_for_app(
        r#"["sh", "-c", "echo 'ruta-test: the setup is broken'; exit 3"]"#,
        TITLE,
        r#"line = "json""#,
    ));
    let sandbox = project.sandbox("broken-app");

    let output = sandbox.run(&["up"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "up fails when the app exits");
    assert!(
        stderr.contains("ruta-test: the setup is broken"),
        "the error holds the app's last line: {stderr}"
    );
    sandbox.assert_gone();
}
