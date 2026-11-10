use super::*;

#[test]
fn default_config_keeps_the_complete_template() {
    let app = test_app().to_str().expect("UTF-8 test app path");
    let expected = format!(
        r#"name = "e2e"

[app]
command = ["{app}", "ruta test app", "640", "480"]
title = "ruta test app"
width = 640
height = 480

[ready]
line = "json"
"#
    );

    assert_eq!(Config::default().text(), expected);
}

#[test]
fn resized_config_uses_the_new_size_in_the_launch_command() {
    let config = Config {
        title: "a different title to find",
        size: (1000, 700),
        ..Config::default()
    };

    let table: toml::Table = toml::from_str(&config.text()).expect("valid TOML");
    let command: Vec<&str> = table["app"]["command"]
        .as_array()
        .expect("command array")
        .iter()
        .map(|argument| argument.as_str().expect("string argument"))
        .collect();
    let app = test_app().to_str().expect("UTF-8 test app path");

    assert_eq!(command, [app, "ruta test app", "1000", "700"]);
    assert_eq!(
        table["app"]["title"].as_str(),
        Some("a different title to find")
    );
    assert_eq!(table["app"]["width"].as_integer(), Some(1000));
    assert_eq!(table["app"]["height"].as_integer(), Some(700));
}

#[test]
fn custom_config_keeps_the_command_and_readiness_text() {
    let config = Config {
        command: Some(r#"["sleep", "2"]"#),
        title: "custom title",
        ready: COMMAND_READY,
        size: (900, 500),
    };

    assert_eq!(
        config.text(),
        r#"name = "e2e"

[app]
command = ["sleep", "2"]
title = "custom title"
width = 900
height = 500

[ready]
command = ["true"]
"#
    );
}

#[test]
fn polling_attempts_once_even_when_the_timeout_is_zero() {
    for ready in [false, true] {
        let mut attempts = 0;
        let completed = poll_until(Duration::ZERO, Duration::ZERO, || {
            attempts += 1;
            ready
        });

        assert_eq!(completed, ready, "ready={ready}");
        assert_eq!(attempts, 1, "ready={ready}");
    }
}

#[test]
fn polling_stops_when_the_attempt_succeeds() {
    let mut attempts = 0;
    let completed = poll_until(Duration::from_secs(30), Duration::ZERO, || {
        attempts += 1;
        attempts == 3
    });

    assert!(completed);
    assert_eq!(attempts, 3);
}

#[test]
fn bounded_run_keeps_the_exit_status_and_stderr() {
    in_test_namespace!();

    let output = run_within(
        Command::new("sh").args(["-c", "printf ignored; printf diagnostic >&2; exit 7"]),
        Duration::from_secs(5),
    )
    .expect("the command exits before the timeout");

    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stderr, b"diagnostic");
    assert_eq!(output.stdout, b"");
}

#[test]
fn bounded_run_kills_and_reaps_on_timeout() {
    in_test_namespace!();

    let children =
        fs::read_to_string("/proc/thread-self/children").expect("children before the run");
    let output = run_within(Command::new("sleep").arg("300"), Duration::ZERO);

    assert!(output.is_none());
    assert_eq!(
        fs::read_to_string("/proc/thread-self/children").expect("children after the run"),
        children
    );
}
