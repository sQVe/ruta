use std::os::unix::fs::PermissionsExt;

use super::*;

#[test]
fn restart_plan_reports_a_missing_record() {
    let root = crate::test_support::temporary_sandbox_root("recorded-arguments");

    let error = restart_plan(root.path(), None, false).expect_err("a missing record fails");

    assert_eq!(
        error.downcast_ref::<SandboxError>(),
        Some(&SandboxError::RestartNotUp)
    );
}

#[test]
fn both_not_up_errors_keep_their_text() {
    assert_eq!(
        SandboxError::RestartNotUp.to_string(),
        "The sandbox is not up; run 'ruta up' first."
    );
    assert_eq!(
        SandboxError::NotUp {
            name: "test".to_string()
        }
        .to_string(),
        "ruta test is not up; run 'ruta up' first"
    );
}

#[test]
fn restart_plan_reuses_the_recorded_arguments_and_headed_mode() {
    let root = crate::test_support::temporary_sandbox_root("restart-recorded");
    fs::write(
        root.path().join("arguments"),
        "[\"--fast\",\"a b\"]
",
    )
    .expect("write the arguments");
    fs::write(
        root.path().join("mode"),
        "headed
",
    )
    .expect("write the mode");

    assert_eq!(
        restart_plan(root.path(), None, false).expect("the records are read"),
        (strings(&["--fast", "a b"]), true)
    );
}

#[test]
fn restart_plan_reuses_a_recorded_headless_mode() {
    let root = crate::test_support::temporary_sandbox_root("restart-headless");
    fs::write(
        root.path().join("arguments"),
        "[]
",
    )
    .expect("write the arguments");
    fs::write(
        root.path().join("mode"),
        "headless
",
    )
    .expect("write the mode");

    assert_eq!(
        restart_plan(root.path(), None, false).expect("the records are read"),
        (Vec::new(), false)
    );
}

#[test]
fn restart_plan_prefers_the_given_arguments_and_mode() {
    let root = crate::test_support::temporary_sandbox_root("restart-explicit");
    fs::write(
        root.path().join("arguments"),
        "[\"--fast\"]
",
    )
    .expect("write the arguments");
    fs::write(
        root.path().join("mode"),
        "headless
",
    )
    .expect("write the mode");

    assert_eq!(
        restart_plan(root.path(), Some(strings(&["--slow"])), true).expect("the given values win"),
        (strings(&["--slow"]), true)
    );
}

#[test]
fn restart_plan_refuses_an_unsafe_root_even_with_given_arguments() {
    let root = crate::test_support::temporary_sandbox_root("restart-unsafe");
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).expect("set the mode");

    let error = restart_plan(root.path(), Some(strings(&["--slow"])), false)
        .expect_err("the root is refused");

    assert!(
        matches!(
            error.downcast_ref::<RootError>(),
            Some(RootError::Unsafe {
                path,
                reason: RootReason::WrongMode { mode: 0o755 },
            }) if path.as_path() == root.path()
        ),
        "unexpected error: {error:#}"
    );
}
