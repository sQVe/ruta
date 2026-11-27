use std::collections::HashSet;

use super::*;

const CAPTURE_LABELS: &[&str] = &[
    "broken-stdout",
    "capture-crop",
    "capture-plain",
    "capture-ready",
    "output-capture",
    "output-order",
];

#[test]
fn one_label_generates_different_sandbox_roots() {
    let root = sandbox_root(&next_sandbox_name("shared-label"));
    let other_root = sandbox_root(&next_sandbox_name("shared-label"));

    assert_ne!(root, other_root);
}

#[test]
fn a_normal_label_builds_a_valid_name() {
    let name = next_sandbox_name("mode-child-command");

    assert_valid_ruta_name(&name);
}

#[test]
#[should_panic(
    expected = "the label \"over-long-label-that-does-not-fit\" builds a name longer than"
)]
fn an_over_long_label_is_rejected() {
    next_sandbox_name("over-long-label-that-does-not-fit");
}

#[test]
#[should_panic(
    expected = "the label \"bad_label\" builds a name with a character RUTA_NAME does not allow"
)]
fn a_label_with_a_disallowed_character_is_rejected() {
    next_sandbox_name("bad_label");
}

#[test]
#[should_panic(
    expected = "the label \"-bad\" builds a name that does not start with a letter or digit"
)]
fn a_label_starting_with_a_hyphen_is_rejected() {
    next_sandbox_name("-bad");
}

#[test]
fn capture_names_fit_the_ruta_name_limit() {
    for &label in CAPTURE_LABELS {
        let name = next_sandbox_name(label);
        let capture_name = format!("{name}-capture");

        assert_valid_ruta_name(&capture_name);
    }
}

#[test]
fn generated_names_keep_one_length_and_stay_distinct() {
    let names: Vec<String> = (0..5000)
        .map(|_| next_sandbox_name("shared-label"))
        .collect();
    let lengths: HashSet<usize> = names.iter().map(String::len).collect();
    let distinct: HashSet<&str> = names.iter().map(String::as_str).collect();
    let counters: HashSet<&str> = names
        .iter()
        .map(|name| &name[name.len() - NAME_COUNTER_DIGITS..])
        .collect();

    assert_eq!(lengths.len(), 1, "every name has one length: {lengths:?}");
    assert_eq!(distinct.len(), names.len(), "every name is distinct");
    assert_eq!(
        counters.len(),
        names.len(),
        "every name has a distinct counter"
    );
}

fn assert_valid_ruta_name(name: &str) {
    let starts_with_letter_or_digit = name
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    let holds_only_valid_characters = name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });

    assert!(
        name.len() <= NAME_LIMIT,
        "{name}: at most {NAME_LIMIT} bytes"
    );
    assert!(
        starts_with_letter_or_digit,
        "{name}: starts with a lowercase letter or a digit"
    );
    assert!(
        holds_only_valid_characters,
        "{name}: holds only lowercase letters, digits, and hyphens"
    );
}

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
