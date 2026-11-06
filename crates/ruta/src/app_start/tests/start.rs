use super::*;
use crate::config::App;
use crate::test_support::{temporary_sandbox_root, wait_until_gone};

const DISPLAY: &str = ":999";

fn command_line(program: &str, arguments: &[&str]) -> CommandLine {
    CommandLine {
        program: PathBuf::from(program),
        arguments: arguments
            .iter()
            .map(|argument| argument.to_string())
            .collect(),
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
fn start_app_stops_the_app_group_when_the_app_record_fails() {
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
