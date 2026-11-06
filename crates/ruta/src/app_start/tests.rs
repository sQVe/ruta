use std::fs;

use super::*;
use crate::test_support::ChildGuard;

fn start_script(script: &str) -> (ChildGuard, ChildStdout) {
    let mut command = Command::new("sh");
    command
        .args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    let mut started = ChildGuard::spawn_group(&mut command);
    let stdout = started.take_stdout();

    (started, stdout)
}

fn log_file(folder: &Path) -> (File, PathBuf) {
    let path = folder.join("app.log");
    let file = File::create(&path).expect("create the log");

    (file, path)
}

struct ReadReady {
    found: anyhow::Result<String>,
    logged: String,
    log_path: PathBuf,
}

fn read_ready(script: &str, timeout: Duration) -> ReadReady {
    read_ready_after(script, timeout, |_| {})
}

fn read_ready_after(script: &str, timeout: Duration, before_read: impl FnOnce(u32)) -> ReadReady {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let (started, stdout) = start_script(script);
    before_read(started.id());

    let found = read_ready_line(stdout, log, &log_path, started.id(), timeout);
    let logged = fs::read_to_string(&log_path).expect("read the log");

    ReadReady {
        found: found.map(|(line, _)| line),
        logged,
        log_path,
    }
}

fn wait_for_zombie(pid: u32) {
    let zombie = poll_until(Duration::from_secs(3), || {
        Ok(process_is_zombie(pid).then_some(()))
    })
    .expect("the zombie check cannot fail");

    assert!(zombie.is_some(), "the app became a zombie");
}

#[test]
fn read_ready_line_returns_the_first_json_object_and_logs_the_lines_before_it() {
    let ReadReady { found, logged, .. } = read_ready(
        r#"echo noise; echo '[1]'; echo '{"title":"t"}'; echo '{"title":"later"}'; exec sleep 30"#,
        Duration::from_secs(5),
    );

    assert_eq!(found.expect("a ready line"), r#"{"title":"t"}"#);
    assert!(
        logged.starts_with("noise\n[1]\n{\"title\":\"t\"}\n"),
        "logged: {logged:?}"
    );
}

#[test]
fn read_ready_line_reports_an_app_that_exited() {
    let ReadReady {
        found,
        logged,
        log_path,
    } = read_ready("echo noise", Duration::from_secs(5));

    let error = found.expect_err("the app exited");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::AppExited { log }) if log == &log_path
        ),
        "unexpected error: {error:#}"
    );
    assert_eq!(logged, "noise\n");
}

#[test]
fn read_ready_line_prefers_a_printed_ready_line_over_the_exit() {
    for attempt in 0..10 {
        let ReadReady { found, .. } = read_ready_after(
            r#"echo '{"ready":true}'"#,
            Duration::from_secs(5),
            wait_for_zombie,
        );

        assert_eq!(
            found.expect("the ready line wins over the exit check"),
            r#"{"ready":true}"#,
            "attempt {attempt}"
        );
    }
}

#[test]
fn read_ready_line_reports_an_exited_app_whose_output_stays_open_before_the_timeout() {
    let started = Instant::now();

    let ReadReady {
        found, log_path, ..
    } = read_ready_after(
        "sleep 30 & echo noise",
        Duration::from_secs(30),
        wait_for_zombie,
    );
    let elapsed = started.elapsed();

    let error = found.expect_err("the app exited");
    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::AppExited { log }) if log == &log_path
        ),
        "unexpected error: {error:#}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the exit was reported after {elapsed:?} instead of before the timeout"
    );
}

#[test]
fn read_ready_line_reports_a_timeout() {
    let timeout = Duration::from_millis(300);

    let ReadReady {
        found, log_path, ..
    } = read_ready("echo noise; exec sleep 30", timeout);

    let error = found.expect_err("no ready line");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::ReadyTimeout { timeout: recorded, log })
                if *recorded == timeout && log == &log_path
        ),
        "unexpected error: {error:#}"
    );
}

#[test]
fn copy_rest_keeps_copying_output_after_the_ready_line() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let (started, stdout) =
        start_script(r#"echo '{}'; echo buffered; sleep 0.3; echo late; exec sleep 30"#);
    let pid = started.id();
    let reader_log = log.try_clone().expect("clone the log");
    let environment = vec![(
        "PATH".to_string(),
        std::env::var("PATH").unwrap_or_default(),
    )];

    let (line, rest) = read_ready_line(stdout, reader_log, &log_path, pid, Duration::from_secs(5))
        .expect("a ready line");
    copy_rest(rest, log, &environment, pid).expect("the copy starts");

    let late = poll_until(Duration::from_secs(5), || {
        let logged = fs::read_to_string(&log_path).expect("read the log");

        Ok(logged.contains("late\n").then_some(logged))
    })
    .expect("the log check cannot fail");

    assert_eq!(line, "{}");
    assert_eq!(late.as_deref(), Some("{}\nbuffered\nlate\n"));
}

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
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");

    wait_for_ready_command(
        || counting_probe(folder.path()),
        std::process::id(),
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
fn wait_for_ready_command_reports_a_timeout_when_the_probe_never_passes() {
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");
    let timeout = Duration::from_millis(300);

    let error = wait_for_ready_command(|| Command::new("false"), std::process::id(), &log, timeout)
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

#[test]
fn wait_for_ready_command_lets_a_slow_probe_finish() {
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");

    wait_for_ready_command(
        || sleeping_probe("0.5"),
        std::process::id(),
        &log,
        Duration::from_secs(10),
    )
    .expect("a probe that passes after half a second is ready");
}

#[test]
fn wait_for_ready_command_counts_a_hung_probe_as_not_ready() {
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");
    let timeout = Duration::from_millis(300);

    let error = wait_for_ready_command(|| sleeping_probe("30"), std::process::id(), &log, timeout)
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
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");

    let error = wait_for_ready_command(
        || {
            let mut command = Command::new("sh");
            command
                .args(["-c", "sleep 30 & wait"])
                .env("RUTA_TEST_GROUP", "probe-timeout");

            command
        },
        std::process::id(),
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
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");
    let mut child = Command::new("true").spawn().expect("spawn the app");
    let pid = child.id();
    let zombie = poll_until(Duration::from_secs(3), || {
        Ok(process_is_zombie(pid).then_some(()))
    })
    .expect("the zombie check cannot fail");
    assert!(zombie.is_some(), "the app became a zombie");

    let started = Instant::now();
    let error =
        wait_for_ready_command(|| Command::new("false"), pid, &log, Duration::from_secs(30))
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
    let folder = crate::test_support::temporary_directory();
    let log = folder.path().join("app.log");

    let error = wait_for_ready_command(
        || Command::new("/nonexistent/ruta-probe"),
        std::process::id(),
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

#[test]
fn the_app_start_errors_render_their_messages() {
    let cases = [
        (
            AppStartError::AppExited {
                log: PathBuf::from("/tmp/app.log"),
            },
            "The app exited before it was ready; see /tmp/app.log",
        ),
        (
            AppStartError::ReadyTimeout {
                timeout: Duration::from_secs(2),
                log: PathBuf::from("/tmp/app.log"),
            },
            "The app was not ready within 2 seconds; see /tmp/app.log",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

mod start;
