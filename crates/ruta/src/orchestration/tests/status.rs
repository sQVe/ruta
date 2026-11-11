use std::os::unix::ffi::OsStrExt as _;

use super::*;
use crate::orchestration::status::{StatusError, status};

fn served_sandbox(label: &str, folder: &Path) -> tempfile::TempDir {
    let root = live_sandbox(label);
    fs::write(root.path().join("folder"), folder.as_os_str().as_bytes())
        .expect("write the folder record");

    root
}

fn root_contents(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut contents: Vec<_> = fs::read_dir(root)
        .expect("list the root")
        .map(|entry| {
            let path = entry.expect("a root entry").path();
            let bytes = fs::read(&path).expect("read the record");

            (path, bytes)
        })
        .collect();
    contents.sort();

    contents
}

fn status_error(root: &Path) -> StatusError {
    let before = root_contents(root);

    let error = status(&test_config("test"), root).expect_err("the status is refused");

    assert_eq!(root_contents(root), before, "the root must not change");

    error
        .downcast::<StatusError>()
        .expect("the error is a status error")
}

#[test]
fn status_names_the_folder_a_live_sandbox_serves() {
    let root = served_sandbox("status-up", Path::new("/nonexistent"));
    let before = root_contents(root.path());

    let output = status(&test_config("test"), root.path()).expect("the sandbox serves the folder");

    assert_eq!(output, "ruta test is up and serves /nonexistent\n");
    assert_eq!(
        root_contents(root.path()),
        before,
        "the root must not change"
    );
}

#[test]
fn guide_quotes_the_line_status_prints() {
    let root = served_sandbox("status-guide", Path::new("<folder>"));
    let mut config = test_config("<name>");
    config.folder = PathBuf::from("<folder>");

    let output = status(&config, root.path()).expect("the sandbox serves the folder");

    assert!(
        crate::command::GUIDE.contains(&format!("prints\n  `{}`.", output.trim_end())),
        "{output}"
    );
}

#[test]
fn status_refuses_a_sandbox_that_is_not_up() {
    let not_up = StatusError::NotUp {
        name: "test".to_string(),
    };

    for (case, make_stale) in stale_display_cases() {
        let root = served_sandbox("status-stale", Path::new("/nonexistent"));
        make_stale(root.path());

        assert_eq!(status_error(root.path()), not_up, "{case}");
    }

    let root = served_sandbox("status-window", Path::new("/nonexistent"));
    fs::remove_file(root.path().join("window")).expect("remove the window");

    assert_eq!(status_error(root.path()), not_up, "missing window");
    assert_eq!(not_up.to_string(), "ruta test is not up");
}

#[test]
fn status_refuses_an_absent_root_and_does_not_create_it() {
    let folder = crate::test_support::temporary_directory();
    let root = folder.path().join("ruta-absent");

    let error = status(&test_config("test"), &root).expect_err("an absent sandbox is not up");

    assert_eq!(
        error.downcast_ref::<StatusError>(),
        Some(&StatusError::NotUp {
            name: "test".to_string()
        })
    );
    assert!(!root.exists());
}

#[test]
fn status_refuses_a_sandbox_another_folder_started() {
    let root = served_sandbox("status-other", Path::new("/elsewhere"));

    let error = status_error(root.path());

    assert_eq!(
        error,
        StatusError::OtherFolder {
            name: "test".to_string(),
            recorded: PathBuf::from("/elsewhere"),
            folder: PathBuf::from("/nonexistent"),
        }
    );
    assert_eq!(
        error.to_string(),
        "ruta test serves /elsewhere, not /nonexistent"
    );
}

#[test]
fn status_refuses_a_root_without_a_folder_record() {
    let root = live_sandbox("status-no-record");

    let error = status_error(root.path());

    assert_eq!(
        error,
        StatusError::NoFolderRecord {
            name: "test".to_string()
        }
    );
    assert_eq!(
        error.to_string(),
        "ruta test has no folder record; start it again with this ruta"
    );
}

#[test]
fn status_refuses_an_unsafe_root() {
    let root = served_sandbox("status-unsafe", Path::new("/nonexistent"));
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).expect("make root unsafe");

    let error = status(&test_config("test"), root.path()).expect_err("the root is refused");

    assert!(
        matches!(
            error.downcast_ref::<RootError>(),
            Some(RootError::Unsafe { .. })
        ),
        "unexpected error: {error:#}"
    );
}

#[test]
fn a_start_records_the_project_folder_before_the_app_starts() {
    crate::test_support::in_test_namespace!();

    let root = live_sandbox("start-folder");
    let mut config = test_config("test");
    config.folder = PathBuf::from("/project/of the\nsandbox");
    let size = display_size((config.app.width, config.app.height));
    // The live display matches the config, so the start reuses it and stops nothing.
    record_display_settings(root.path(), "headless", size);
    prepare_root(root.path()).expect("the root is prepared");
    let target = crate::test_support::temporary_directory();
    let link = root.path().join("bin").join("xdg-open");
    std::os::unix::fs::symlink(target.path(), &link).expect("link the opener");

    start_sandbox(&config, root.path(), &[], &Session::Headless)
        .expect_err("the linked opener stops the start");

    assert_eq!(
        status(&config, root.path()).expect("the sandbox serves the folder"),
        "ruta test is up and serves /project/of the\nsandbox\n"
    );
}
