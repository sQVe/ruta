use std::fs;
use std::io::{BufRead as _, BufReader, Read as _};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use crate::support::*;

const INPUT_COMMANDS: [&[&str]; 8] = [
    &["move", "1", "2", "3", "4"],
    &["click", "1", "2"],
    &["drag", "1", "2", "3", "4"],
    &["type", "hi"],
    &["key", "Return"],
    &["scroll", "1", "2"],
    &["hold", "Shift_L"],
    &["release", "Shift_L"],
];

fn assert_success(output: &Output, text: &str) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, text.as_bytes());
    assert_eq!(output.stderr, b"");
}

fn assert_failure(output: &Output, text: &str) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(output.stdout, b"");
    assert_eq!(output.stderr, text.as_bytes());
}

fn startup_text(name: &str, root: &Path) -> String {
    let display = fs::read_to_string(root.join("display")).expect("recorded display");

    format!(
        "ruta {name} is up on {} (log: {})\n",
        display.trim(),
        root.join("app.log").display()
    )
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn command_output_keeps_exact_text_and_silent_input() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("output");
    let up = sandbox.run(&["up"]);
    assert_success(&up, &startup_text(sandbox.name(), &sandbox.root()));
    sandbox.wait_for_log("button-release");

    let log = sandbox.run(&["log"]);
    let recorded = fs::read_to_string(sandbox.root().join("app.log")).expect("app log");
    assert_success(&log, &recorded);
    assert_success(
        &sandbox.run(&["shot"]),
        &format!("{}\n", sandbox.root().join("shot.png").display()),
    );
    assert_success(&sandbox.run(&["opened"]), "");
    fs::write(
        sandbox.root().join("opened"),
        "https://example.com\nno-final-newline",
    )
    .expect("opened record");
    assert_success(
        &sandbox.run(&["opened"]),
        "https://example.com\nno-final-newline",
    );

    for arguments in INPUT_COMMANDS {
        assert_success(&sandbox.run(arguments), "");
    }

    let restarted = sandbox.run(&["restart"]);
    assert_success(&restarted, &startup_text(sandbox.name(), &sandbox.root()));
    assert_success(
        &sandbox.run(&["down"]),
        &format!("ruta {} is gone\n", sandbox.name()),
    );
    assert_success(
        &sandbox.run(&["down"]),
        &format!("ruta {} is not up\n", sandbox.name()),
    );
}

#[test]
fn rules_print_without_a_config_and_status_refuses_an_absent_sandbox() {
    let folder = tempfile::tempdir().expect("create a folder without ruta.toml");
    let mut rules = std::process::Command::new(env!("CARGO_BIN_EXE_ruta"));
    rules.arg("rules").current_dir(folder.path());

    assert_success(
        &Sandbox::run_command(&mut rules),
        "Rules for a session that did not start the sandbox:
- Run from the project folder, with the RUTA_NAME=<name> you were given.
- `ruta status` must exit 0 before any input.
- Never run up, restart, down, or capture.
- Touch no sandbox with another name.
- Write under /var/tmp/ruta-* only where your brief says so.
- Never call xdotool, import, or ydotool, or start the app, yourself. They reach the real display.
- Follow each hold with a release of the same keys.
- Pass shot a file outside the sandbox folder.
",
    );

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("status-absent");

    assert_failure(
        &sandbox.run(&["status"]),
        &format!("ruta {} is not up\n", sandbox.name()),
    );
    assert!(!sandbox.root().exists(), "status creates no root");
}

#[test]
#[ignore = "checks installed dependencies before refusing absent sandboxes"]
fn command_errors_keep_stderr_exit_codes_and_empty_stdout() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    project.write_config("bogus = true\n");
    let sandbox = project.sandbox("output-errors");
    assert_failure(
        &sandbox.run(&["scroll", "1", "2", "0"]),
        "scroll needs a nonzero number of notches\n",
    );
    let command = sandbox.command(&["log"]);
    let config = command
        .get_current_dir()
        .expect("project folder")
        .join("ruta.toml");
    assert_failure(
        &sandbox.run(&["log"]),
        &format!("Invalid {}: unknown key \"bogus\"\n", config.display()),
    );

    project.write_config(&Config::default().text());
    let root = tempfile::Builder::new()
        .prefix(&format!("ruta-{}", sandbox.name()))
        .rand_bytes(0)
        .tempdir_in("/var/tmp")
        .expect("private sandbox root without an app log");
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
        .expect("secure the sandbox root");
    assert_eq!(root.path(), sandbox.root());
    assert!(!root.path().join("app.log").exists());
    assert_failure(
        &sandbox.run(&["log"]),
        "Could not read the app log: No such file or directory (os error 2)\n",
    );
    assert_failure(
        &sandbox.run(&["restart"]),
        "The sandbox is not up; run 'ruta up' first.\n",
    );
    let not_up = format!("ruta {} is not up; run 'ruta up' first\n", sandbox.name());
    assert_failure(&sandbox.run(&["shot"]), &not_up);
    for arguments in INPUT_COMMANDS {
        assert_failure(&sandbox.run(arguments), &not_up);
    }
}

#[test]
#[ignore = "needs Weston, Xwayland, and xdotool"]
fn capture_emits_startup_before_screenshot_and_cleanup_warnings() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("output-order");
    let folder = tempfile::tempdir().expect("capture output and fake tool");
    let entered = folder.path().join("entered");
    let release = folder.path().join("release");
    let import = folder.path().join("import");
    let script = format!(
        "#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nchmod 755 '{}'\nprintf 'forced screenshot failure\\n' >&2\nexit 7\n",
        entered.display(),
        release.display(),
        sandbox.capture_root().display()
    );
    fs::write(&import, script).expect("write the gated screenshot tool");
    fs::set_permissions(&import, fs::Permissions::from_mode(0o755))
        .expect("make import executable");
    let path = format!(
        "{}:{}",
        folder.path().display(),
        std::env::var("PATH").expect("tool path")
    );
    let (reader, writer) = UnixStream::pair().expect("combined output stream");
    reader
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound the startup read");
    let mut child = sandbox
        .command(&[
            "capture",
            "--out",
            folder.path().to_str().expect("UTF-8 path"),
        ])
        .env("PATH", path)
        .stdout(Stdio::from(OwnedFd::from(
            writer.try_clone().expect("clone output stream"),
        )))
        .stderr(Stdio::from(OwnedFd::from(writer)))
        .spawn()
        .expect("start capture");
    let mut reader = BufReader::new(reader);
    poll_until(Duration::from_secs(30), Duration::from_millis(10), || {
        entered.exists() || child.try_wait().expect("check capture").is_some()
    });
    let screenshot_started = entered.exists();
    let mut first = String::new();
    let first_read = reader.read_line(&mut first);
    let live_window = sandbox.capture_root().join("window").is_file();
    let expected_startup = if live_window {
        startup_text(sandbox.name(), &sandbox.capture_root())
    } else {
        String::new()
    };
    fs::write(&release, "continue")
        .expect("release the screenshot tool even if startup was not emitted");
    reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("bound the completion read");
    let mut rest = String::new();
    let remaining = reader.read_to_string(&mut rest);
    if remaining.is_err() {
        child.kill().expect("stop a capture that did not finish");
    }
    let status = child.wait().expect("reap capture");

    if sandbox.capture_root().exists() {
        fs::set_permissions(sandbox.capture_root(), fs::Permissions::from_mode(0o700))
            .expect("restore the capture root mode");
        let cleanup = Sandbox::run_command(
            sandbox
                .command(&["down"])
                .env("RUTA_NAME", format!("{}-capture", sandbox.name())),
        );
        assert_eq!(cleanup.status.code(), Some(0), "{cleanup:?}");
    }

    assert!(screenshot_started, "capture reached the screenshot tool");
    assert!(
        first_read.is_ok(),
        "startup was not readable while the screenshot was blocked: {first_read:?}"
    );
    assert!(
        live_window,
        "the window must still be up when startup is emitted"
    );
    assert_eq!(first, expected_startup);
    remaining.expect("capture closes its output after teardown");
    assert_eq!(status.code(), Some(1));
    assert_eq!(
        rest,
        format!(
            "Could not clean up after capture: refusing {}: its mode is 755, not 700\nimport failed: forced screenshot failure\n",
            sandbox.capture_root().display()
        )
    );
}

fn assert_capture_startup(output: &Output, sandbox: &Sandbox) {
    let text = std::str::from_utf8(&output.stdout).expect("UTF-8 output");
    let prefix = format!("ruta {} is up on :", sandbox.name());
    let rest = text
        .strip_prefix(&prefix)
        .expect("capture retains the startup line");
    let (number, _) = rest.split_once(" (log: ").expect("startup log path");
    assert!(!number.is_empty());
    assert!(number.chars().all(|character| character.is_ascii_digit()));
    assert_eq!(
        text,
        format!(
            "{prefix}{number} (log: {})\n",
            sandbox.capture_root().join("app.log").display()
        )
    );
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn capture_keeps_startup_stdout_on_success_and_later_failure() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("output-capture");
    let folder = tempfile::tempdir().expect("capture output");
    let path = folder.path().to_str().expect("UTF-8 path");

    let success = sandbox.run(&["capture", "--out", path]);
    assert_eq!(success.status.code(), Some(0), "{success:?}");
    assert_eq!(success.stderr, b"");
    assert_capture_startup(&success, &sandbox);
    assert_gone(&sandbox.capture_root());

    let failure = sandbox.run(&[
        "capture",
        "--out",
        path,
        "--",
        "--geometry",
        "10000,0,1,1,1",
    ]);
    assert_eq!(failure.status.code(), Some(1), "{failure:?}");
    assert_eq!(
        failure.stderr,
        b"The crop 1x1+10000+0 falls outside the 640x480 shot.\n"
    );
    assert_capture_startup(&failure, &sandbox);
    assert_gone(&sandbox.capture_root());
}
