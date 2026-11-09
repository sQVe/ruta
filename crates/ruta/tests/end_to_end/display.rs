use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt as _;

use crate::support::*;

const JSON: &str = r#"line = "json""#;

fn recorded_pid(sandbox: &Sandbox, record: &str) -> u32 {
    fs::read_to_string(sandbox.root().join(record))
        .expect("read the record")
        .split_whitespace()
        .next()
        .and_then(|pid| pid.parse().ok())
        .expect("a recorded PID")
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn up_after_a_size_change_recreates_the_display_at_the_new_size() {
    in_test_namespace!();

    let larger = (1000, 700);
    let project = Project::new(TITLE);
    let sandbox = project.sandbox("resize");
    sandbox.succeed(&["up"]);
    let first_weston = recorded_pid(&sandbox, "weston.pid");

    project.write_config(&config_text_at_size(TITLE, JSON, larger));
    sandbox.succeed(&["up"]);

    assert_ne!(
        recorded_pid(&sandbox, "weston.pid"),
        first_weston,
        "a new size starts a new display"
    );
    assert_eq!(sandbox.shot_size(), larger, "the shot has the new size");

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn up_skips_a_caller_variable_that_is_not_utf8() {
    in_test_namespace!();

    let project = Project::new(TITLE);
    let sandbox = project.sandbox("non-utf8");

    let output = sandbox
        .command(&["up"])
        .env("LC_RUTA_GOOD", "kept")
        .env("LC_RUTA_BAD", OsStr::from_bytes(b"\xff\xfe"))
        .output()
        .expect("run the ruta binary");

    assert!(
        output.status.success(),
        "up succeeds: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let app = recorded_pid(&sandbox, "pid");
    let environment = fs::read(format!("/proc/{app}/environ")).expect("read the app environment");
    let entries: Vec<&[u8]> = environment.split(|byte| *byte == 0).collect();

    assert!(
        entries.contains(&b"LC_RUTA_GOOD=kept".as_slice()),
        "a valid allowed variable reaches the app"
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.starts_with(b"LC_RUTA_BAD=")),
        "the variable that is not UTF-8 does not reach the app"
    );

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}
