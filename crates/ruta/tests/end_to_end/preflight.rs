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

    assert_eq!(
        sandbox.records(),
        before,
        "the sandbox records are unchanged"
    );
    assert_eq!(sandbox.shot_size(), WINDOW_SIZE, "the sandbox is still up");
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
