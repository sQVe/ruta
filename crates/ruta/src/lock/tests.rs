use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;
use crate::test_support::{ChildGuard, temporary_directory};

const BUSY: &str =
    " is busy: another ruta command is using it. Run this command again after that one ends.";

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);

    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the child never signalled readiness"
        );

        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_lock(path: &Path) -> SandboxLock {
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        match take_path(path, "test", LockMode::Exclusive) {
            Ok(lock) => return lock,
            Err(LockError::Busy { .. }) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("the lock did not free after SIGKILL: {error}"),
        }
    }
}

#[test]
fn take_repairs_a_lock_file_with_no_permissions() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-repair.lock");
    fs::write(&path, "").expect("create the lock file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("remove the permissions");

    let lock = take_path(&path, "test", LockMode::Exclusive).expect("the mode is repaired");
    let mode = fs::metadata(&path)
        .expect("the lock file exists")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o600, "the existing lock file is repaired");
    drop(lock);
}

#[test]
fn refuse_unsafe_path_reports_whether_a_lock_file_is_already_there() {
    let folder = temporary_directory();
    let missing = folder.path().join("ruta-missing.lock");
    let existing = folder.path().join("ruta-existing.lock");
    fs::write(&existing, "").expect("create the lock file");

    let validated = refuse_unsafe_path(&existing, current_uid()).expect("the file is safe");
    let absent = refuse_unsafe_path(&missing, current_uid()).expect("the missing path is safe");

    assert!(validated, "an existing regular file is validated");
    assert!(!absent, "a missing path holds no lock file");
}

#[test]
fn the_open_fallback_does_not_change_an_existing_lock_mode() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-fallback.lock");
    fs::write(&path, "").expect("create the lock file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("widen the mode");

    let file = open_lock_inode(&path).expect("the existing file opens");
    let mode = fs::metadata(&path)
        .expect("the lock file exists")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(mode, 0o644, "the create fallback leaves the mode alone");
    drop(file);
}

#[test]
fn a_failed_permission_change_names_securing() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-missing.lock");

    let error = set_lock_mode(&path).expect_err("the missing file cannot be secured");

    assert!(
        matches!(&error, LockError::Secure { path: refused, .. } if refused == &path),
        "unexpected error: {error}"
    );
    assert_eq!(
        error.to_string(),
        format!("Could not secure {}", path.display())
    );
}

#[test]
fn a_restrictive_umask_does_not_keep_the_lock_from_reopening() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-umask.lock");
    let output =
        std::process::Command::new(std::env::current_exe().expect("the test binary has a path"))
            .args([
                "--exact",
                "lock::tests::a_child_sets_a_restrictive_umask_and_reopens_the_lock",
                "--nocapture",
            ])
            .env("RUTA_TEST_LOCK_UMASK_PATH", &path)
            .output()
            .expect("run the child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let ran_once = stdout.contains("test result: ok. 1 passed");

    assert!(
        output.status.success() && ran_once,
        "the child umask test failed ({}):\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_child_sets_a_restrictive_umask_and_reopens_the_lock() {
    let Some(path) = std::env::var_os("RUTA_TEST_LOCK_UMASK_PATH") else {
        return;
    };
    let previous = rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o777));

    let first =
        take_path(Path::new(&path), "test", LockMode::Exclusive).expect("the lock is created");
    let created = fs::metadata(&path).expect("the lock file exists");
    let mode = created.permissions().mode() & 0o777;
    let inode = created.ino();
    drop(first);

    let second =
        take_path(Path::new(&path), "test", LockMode::Exclusive).expect("the lock reopens");
    let reopened = fs::metadata(&path).expect("the lock file still exists");
    rustix::process::umask(previous);

    assert_eq!(mode, 0o600, "the create sets the exact mode");
    assert_eq!(reopened.ino(), inode, "the second open keeps the inode");
    drop(second);
}

#[test]
fn shared_locks_are_held_together() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-test.lock");
    let first = take_path(&path, "test", LockMode::Shared).expect("the first shared lock");
    let second = take_path(&path, "test", LockMode::Shared).expect("the second shared lock");

    let error =
        take_path(&path, "test", LockMode::Exclusive).expect_err("the exclusive lock is refused");

    assert!(
        matches!(&error, LockError::Busy { .. }),
        "unexpected error: {error}"
    );

    drop(first);
    drop(second);
}

#[test]
fn an_exclusive_lock_refuses_a_shared_and_an_exclusive_lock() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-test.lock");
    let lock = take_path(&path, "test", LockMode::Exclusive).expect("the exclusive lock");

    for mode in [LockMode::Shared, LockMode::Exclusive] {
        let error = take_path(&path, "test", mode).expect_err("the second lock is refused");

        assert!(
            matches!(&error, LockError::Busy { .. }),
            "{mode:?}: unexpected error: {error}"
        );
    }

    drop(lock);
}

#[test]
fn a_busy_error_names_the_root_it_tried() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-test.lock");
    let _lock = take_path(&path, "demo-capture", LockMode::Exclusive).expect("the exclusive lock");

    let error = take_path(&path, "demo-capture", LockMode::Exclusive)
        .expect_err("the second lock is refused");

    assert_eq!(error.to_string(), format!("ruta demo-capture{BUSY}"));
}

#[test]
fn a_symlinked_lock_path_is_refused() {
    let folder = temporary_directory();
    let target = folder.path().join("target");
    fs::write(&target, "keep").expect("write the target");
    let path = folder.path().join("ruta-test.lock");
    std::os::unix::fs::symlink(&target, &path).expect("link the lock path");

    let error = take_path(&path, "test", LockMode::Exclusive).expect_err("the link is refused");

    assert!(
        matches!(
            &error,
            LockError::Unsafe {
                path: refused,
                reason: RootReason::Symlink,
            } if refused == &path
        ),
        "unexpected error: {error}"
    );
    assert_eq!(
        error.to_string(),
        format!("refusing {}: it is a symlink", path.display())
    );
    assert_eq!(fs::read_to_string(&target).expect("the target"), "keep");
    assert!(
        fs::symlink_metadata(&path)
            .expect("the link")
            .file_type()
            .is_symlink()
    );
}

#[test]
fn a_fifo_lock_path_is_refused() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-test.lock");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &path,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .expect("make the fifo");

    let error = take_path(&path, "test", LockMode::Exclusive).expect_err("the fifo is refused");

    assert!(
        matches!(
            &error,
            LockError::Unsafe {
                path: refused,
                reason: RootReason::NotRegularFile,
            } if refused == &path
        ),
        "unexpected error: {error}"
    );
    assert_eq!(
        error.to_string(),
        format!("refusing {}: it is not a regular file", path.display())
    );
    assert!(path.exists(), "the fifo is left for the user to remove");
}

#[test]
fn a_folder_lock_path_is_refused() {
    let folder = temporary_directory();
    let path = folder.path().join("ruta-test.lock");
    fs::create_dir(&path).expect("create the folder");

    let error = take_path(&path, "test", LockMode::Exclusive).expect_err("the folder is refused");

    assert!(
        matches!(
            &error,
            LockError::Unsafe {
                path: refused,
                reason: RootReason::NotRegularFile,
            } if refused == &path
        ),
        "unexpected error: {error}"
    );
    assert!(path.is_dir(), "the folder is left for the user to remove");
}

#[test]
fn a_lock_path_owned_by_another_user_is_refused() {
    let metadata = LockMetadata {
        is_symlink: false,
        is_file: true,
        owner: 1001,
    };

    assert_eq!(
        lock_metadata_problem(&metadata, 1000),
        Some(RootReason::WrongOwner)
    );

    let error = LockError::Unsafe {
        path: PathBuf::from("/var/tmp/ruta-x.lock"),
        reason: RootReason::WrongOwner,
    };

    assert_eq!(
        error.to_string(),
        "refusing /var/tmp/ruta-x.lock: it is not owned by the current user"
    );
}

#[test]
fn an_open_file_that_is_not_regular_is_refused() {
    let folder = temporary_directory();
    let path = folder.path().join("folder");
    fs::create_dir(&path).expect("create a folder");
    let file = fs::File::open(&path).expect("open the folder");

    let error = refuse_unsafe_file(&path, &file, current_uid()).expect_err("the file is refused");

    assert!(
        matches!(
            &error,
            LockError::Unsafe {
                path: refused,
                reason: RootReason::NotRegularFile,
            } if refused == &path
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn a_loop_open_error_is_a_symlink_refusal() {
    let path = PathBuf::from("/var/tmp/ruta-x.lock");
    let source = std::io::Error::from_raw_os_error(rustix::io::Errno::LOOP.raw_os_error());

    let error = open_error(&path, source);

    assert!(
        matches!(
            &error,
            LockError::Unsafe {
                path: refused,
                reason: RootReason::Symlink,
            } if refused == &path
        ),
        "unexpected error: {error}"
    );
    assert_eq!(
        error.to_string(),
        format!("refusing {}: it is a symlink", path.display())
    );
}

#[test]
fn a_killed_holder_frees_the_lock() {
    crate::test_support::in_test_namespace!();

    let folder = temporary_directory();
    let path = folder.path().join("ruta-killed.lock");
    let ready = folder.path().join("ready");
    let mut child =
        std::process::Command::new(std::env::current_exe().expect("the test binary has a path"))
            .args([
                "--exact",
                "lock::tests::a_child_holds_the_lock_for_its_killer",
                "--nocapture",
            ])
            .env("RUTA_TEST_LOCK_HOLD_PATH", &path)
            .env("RUTA_TEST_LOCK_HOLD_READY", &ready)
            .spawn()
            .expect("spawn the child");

    wait_for_file(&ready);
    let error =
        take_path(&path, "test", LockMode::Exclusive).expect_err("the child holds the lock");
    assert!(
        matches!(&error, LockError::Busy { .. }),
        "unexpected error: {error}"
    );

    child.kill().expect("kill the child with SIGKILL");
    child.wait().expect("reap the child");

    let lock = wait_for_lock(&path);
    drop(lock);
}

#[test]
fn a_spawned_child_does_not_inherit_the_lock() {
    crate::test_support::in_test_namespace!();

    let folder = temporary_directory();
    let path = folder.path().join("ruta-inherit.lock");
    let lock = take_path(&path, "test", LockMode::Exclusive).expect("the lock is free");
    let mut command = std::process::Command::new("sleep");
    command.arg("30");
    let child = ChildGuard::spawn(&mut command);
    child.wait_until_ready("sleep", Duration::from_secs(2));

    drop(lock);

    let second =
        take_path(&path, "test", LockMode::Exclusive).expect("the child did not inherit the lock");
    drop(second);
}

#[test]
fn a_child_holds_the_lock_for_its_killer() {
    let Some(path) = std::env::var_os("RUTA_TEST_LOCK_HOLD_PATH") else {
        return;
    };
    let ready = std::env::var_os("RUTA_TEST_LOCK_HOLD_READY").expect("the ready path");
    let _lock =
        take_path(Path::new(&path), "test", LockMode::Exclusive).expect("the child takes the lock");

    fs::write(ready, "").expect("the child signals readiness");
    std::thread::sleep(Duration::from_secs(30));
}
