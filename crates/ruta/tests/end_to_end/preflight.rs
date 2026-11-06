use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::support::*;

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_bad_config_or_bad_arguments_leave_a_running_sandbox_unchanged() {
    in_test_namespace!();

    let project = Project::new(TITLE);
    let sandbox = project.sandbox("preflight");
    sandbox.succeed(&["up"]);
    let before = records(&sandbox.root());

    project.write_config(&format!(
        "colour = \"red\"\n{}",
        config_text(TITLE, JSON_READY)
    ));
    for arguments in [&["up"][..], &["restart"], &["down"]] {
        let output = sandbox.run(arguments);

        assert!(
            !output.status.success(),
            "{arguments:?} with a bad ruta.toml fails"
        );
    }

    project.write_config(&config_text(TITLE, JSON_READY));
    for arguments in [
        &["shot", "a.png", "b.png"][..],
        &["nonsense"],
        &["log", "x"],
    ] {
        let output = sandbox.run(arguments);

        assert!(!output.status.success(), "{arguments:?} fails");
    }

    assert_eq!(
        records(&sandbox.root()),
        before,
        "the sandbox records are unchanged"
    );
    assert_eq!(sandbox.shot_size(), WINDOW_SIZE, "the sandbox is still up");
}

// A PATH folder with the real display tools and no `cat`. `xdotool` is the real one, or a file with
// the given mode.
fn path_with_tools(xdotool_mode: Option<u32>) -> TempDir {
    let folder = tempfile::Builder::new()
        .prefix("ruta-end-to-end-path-")
        .tempdir()
        .expect("create the PATH folder");
    let mut linked = vec!["weston", "Xwayland", "import"];

    if xdotool_mode.is_none() {
        linked.push("xdotool");
    }

    for tool in linked {
        let real = find_on_path(tool).unwrap_or_else(|| panic!("{tool} is installed"));
        std::os::unix::fs::symlink(real, folder.path().join(tool)).expect("link the tool");
    }

    if let Some(mode) = xdotool_mode {
        let plain = folder.path().join("xdotool");
        fs::write(&plain, "#!/bin/sh\nexit 0\n").expect("write the plain xdotool");
        fs::set_permissions(&plain, std::os::unix::fs::PermissionsExt::from_mode(mode))
            .expect("set the xdotool mode");
    }

    folder
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

    let project = Project::new(TITLE);
    let sandbox = project.sandbox("plain-tool");
    sandbox.succeed(&["up"]);
    let before = records(&sandbox.root());

    // 0o641 lets only others run a file that the current user owns. Root may run any file with an
    // execute bit, so that case applies only to other users.
    let mut cases = vec![(Some(0o644), "xdotool"), (None, "cat")];

    if !rustix::process::geteuid().is_root() {
        cases.push((Some(0o641), "xdotool"));
    }

    for (xdotool_mode, refused) in cases {
        let path = path_with_tools(xdotool_mode);

        for arguments in [&["up"][..], &["restart"]] {
            let output = sandbox
                .command(arguments)
                .env("PATH", path.path())
                .output()
                .expect("run the ruta binary");
            let stderr = String::from_utf8_lossy(&output.stderr);
            let case = format!("{xdotool_mode:?} {arguments:?}");

            assert!(!output.status.success(), "{case} fails");
            assert!(
                stderr.contains(&format!("The tool \"{refused}\" is missing.")),
                "{case}: {stderr}"
            );
        }
    }

    assert_eq!(
        records(&sandbox.root()),
        before,
        "the sandbox records are unchanged"
    );
    assert_eq!(sandbox.shot_size(), WINDOW_SIZE, "the sandbox is still up");
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn up_on_a_refused_root_with_a_fifo_log_fails_at_once() {
    in_test_namespace!();

    let project = Project::new(TITLE);
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

    let mut child = sandbox
        .command(&["up"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start ruta up");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let exited = loop {
        if child.try_wait().expect("check ruta up").is_some() {
            break true;
        }

        if std::time::Instant::now() >= deadline {
            child.kill().expect("stop ruta up");
            break false;
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let output = child.wait_with_output().expect("collect ruta up");
    let stderr = String::from_utf8_lossy(&output.stderr);

    fs::remove_file(&fifo).expect("remove the fifo");
    fs::remove_dir(&root).expect("remove the root");

    assert!(exited, "up must not hang on the fifo");
    assert!(!output.status.success(), "up on a refused root fails");
    assert!(
        stderr.contains("its mode is 755, not 700"),
        "the error is the refusal: {stderr}"
    );
}
