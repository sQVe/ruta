use std::process::{Command, Stdio};

use crate::support::*;

// A process that leaves its process group with `setsid`, then lives on.
const ESCAPE: &str = "setsid sleep 300 >/dev/null 2>&1 < /dev/null &";

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_failed_up_stops_a_ready_check_child_that_left_its_group() {
    in_test_namespace!();

    let project = Project::with_config(&config_for_app(
        r#"["sleep", "2"]"#,
        TITLE,
        &format!(r#"command = ["sh", "-c", "{ESCAPE} exit 1"]"#),
    ));
    let sandbox = project.sandbox("escaped-probe");

    let output = sandbox.run(&["up"]);

    assert!(!output.status.success(), "up fails when the app exits");
    sandbox.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn down_stops_an_app_child_that_left_its_group_and_spares_other_sandboxes() {
    in_test_namespace!();

    let app = test_app().to_str().expect("the test app path is UTF-8");
    let (width, height) = WINDOW_SIZE;
    let launch = format!(r#"{ESCAPE} exec '{app}' '{TITLE}' {width} {height}"#);
    let project = Project::with_config(&config_for_app(
        &format!(r#"["sh", "-c", "{launch}"]"#),
        TITLE,
        r#"line = "json""#,
    ));
    let sandbox = project.sandbox("escaped-app");
    let other = project.sandbox("escaped-other");
    let mut bystander = Command::new("setsid")
        .args(["sleep", "300"])
        .env("RUTA_ROOT", other.root())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start a process with another root's marker");

    sandbox.succeed(&["up"]);
    sandbox.succeed(&["down"]);

    let bystander_status = bystander.try_wait().expect("check the bystander");
    bystander.kill().expect("stop the bystander");
    bystander.wait().expect("reap the bystander");

    assert_eq!(
        bystander_status, None,
        "a process with another root's marker keeps running"
    );
    sandbox.assert_gone();
}
