use super::*;

#[test]
fn a_failed_start_keeps_the_end_of_the_app_log_after_the_root_is_removed() {
    let root = crate::test_support::temporary_sandbox_root("failed-start-log");
    let lines: String = (1..=40)
        .map(|number| format!("app line {number}\n"))
        .collect();
    fs::write(root.path().join("app.log"), lines).expect("write the app log");

    let error = clean_up_failed_start(root.path(), Err(anyhow::anyhow!("the app exited")), |_| {
        Ok(())
    })
    .expect_err("the start failed");

    let failed = error
        .downcast_ref::<FailedStart>()
        .expect("the error holds the log tail");
    assert_eq!(failed.log_tail.len(), LOG_TAIL_LINES);
    assert_eq!(
        failed.log_tail.last().map(String::as_str),
        Some("app line 40")
    );
    assert_eq!(failed.error.to_string(), "the app exited");
    assert!(!root.path().exists(), "the root is removed");
}

#[test]
fn a_failed_start_without_an_app_log_keeps_its_error_unchanged() {
    let root = crate::test_support::temporary_sandbox_root("failed-start-no-log");

    let error = clean_up_failed_start(
        root.path(),
        Err(anyhow::anyhow!("the display did not start")),
        |_| Ok(()),
    )
    .expect_err("the start failed");

    assert!(error.downcast_ref::<FailedStart>().is_none());
    assert_eq!(error.to_string(), "the display did not start");
}

#[test]
fn a_start_whose_xwayland_is_not_recorded_fails_and_leaves_no_root_or_process() {
    crate::test_support::in_test_namespace!();

    let root = crate::test_support::temporary_sandbox_root("failed-start-xwayland");
    let mut weston = spawn_ready_sleep();
    record_child(&root.path().join("weston.pid"), &weston);

    let started = record_xwayland(root.path(), ":97", Duration::from_millis(200));
    let error =
        clean_up_failed_start(root.path(), started, stop_target).expect_err("the start fails");

    assert!(
        matches!(
            error.downcast_ref::<crate::identity::IdentityError>(),
            Some(crate::identity::IdentityError::XwaylandNotRecorded { .. })
        ),
        "unexpected error: {error:#}"
    );
    assert!(!root.path().exists(), "the root is removed");
    assert!(
        weston.wait_for_exit(Duration::from_secs(2)),
        "the recorded Weston is stopped"
    );
}
