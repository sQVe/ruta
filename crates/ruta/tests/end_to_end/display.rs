use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt as _;

use crate::support::*;

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn up_after_a_size_change_recreates_the_display_at_the_new_size() {
    in_test_namespace!();

    let larger = (1000, 700);
    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("resize");
    sandbox.succeed(&["up"]);
    let first_weston = sandbox.recorded_pid("weston.pid");

    project.write_config(
        &Config {
            size: larger,
            ..Config::default()
        }
        .text(),
    );
    sandbox.succeed(&["up"]);

    assert_ne!(
        sandbox.recorded_pid("weston.pid"),
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

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("non-utf8");

    Sandbox::succeed_command(
        sandbox
            .command(&["up"])
            .env("LC_RUTA_GOOD", "kept")
            .env("LC_RUTA_BAD", OsStr::from_bytes(b"\xff\xfe")),
    );

    let app = sandbox.recorded_pid("pid");
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
