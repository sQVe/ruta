use super::*;

#[test]
fn capture_refuses_symlinked_output_in_its_root_without_starting_processes() {
    crate::test_support::in_test_namespace!();

    for suffix in ["", "output"] {
        let root = crate::test_support::temporary_sandbox_root("capture-linked-output");
        let folder = crate::test_support::temporary_directory();
        let link = folder.path().join("out");
        let resolved_root = fs::canonicalize(root.path()).expect("resolve root");
        fs::remove_dir(root.path()).expect("capture creates its own root");
        std::os::unix::fs::symlink(root.path(), &link).expect("link output into capture root");
        let out = link.join(suffix);
        let expected_out = if suffix.is_empty() {
            resolved_root.clone()
        } else {
            resolved_root.join(suffix)
        };
        let last_pid =
            fs::read_to_string("/proc/sys/kernel/ns_last_pid").expect("last namespace PID");

        let error = capture(&test_config("test"), root.path(), &out, &[])
            .expect_err("resolved output would be removed during teardown");

        assert_eq!(
            error.downcast_ref::<CaptureOutputInsideRoot>(),
            Some(&CaptureOutputInsideRoot {
                out: expected_out.clone(),
                root: resolved_root.clone(),
            })
        );
        assert!(
            error
                .to_string()
                .contains(&expected_out.display().to_string())
        );
        assert!(
            error
                .to_string()
                .contains(&resolved_root.display().to_string())
        );
        assert_eq!(
            fs::read_to_string("/proc/sys/kernel/ns_last_pid").expect("last namespace PID"),
            last_pid,
            "no app or display process was started"
        );
        assert!(!root.path().exists());
        assert_eq!(
            fs::read_link(&link).expect("output link stays"),
            root.path()
        );
    }
}

#[test]
fn capture_refuses_output_in_its_root_before_creating_it() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let root = folder.path().join("capture");
    let cases = [
        (root.clone(), root.clone()),
        (root.join("output"), root.join("output")),
        (root.join("../capture/output"), root.join("output")),
    ];

    for (out, expected) in cases {
        let error = capture(&test_config("test"), &root, &out, &[])
            .expect_err("output would be removed during teardown");

        assert_eq!(
            error.downcast_ref::<CaptureOutputInsideRoot>(),
            Some(&CaptureOutputInsideRoot {
                out: expected.clone(),
                root: root.clone(),
            })
        );
        assert!(
            error.to_string().contains(&expected.display().to_string()),
            "{error:#}"
        );
        assert!(
            error.to_string().contains(&root.display().to_string()),
            "{error:#}"
        );
        assert!(!root.exists());
        assert_eq!(
            fs::read_dir(folder.path())
                .expect("untouched folder")
                .count(),
            0
        );
    }
}

#[test]
fn capture_refuses_relative_output_in_its_root() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let root = folder.path().join("capture");
    let out = root.join("output");
    let current = std::env::current_dir().expect("current folder");
    let mut relative = PathBuf::new();
    for _ in current.components().skip(1) {
        relative.push("..");
    }
    relative.push(out.strip_prefix("/").expect("absolute output"));

    let error = capture(&test_config("test"), &root, &relative, &[])
        .expect_err("relative output would be removed during teardown");

    assert_eq!(
        error.downcast_ref::<CaptureOutputInsideRoot>(),
        Some(&CaptureOutputInsideRoot {
            out: out.clone(),
            root: root.clone(),
        })
    );
    assert!(
        error.to_string().contains(&out.display().to_string()),
        "{error:#}"
    );
    assert!(
        error.to_string().contains(&root.display().to_string()),
        "{error:#}"
    );
    assert!(!root.exists());
    assert_eq!(
        fs::read_dir(folder.path())
            .expect("untouched folder")
            .count(),
        0
    );
}

#[test]
fn capture_accepts_output_outside_its_root() {
    let folder = crate::test_support::temporary_directory();
    let root = folder.path().join("capture");
    let cases = [
        folder.path().to_path_buf(),
        folder.path().join("capture-output"),
        root.join("../output"),
    ];

    for out in cases {
        refuse_capture_output_in_root(&root, &out).expect("output survives root teardown");
    }

    assert_eq!(
        fs::read_dir(folder.path())
            .expect("untouched folder")
            .count(),
        0
    );
}

#[test]
fn capture_rejects_a_26_byte_name_before_creating_its_root() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let name = "a".repeat(26);
    let derived_name = format!("{name}-capture");
    let root = folder.path().join(&derived_name);
    let out = folder.path().join("output");

    let error = capture(&test_config(&name), &root, &out, &[])
        .expect_err("the derived capture name exceeds the limit");

    assert_eq!(
        error.downcast_ref::<crate::config::ConfigError>(),
        Some(&crate::config::ConfigError::BadName {
            key: "RUTA_NAME".to_string(),
            name: derived_name.clone(),
        })
    );
    assert!(error.to_string().contains(&derived_name));
    assert!(error.to_string().contains("at most 32 bytes"));
    assert!(!root.exists());
    assert!(!out.exists());
    assert_eq!(
        fs::read_dir(folder.path())
            .expect("untouched folder")
            .count(),
        0
    );
}

#[test]
fn capture_accepts_a_24_byte_name_before_checking_its_root() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("capture-name-limit");
    let folder = crate::test_support::temporary_directory();
    let out = folder.path().join("output");
    let name = "a".repeat(24);
    fs::write(root.path().join("marker"), "keep").expect("write marker");

    let error = capture(&test_config(&name), root.path(), &out, &[])
        .expect_err("the valid name reaches the existing-root check");

    assert!(error.downcast_ref::<crate::config::ConfigError>().is_none());
    assert_eq!(
        error.to_string(),
        format!("The capture root already exists; run 'RUTA_NAME={name}-capture ruta down' first.")
    );
    assert_eq!(
        fs::read_to_string(root.path().join("marker")).expect("marker stays"),
        "keep"
    );
    assert_eq!(fs::read_dir(root.path()).expect("root stays").count(), 1);
    assert!(!out.exists());
}

#[test]
fn capture_error_wins_over_cleanup_error() {
    let root = crate::test_support::temporary_sandbox_root("capture-errors");
    let mut visited = Vec::new();
    let error = finish_capture(
        root.path(),
        Err(crate::capture::CaptureError::NotPng.into()),
        |target| {
            visited.push(target);
            Err(WillNotStop.into())
        },
    )
    .expect_err("capture error wins");

    assert_eq!(
        error.downcast_ref::<crate::capture::CaptureError>(),
        Some(&crate::capture::CaptureError::NotPng)
    );
    assert_eq!(visited, cleanup_order(root.path())[..1]);
    assert!(root.path().exists());
}

#[test]
fn successful_capture_returns_cleanup_error() {
    let root = crate::test_support::temporary_sandbox_root("capture-stop");
    let error = finish_capture(root.path(), Ok(()), |_| Err(WillNotStop.into()))
        .expect_err("cleanup failed");

    assert!(error.downcast_ref::<WillNotStop>().is_some());
    assert!(root.path().exists());
}

#[test]
fn capture_reports_root_creation_errors_without_starting() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let root = folder.path().join("missing/root");
    let out = folder.path().join("output");
    let error = capture(&test_config("test"), &root, &out, &[])
        .expect_err("missing parent prevents root creation");

    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .expect("IO error")
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(!error.to_string().contains("ruta down"));
    assert!(!root.exists());
    assert!(!out.exists());
    assert_eq!(
        fs::read_dir(folder.path())
            .expect("untouched folder")
            .count(),
        0
    );
}

#[test]
fn prepare_capture_folder_removes_only_capture_outputs() {
    let directory = crate::test_support::temporary_directory();
    let folder = directory.path().join("nested/output");

    prepare_capture_folder(&folder).expect("create missing output folder");
    assert!(folder.is_dir());
    for name in ["shot.png", "crop.png", "capture.json", "keep.txt"] {
        fs::write(folder.join(name), "old").expect("write old output");
    }

    prepare_capture_folder(&folder).expect("clear old outputs");

    for name in ["shot.png", "crop.png", "capture.json"] {
        assert!(!folder.join(name).exists(), "{name}");
    }
    assert_eq!(
        fs::read_to_string(folder.join("keep.txt")).expect("unrelated file"),
        "old"
    );
}

#[test]
fn prepare_capture_folder_refuses_to_remove_a_directory() {
    let folder = crate::test_support::temporary_directory();
    let shot = folder.path().join("shot.png");
    fs::create_dir(&shot).expect("create shot directory");
    fs::write(shot.join("keep.txt"), "keep").expect("write marker");
    fs::write(folder.path().join("crop.png"), "keep").expect("write later output");

    let error = prepare_capture_folder(folder.path()).expect_err("refuse directory");

    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .expect("IO error")
            .kind(),
        std::io::ErrorKind::IsADirectory
    );
    assert_eq!(
        fs::read_to_string(shot.join("keep.txt")).expect("marker stays"),
        "keep"
    );
    assert_eq!(
        fs::read_to_string(folder.path().join("crop.png")).expect("later output stays"),
        "keep"
    );
}
