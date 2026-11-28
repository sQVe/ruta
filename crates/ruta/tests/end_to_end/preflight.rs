use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::TempDir;

use crate::support::*;

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_bad_config_or_bad_arguments_leave_a_running_sandbox_unchanged() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("preflight");
    sandbox.succeed(&["up"]);
    let before = sandbox.records();

    project.write_config(&format!("colour = \"red\"\n{}", Config::default().text()));
    for arguments in [&["up"][..], &["restart"], &["down"]] {
        sandbox.fail(arguments);
    }

    project.write_config(&Config::default().text());
    for arguments in [
        &["shot", "a.png", "b.png"][..],
        &["nonsense"],
        &["log", "x"],
    ] {
        sandbox.fail(arguments);
    }

    assert_a_refused_weston_version_leaves_the_sandbox_up(&sandbox, &before);

    assert_eq!(
        sandbox.records(),
        before,
        "the sandbox records are unchanged"
    );
    assert_eq!(sandbox.shot_size(), WINDOW_SIZE, "the sandbox is still up");
}

fn assert_a_refused_weston_version_leaves_the_sandbox_up(
    sandbox: &Sandbox,
    before: &[(&'static str, Option<String>)],
) {
    let log = sandbox.wait_for_log("ruta-test-app: running");
    let tools = tempfile::Builder::new()
        .prefix("ruta-end-to-end-path-")
        .tempdir()
        .expect("create the PATH folder");
    let calls = fake_weston_14(tools.path());
    let path = old_weston_path(tools.path());

    for arguments in [&["up"][..], &["restart"]] {
        let mut command = sandbox.command(arguments);
        command.env("PATH", &path);
        let output = Sandbox::run_command(&mut command);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
        assert!(
            stderr
                .contains("The installed weston is version 14.0.2. ruta needs weston 15 or newer"),
            "{arguments:?}: {stderr}"
        );
    }

    assert_eq!(
        sandbox.records(),
        before,
        "the running sandbox records are unchanged"
    );
    assert!(
        sandbox.succeed(&["status"]).contains("is up and serves"),
        "the sandbox is still up"
    );
    assert_eq!(sandbox.succeed(&["log"]), log, "the app log is unchanged");
    assert_eq!(
        sandbox.shot_size(),
        WINDOW_SIZE,
        "the shot still has the app size"
    );
    assert_eq!(
        fs::read_to_string(&calls).expect("read the weston calls"),
        "--version\n--version\n",
        "each start request probes weston once"
    );
}

fn path_with(linked: &[&str]) -> TempDir {
    let folder = tempfile::Builder::new()
        .prefix("ruta-end-to-end-path-")
        .tempdir()
        .expect("create the PATH folder");

    for tool in linked {
        let real = find_on_path(tool).unwrap_or_else(|| panic!("{tool} is installed"));
        std::os::unix::fs::symlink(real, folder.path().join(tool)).expect("link the tool");
    }

    folder
}

fn write_plain_file(folder: &Path, name: &str, mode: u32) {
    let plain = folder.join(name);
    fs::write(&plain, "#!/bin/sh\nexit 0\n").expect("write the plain tool");

    fs::set_permissions(&plain, std::os::unix::fs::PermissionsExt::from_mode(mode))
        .expect("set the tool mode");
}

fn find_on_path(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;

    std::env::split_paths(&path)
        .map(|folder| folder.join(tool))
        .find(|candidate| candidate.is_file())
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_missing_or_unrunnable_tool_is_refused_and_leaves_a_running_sandbox_up() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("plain-tool");
    sandbox.succeed(&["up"]);
    let before = sandbox.records();

    // 0o641 lets only others run a file that the current user owns. Root may run any file with an
    // execute bit, so that case applies only to other users.
    let cases = [
        ("non-executable tool", "xdotool", Some(0o644)),
        ("missing cat", "cat", None),
        ("only others can execute", "xdotool", Some(0o641)),
    ];

    for (name, refused, mode) in cases {
        if mode == Some(0o641) && rustix::process::geteuid().is_root() {
            continue;
        }

        let mut linked = vec!["weston", "Xwayland", "import"];
        if mode.is_none() {
            linked.push("xdotool");
        }

        let path = path_with(&linked);
        if let Some(mode) = mode {
            write_plain_file(path.path(), "xdotool", mode);
        }

        for arguments in [&["up"][..], &["restart"]] {
            let output = Sandbox::fail_command(sandbox.command(arguments).env("PATH", path.path()));
            let stderr = String::from_utf8_lossy(&output.stderr);
            let case = format!("{name} {arguments:?}");
            assert!(
                stderr.contains(&format!("The tool \"{refused}\" is missing.")),
                "{case}: {stderr}"
            );
        }
    }

    assert_eq!(
        sandbox.records(),
        before,
        "the sandbox records are unchanged"
    );
    assert_eq!(sandbox.shot_size(), WINDOW_SIZE, "the sandbox is still up");
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn up_on_a_refused_root_with_a_fifo_log_fails_at_once() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("fifo-log");
    let root = sandbox.root();
    fs::create_dir(&root).expect("create the root");
    fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("open the root");
    let fifo = root.join("app.log");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(made.success(), "mkfifo app.log");

    let output = sandbox.run_within(&["up"], Duration::from_secs(10));

    fs::remove_file(&fifo).expect("remove the fifo");
    fs::remove_dir(&root).expect("remove the root");

    let output = output.expect("up must not hang on the fifo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "up on a refused root fails");
    assert!(
        stderr.contains("its mode is 755, not 700"),
        "the error is the refusal: {stderr}"
    );
}

fn fake_weston_14(folder: &Path) -> PathBuf {
    let calls = folder.join("weston.calls");
    let weston = folder.join("weston");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = \"--version\" ]; then\n  echo 'weston 14.0.2'\n  exit 0\nfi\nexit 0\n",
        calls.display()
    );
    fs::write(&weston, script).expect("write the fake weston");
    fs::set_permissions(&weston, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("make the fake weston runnable");

    calls
}

fn old_weston_path(folder: &Path) -> String {
    format!(
        "{}:{}",
        folder.display(),
        std::env::var("PATH").expect("PATH is set")
    )
}

#[test]
fn capture_refuses_an_old_weston_before_it_clears_the_output() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("old-weston");
    let tools = tempfile::Builder::new()
        .prefix("ruta-end-to-end-path-")
        .tempdir()
        .expect("create the PATH folder");
    let calls = fake_weston_14(tools.path());
    let out = tempfile::tempdir().expect("create the output folder");
    for name in ["shot.png", "crop.png", "capture.json"] {
        fs::write(out.path().join(name), format!("old {name}")).expect("write the old output");
    }
    let out_argument = out.path().to_str().expect("the output path is UTF-8");

    let output = Sandbox::fail_command(
        sandbox
            .command(&["capture", "--out", out_argument])
            .env("PATH", old_weston_path(tools.path())),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("The installed weston is version 14.0.2. ruta needs weston 15 or newer"),
        "{stderr}"
    );
    for name in ["shot.png", "crop.png", "capture.json"] {
        assert_eq!(
            fs::read_to_string(out.path().join(name)).expect("the old output remains"),
            format!("old {name}"),
            "{name} is untouched"
        );
    }
    assert_eq!(
        fs::read_to_string(&calls).expect("read the weston calls"),
        "--version\n"
    );
    assert!(!sandbox.capture_root().exists(), "no capture root is made");
}

#[test]
fn capture_refuses_a_directory_output_before_it_starts_the_app() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("dir-output");
    let tools = tempfile::Builder::new()
        .prefix("ruta-end-to-end-path-")
        .tempdir()
        .expect("create the PATH folder");
    let calls = fake_weston_14(tools.path());
    let out = tempfile::tempdir().expect("create the output folder");
    let blocked = out.path().join("shot.png");
    fs::create_dir(&blocked).expect("block the output");
    let out_argument = out.path().to_str().expect("the output path is UTF-8");

    let output = Sandbox::fail_command(
        sandbox
            .command(&["capture", "--out", out_argument])
            .env("PATH", old_weston_path(tools.path())),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains(&blocked.display().to_string()),
        "the error names the path: {stderr}"
    );
    assert!(
        stderr.contains("is not a regular file"),
        "the error is the typed refusal: {stderr}"
    );
    assert!(blocked.is_dir(), "the directory stays");
    assert!(!calls.exists(), "the fake weston saw no call");
    assert!(!sandbox.capture_root().exists(), "no capture root is left");
}

#[test]
fn up_refuses_an_old_weston_before_it_makes_a_root() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("old-weston-up");
    let tools = tempfile::Builder::new()
        .prefix("ruta-end-to-end-path-")
        .tempdir()
        .expect("create the PATH folder");
    let calls = fake_weston_14(tools.path());

    let output = Sandbox::fail_command(
        sandbox
            .command(&["up"])
            .env("PATH", old_weston_path(tools.path())),
    );
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr.contains("The installed weston is version 14.0.2. ruta needs weston 15 or newer"),
        "{stderr}"
    );
    assert!(!sandbox.root().exists(), "no sandbox root is made");
    assert_eq!(
        fs::read_to_string(&calls).expect("read the weston calls"),
        "--version\n"
    );
}
