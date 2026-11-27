use super::*;
use crate::config::App;
use crate::test_support::{strings, temporary_sandbox_root, wait_until_gone};

const DISPLAY: &str = ":999";

fn command_line(program: &str, arguments: &[&str]) -> CommandLine {
    CommandLine {
        program: PathBuf::from(program),
        arguments: strings(arguments),
    }
}

fn command_config(folder: &Path, app: CommandLine, probe: CommandLine, label: &str) -> Config {
    Config {
        name: "test".to_string(),
        folder: folder.to_path_buf(),
        app: App {
            command: app,
            title: "ruta test".to_string(),
            width: 640,
            height: 480,
        },
        ready: Ready::Command(probe),
        environment: vec![("RUTA_TEST_APP".to_string(), label.to_string())],
    }
}

#[test]
fn start_app_fails_at_once_when_the_app_exits_during_a_slow_probe() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-exits");
    let config = command_config(
        root.path(),
        command_line("sleep", &["0.5"]),
        command_line("sh", &["-c", "sleep 10; exit 1"]),
        "start-exits",
    );
    let started = Instant::now();

    let error = start_app(root.path(), &config, &[], DISPLAY).expect_err("the app exits");
    let elapsed = started.elapsed();

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::AppExited { log }) if log == &root.path().join("app.log")
        ),
        "unexpected error: {error:#}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the exit was reported after {elapsed:?}, not before the probe finished"
    );
}

#[test]
fn start_app_in_command_mode_writes_no_ready_json() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-command");
    let config = command_config(
        root.path(),
        command_line("sleep", &["30"]),
        command_line("true", &[]),
        "start-command",
    );

    start_app(root.path(), &config, &["1".to_string()], DISPLAY).expect("the probe passes at once");

    assert!(!root.path().join("ready.json").exists(), "no ready.json");
    assert_eq!(
        fs::read_to_string(root.path().join("arguments")).expect("the arguments"),
        "[\"1\"]\n"
    );

    stop_window(root.path()).expect("the app stops");
    assert_eq!(
        wait_until_gone("RUTA_TEST_APP=start-command", Duration::from_secs(2)),
        Vec::<u32>::new()
    );
}

#[test]
fn start_app_in_line_mode_keeps_logs_ready_text_and_arguments() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-line");
    let mut config = command_config(
        root.path(),
        command_line(
            "sh",
            &[
                "-c",
                "printf 'before\\n{\"ready\":true}\\nafter\\n'; printf errors >&2; sleep 30",
            ],
        ),
        command_line("true", &[]),
        "start-line",
    );
    config.ready = Ready::Line;

    start_app(root.path(), &config, &strings(&["x"]), DISPLAY)
        .expect("the JSON line makes the app ready");

    assert_eq!(
        fs::read_to_string(root.path().join("ready.json")).expect("ready record"),
        "{\"ready\":true}\n"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("arguments")).expect("arguments"),
        "[\"x\"]\n"
    );
    let copied = poll_until(Duration::from_secs(2), || {
        let log = fs::read_to_string(root.path().join("app.log")).expect("app log");
        Ok((log.contains("after\n") && log.contains("errors")).then_some(log))
    })
    .expect("log reads succeed")
    .expect("the rest of stdout and stderr reach the log");
    assert!(copied.contains("before\n"), "{copied:?}");
    assert!(copied.contains("{\"ready\":true}\n"), "{copied:?}");

    stop_window(root.path()).expect("stop the app and output copier");
    assert!(wait_until_gone("RUTA_TEST_APP=start-line", Duration::from_secs(2)).is_empty());
}

#[test]
fn start_app_fails_when_the_app_prints_a_line_over_the_bound() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-line-long");
    let script = format!(
        "head -c {} /dev/zero | tr \"\\0\" a; exec sleep 30",
        READY_LINE_LIMIT + 1
    );
    let mut config = command_config(
        root.path(),
        command_line("sh", &["-c", script.as_str()]),
        command_line("true", &[]),
        "start-line-long",
    );
    config.ready = Ready::Line;

    let error =
        start_app(root.path(), &config, &[], DISPLAY).expect_err("the line passes the bound");
    let log_path = root.path().join("app.log");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::ReadyLineTooLong { app, limit })
                if app == &PathBuf::from("sh") && *limit == READY_LINE_LIMIT
        ),
        "unexpected error: {error:#}"
    );
    let logged = fs::read(&log_path).expect("read the log");
    assert!(
        logged.len() as u64 > READY_LINE_LIMIT,
        "the log holds {} bytes",
        logged.len()
    );
    assert!(
        logged.iter().all(|byte| *byte == b'a'),
        "the log holds only the app's line"
    );
    assert!(
        !root.path().join("ready.json").exists(),
        "a refused start writes no ready record"
    );

    stop_window(root.path()).expect("the app stops");
    assert_eq!(
        wait_until_gone("RUTA_TEST_APP=start-line-long", Duration::from_secs(2)),
        Vec::<u32>::new()
    );
}

#[test]
fn start_app_stops_the_app_group_when_the_app_record_fails() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-record");
    fs::create_dir(root.path().join("pid")).expect("block the app record");
    let config = command_config(
        root.path(),
        command_line("sh", &["-c", "sleep 30 & sleep 30"]),
        command_line("true", &[]),
        "start-record",
    );

    let error = start_app(root.path(), &config, &[], DISPLAY).expect_err("the record fails");

    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::IsADirectory),
        "unexpected error: {error:#}"
    );
    assert_eq!(
        wait_until_gone("RUTA_TEST_APP=start-record", Duration::from_secs(2)),
        Vec::<u32>::new(),
        "no process of the app's group is alive"
    );
}

#[test]
fn start_app_records_an_app_that_a_launch_script_execs() {
    crate::test_support::in_test_namespace!();

    for attempt in 0..5 {
        let root = temporary_sandbox_root("start-exec");
        let launcher = root.path().join("launch");
        fs::write(&launcher, "#!/bin/sh\nexec sleep 30\n").expect("write the launcher");
        fs::set_permissions(
            &launcher,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("make the launcher runnable");
        let label = format!("start-exec-{attempt}");
        let config = command_config(
            root.path(),
            command_line(&launcher.to_string_lossy(), &[]),
            command_line("true", &[]),
            &label,
        );
        let started = Instant::now();

        start_app(root.path(), &config, &[], DISPLAY)
            .unwrap_or_else(|error| panic!("attempt {attempt}: {error:#}"));

        assert!(
            started.elapsed() < Duration::from_secs(2),
            "attempt {attempt}: the start took {:?}",
            started.elapsed()
        );
        stop_window(root.path()).expect("the app stops");
        assert_eq!(
            wait_until_gone(&format!("RUTA_TEST_APP={label}"), Duration::from_secs(2)),
            Vec::<u32>::new(),
            "attempt {attempt}"
        );
    }
}

#[test]
fn start_app_fails_at_once_when_the_app_exits_while_a_probe_descendant_holds_its_output() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-held");
    let config = command_config(
        root.path(),
        command_line("sleep", &["0.5"]),
        command_line("sh", &["-c", "setsid sleep 10 & exit 1"]),
        "start-held",
    );
    let started = Instant::now();

    let error = start_app(root.path(), &config, &[], DISPLAY).expect_err("the app exits");
    let elapsed = started.elapsed();

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::AppExited { .. })
        ),
        "unexpected error: {error:#}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "the exit was reported after {elapsed:?}"
    );
}

#[test]
fn start_app_accepts_a_ready_line_from_a_child_after_its_launcher_exits() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-orphan-line");
    let mut config = command_config(
        root.path(),
        command_line(
            "sh",
            &["-c", r#"(sleep 2; echo "{\"ready\":true}") & exit 0"#],
        ),
        command_line("true", &[]),
        "start-orphan-line",
    );
    config.ready = Ready::Line;

    start_app(root.path(), &config, &[], DISPLAY)
        .expect("the child prints the ready line after its launcher exits");

    assert_eq!(
        fs::read_to_string(root.path().join("ready.json")).expect("ready record"),
        "{\"ready\":true}\n"
    );

    stop_window(root.path()).expect("the app stops");
    assert_eq!(
        wait_until_gone("RUTA_TEST_APP=start-orphan-line", Duration::from_secs(2)),
        Vec::<u32>::new()
    );
}

#[test]
fn start_app_accepts_a_ready_command_after_its_launcher_exits() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-orphan-command");
    let counting = r#"count=$(cat count 2>/dev/null || echo 0); count=$((count + 1)); echo "$count" > count; [ "$count" -ge "$RUTA_TEST_LIMIT" ]"#;
    let mut config = command_config(
        root.path(),
        command_line("sh", &["-c", "sleep 30 & exit 0"]),
        command_line("sh", &["-c", counting]),
        "start-orphan-command",
    );
    config
        .environment
        .push(("RUTA_TEST_LIMIT".to_string(), "3".to_string()));

    start_app(root.path(), &config, &[], DISPLAY)
        .expect("the child keeps the app alive while the probe fails twice");

    assert_eq!(
        fs::read_to_string(root.path().join("count")).expect("the count"),
        "3\n"
    );

    stop_window(root.path()).expect("the app stops");
    assert_eq!(
        wait_until_gone("RUTA_TEST_APP=start-orphan-command", Duration::from_secs(2)),
        Vec::<u32>::new()
    );
}

#[test]
fn start_app_fails_when_every_app_process_exits_before_a_ready_line() {
    crate::test_support::in_test_namespace!();

    let root = temporary_sandbox_root("start-every-process-gone");
    let mut config = command_config(
        root.path(),
        command_line("sh", &["-c", "(sleep 1) & exit 0"]),
        command_line("true", &[]),
        "start-every-process-gone",
    );
    config.ready = Ready::Line;
    let started = Instant::now();

    let error = start_app(root.path(), &config, &[], DISPLAY).expect_err("every app process exits");
    let elapsed = started.elapsed();

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::AppExited { log }) if log == &root.path().join("app.log")
        ),
        "unexpected error: {error:#}"
    );
    assert!(
        elapsed >= Duration::from_millis(500) && elapsed < Duration::from_secs(5),
        "the exit was reported after {elapsed:?}"
    );

    stop_window(root.path()).expect("the app stops");
    assert_eq!(
        wait_until_gone(
            "RUTA_TEST_APP=start-every-process-gone",
            Duration::from_secs(2)
        ),
        Vec::<u32>::new()
    );
}
