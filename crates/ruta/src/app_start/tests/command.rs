use super::*;

fn counting_probe(folder: &Path) -> Command {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            r#"count=$(cat count 2>/dev/null || echo 0); count=$((count + 1)); echo "$count" > count; [ "$count" -ge "$RUTA_TEST_LIMIT" ]"#,
        ])
        .current_dir(folder)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("RUTA_TEST_LIMIT", "3");

    command
}

#[test]
fn wait_for_ready_command_polls_until_the_probe_exits_0() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");

    let (_app, leader) = live_app();

    wait_for_ready_command(
        || counting_probe(folder.path()),
        leader,
        folder.path(),
        &log,
        Duration::from_secs(10),
    )
    .expect("the probe passes on its third run");

    assert_eq!(
        fs::read_to_string(folder.path().join("count")).expect("the count"),
        "3\n"
    );
}

#[test]
fn a_probe_timeout_with_a_failed_report_is_not_retried() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");
    let (_app, leader) = live_app();
    let mut runs = 0;

    let error = wait_for_ready_command_with(
        || Command::new("false"),
        leader,
        folder.path(),
        &log,
        Duration::from_secs(5),
        |_, _, _| {
            runs += 1;

            Err(cleanup::with_report_failure(
                ProcessError::Timeout {
                    program: "probe".to_string(),
                    duration: Duration::from_secs(1),
                }
                .into(),
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe)),
            ))
        },
    )
    .expect_err("the report failure is returned");

    assert_eq!(runs, 1, "the probe is not retried");
    assert!(
        format!("{error:#}").contains("Could not report the cleanup"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn wait_for_ready_command_reports_a_timeout_when_the_probe_never_passes() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");
    let timeout = Duration::from_millis(300);

    let (_app, leader) = live_app();
    let error = wait_for_ready_command(
        || Command::new("false"),
        leader,
        folder.path(),
        &log,
        timeout,
    )
    .expect_err("the probe never passes");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::ReadyTimeout { timeout: recorded, log: recorded_log })
                if *recorded == timeout && recorded_log == &log
        ),
        "unexpected error: {error:#}"
    );
}

fn sleeping_probe(seconds: &str) -> Command {
    let mut command = Command::new("sleep");
    command.arg(seconds);

    command
}

fn live_app() -> (ChildGuard, ProcessIdentity) {
    let mut command = Command::new("sleep");
    command.arg("30");
    let started = ChildGuard::spawn_group(&mut command);
    let identity = identity_of(started.id());

    (started, identity)
}

#[test]
fn wait_for_ready_command_lets_a_slow_probe_finish() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");

    let (_app, leader) = live_app();

    wait_for_ready_command(
        || sleeping_probe("0.5"),
        leader,
        folder.path(),
        &log,
        Duration::from_secs(10),
    )
    .expect("a probe that passes after half a second is ready");
}

#[test]
fn wait_for_ready_command_counts_a_hung_probe_as_not_ready() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");
    let timeout = Duration::from_millis(300);

    let (_app, leader) = live_app();
    let error = wait_for_ready_command(
        || sleeping_probe("30"),
        leader,
        folder.path(),
        &log,
        timeout,
    )
    .expect_err("the probe never finishes");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::ReadyTimeout { timeout: recorded, .. }) if *recorded == timeout
        ),
        "unexpected error: {error:#}"
    );
}

#[test]
fn wait_for_ready_command_leaves_no_process_of_a_timed_out_probe() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");

    let (_app, leader) = live_app();
    let error = wait_for_ready_command(
        || {
            let mut command = Command::new("sh");
            command
                .args(["-c", "sleep 30 & wait"])
                .env("RUTA_TEST_GROUP", "probe-timeout");

            command
        },
        leader,
        folder.path(),
        &log,
        Duration::from_millis(300),
    )
    .expect_err("the probe never finishes");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::ReadyTimeout { .. })
        ),
        "unexpected error: {error:#}"
    );
    assert_eq!(
        crate::test_support::wait_until_gone(
            "RUTA_TEST_GROUP=probe-timeout",
            Duration::from_secs(2)
        ),
        Vec::<u32>::new(),
        "no process of the probe's group is alive"
    );
}

#[test]
fn wait_for_ready_command_reports_an_app_that_exited_before_the_timeout() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");
    let mut command = Command::new("sleep");
    command.arg("0.2").process_group(0);
    let mut child = command.spawn().expect("spawn the app");
    let pid = child.id();
    let leader = identity_of(pid);
    let zombie = poll_until(Duration::from_secs(3), || {
        Ok(process_is_zombie(pid).then_some(()))
    })
    .expect("the zombie check cannot fail");
    assert!(zombie.is_some(), "the app became a zombie");

    let started = Instant::now();
    let error = wait_for_ready_command(
        || Command::new("false"),
        leader,
        folder.path(),
        &log,
        Duration::from_secs(30),
    )
    .expect_err("the app exited");
    let elapsed = started.elapsed();

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::AppExited { log: recorded }) if recorded == &log
        ),
        "unexpected error: {error:#}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the exit was reported after {elapsed:?} instead of before the timeout"
    );

    crate::test_support::stop_child(&mut child);
}

#[test]
fn wait_for_ready_command_reports_a_probe_that_cannot_start() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");

    let (_app, leader) = live_app();
    let error = wait_for_ready_command(
        || Command::new("/nonexistent/ruta-probe"),
        leader,
        folder.path(),
        &log,
        Duration::from_secs(5),
    )
    .expect_err("the probe cannot start");

    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::NotFound),
        "unexpected error: {error:#}"
    );
}

#[test]
fn probe_command_runs_in_the_config_folder_with_the_given_environment() {
    let folder = crate::test_support::temporary_directory();
    let probe = CommandLine {
        program: PathBuf::from("sh"),
        arguments: vec![
            "-c".to_string(),
            r#"[ "$(pwd)" = "$RUTA_TEST_FOLDER" ]"#.to_string(),
        ],
    };
    let environment = vec![
        (
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_default(),
        ),
        (
            "RUTA_TEST_FOLDER".to_string(),
            folder.path().to_string_lossy().into_owned(),
        ),
    ];

    let status = probe_command(&probe, &environment, folder.path())
        .status()
        .expect("run the probe");

    assert!(status.success());
}
