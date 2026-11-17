use std::process::{Command, Stdio};
use std::time::Duration;

use crate::support::*;

// A process that leaves its process group with `setsid`, then lives on.
const ESCAPE: &str = "setsid sleep 300 >/dev/null 2>&1 < /dev/null &";

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_failed_up_stops_a_ready_check_child_that_left_its_group() {
    in_test_namespace!();

    let project = Project::new(&Config {
        command: Some(r#"["sleep", "2"]"#),
        ready: &format!(r#"command = ["sh", "-c", "{ESCAPE} exit 1"]"#),
        ..Config::default()
    });
    let sandbox = project.sandbox("escaped-probe");

    sandbox.fail(&["up"]);
    sandbox.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn down_stops_an_app_child_that_left_its_group_and_spares_other_sandboxes() {
    in_test_namespace!();

    let app = test_app().to_str().expect("the test app path is UTF-8");
    let (width, height) = WINDOW_SIZE;
    let launch = format!(r#"{ESCAPE} exec '{app}' '{TITLE}' {width} {height}"#);
    let project = Project::new(&Config {
        command: Some(&format!(r#"["sh", "-c", "{launch}"]"#)),
        ..Config::default()
    });
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

fn replacing_stops_a_detached_process(label: &str, replacing: &[&str]) {
    in_test_namespace!();

    let app = test_app().to_str().expect("the test app path is UTF-8");
    let (width, height) = WINDOW_SIZE;
    let launch = format!(
        "setsid sleep 300 >/dev/null 2>&1 < /dev/null & echo $! > escaped.pid; exec '{app}' '{TITLE}' {width} {height}"
    );
    let project = Project::new(&Config {
        command: Some(&format!(r#"["sh", "-c", "{launch}"]"#)),
        ..Config::default()
    });
    let sandbox = project.sandbox(label);

    sandbox.succeed(&["up"]);
    let first = read_escaped_pid(&project);
    assert!(process_is_running(first), "the first app's child lives");

    sandbox.succeed(replacing);
    let second = read_escaped_pid(&project);
    assert_ne!(second, first, "the replacing app starts a new child");
    let gone = poll_until(Duration::from_secs(10), Duration::from_millis(100), || {
        !process_is_running(first)
    });
    assert!(gone, "the replacing app stops the first app's child");

    assert_eq!(sandbox.shot_size(), WINDOW_SIZE);

    sandbox.succeed(&["down"]);
}

fn read_escaped_pid(project: &Project) -> u32 {
    std::fs::read_to_string(project.folder().join("escaped.pid"))
        .expect("read the escaped PID")
        .trim()
        .parse()
        .expect("a PID")
}

// The test namespace's PID 1 is the test binary, which does not reap an orphan, so a stopped
// process stays as a zombie.
fn process_is_running(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let state = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next());

    !matches!(state, Some("Z") | Some("X"))
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn restart_stops_a_detached_process_the_old_app_started() {
    replacing_stops_a_detached_process("restart-escape", &["restart"]);
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_second_up_stops_a_detached_process_the_old_app_started() {
    replacing_stops_a_detached_process("up-escape", &["up"]);
}
