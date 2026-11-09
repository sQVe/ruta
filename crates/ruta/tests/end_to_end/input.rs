use std::fs;
use std::time::{Duration, Instant};

use crate::support::*;

const SHIFT: u16 = 0x1;
const BUTTON_1: u16 = 0x100;
const SHIFT_L: &str = "0xffe1";

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn input_commands_reach_the_window_at_window_coordinates() {
    in_test_namespace!();

    let project = Project::new(TITLE);
    let sandbox = project.sandbox("input");
    sandbox.succeed(&["up"]);
    wait_for_log(&sandbox, "ruta-test-app: running");

    let click = events_after(&sandbox, &["click", "100", "120"], "button-release", 1);
    let press = only(&click, "button-press", &[("x", "100"), ("y", "120")]);
    assert_eq!(press.field("button"), "1", "{click:?}");
    assert_eq!(press.state() & SHIFT, 0, "no Shift before hold: {click:?}");
    only(&click, "button-release", &[("x", "100"), ("y", "120")]);

    let drag = events_after(
        &sandbox,
        &["drag", "50", "60", "200", "210"],
        "button-release",
        1,
    );
    only(
        &drag,
        "button-press",
        &[("x", "50"), ("y", "60"), ("button", "1")],
    );
    let held_motion = matching(&drag, "motion", &[("x", "200"), ("y", "210")]);
    assert!(
        held_motion
            .iter()
            .any(|motion| motion.state() & BUTTON_1 != 0),
        "the drag moves with button 1 down: {drag:?}"
    );
    only(
        &drag,
        "button-release",
        &[("x", "200"), ("y", "210"), ("button", "1")],
    );

    let typed = events_after(&sandbox, &["type", "hi"], "key-release", 2);
    assert_eq!(
        keysyms(&typed, "key-press"),
        ["0x68", "0x69"],
        "type presses h then i: {typed:?}"
    );

    let key = events_after(&sandbox, &["key", "Return"], "key-release", 1);
    assert_eq!(keysyms(&key, "key-press"), ["0xff0d"], "{key:?}");
    assert_eq!(keysyms(&key, "key-release"), ["0xff0d"], "{key:?}");

    let down = events_after(
        &sandbox,
        &["scroll", "300", "200", "3"],
        "button-release",
        3,
    );
    assert_eq!(
        wheel_buttons(&down),
        ["5", "5", "5"],
        "three notches down press button 5 at the point: {down:?}"
    );

    let up = events_after(
        &sandbox,
        &["scroll", "300", "200", "-2"],
        "button-release",
        2,
    );
    assert_eq!(
        wheel_buttons(&up),
        ["4", "4"],
        "two notches up press button 4 at the point: {up:?}"
    );

    let hold = events_after(&sandbox, &["hold", "Shift_L"], "key-press", 1);
    assert_eq!(keysyms(&hold, "key-press"), [SHIFT_L], "{hold:?}");
    assert_eq!(
        keysyms(&hold, "key-release"),
        Vec::<String>::new(),
        "{hold:?}"
    );

    let shifted = events_after(&sandbox, &["click", "150", "160"], "button-release", 1);
    let press = only(&shifted, "button-press", &[("x", "150"), ("y", "160")]);
    assert_ne!(
        press.state() & SHIFT,
        0,
        "the held Shift is in the click state: {shifted:?}"
    );

    let release = events_after(&sandbox, &["release", "Shift_L"], "key-release", 1);
    assert_eq!(keysyms(&release, "key-release"), [SHIFT_L], "{release:?}");

    let plain = events_after(&sandbox, &["click", "170", "180"], "button-release", 1);
    let press = only(&plain, "button-press", &[("x", "170"), ("y", "180")]);
    assert_eq!(press.state() & SHIFT, 0, "release lets Shift go: {plain:?}");

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}

#[test]
fn input_commands_without_a_sandbox_fail_before_any_input() {
    in_test_namespace!();

    let project = Project::with_config(&config_for_app(r#"["true"]"#, TITLE, JSON_READY));
    let sandbox = project.sandbox("input-not-up");
    let tools = tempfile::Builder::new()
        .prefix("ruta-end-to-end-path-")
        .tempdir()
        .expect("create the PATH folder");
    let calls = tools.path().join("calls");
    let xdotool = tools.path().join("xdotool");
    fs::write(
        &xdotool,
        format!("#!/bin/sh\necho \"$@\" >> '{}'\n", calls.display()),
    )
    .expect("write the fake xdotool");
    fs::set_permissions(
        &xdotool,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("make the fake xdotool runnable");

    for arguments in [
        &["move", "1", "2", "3", "4"][..],
        &["click", "1", "2"],
        &["drag", "1", "2", "3", "4"],
        &["type", "hi"],
        &["key", "Return"],
        &["scroll", "1", "2"],
        &["hold", "Shift_L"],
        &["release", "Shift_L"],
    ] {
        let output = sandbox
            .command(arguments)
            .env("PATH", tools.path())
            .output()
            .expect("run the ruta binary");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(!output.status.success(), "{arguments:?} fails");
        assert!(
            stderr.contains(&format!("ruta {} is not up", sandbox.name())),
            "{arguments:?}: {stderr}"
        );
    }

    assert!(!calls.exists(), "no xdotool call was made");
    assert!(!sandbox.root().exists(), "no root was made");
}

#[derive(Debug)]
struct Event {
    kind: String,
    fields: Vec<(String, String)>,
}

impl Event {
    fn parse(line: &str) -> Option<Self> {
        let mut words = line.strip_prefix("ruta-test-app: ")?.split(' ');
        let kind = words.next()?.to_string();
        let fields = words
            .map(|word| {
                let (name, value) = word.split_once('=')?;

                Some((name.to_string(), value.to_string()))
            })
            .collect::<Option<_>>()?;

        Some(Self { kind, fields })
    }

    fn field(&self, name: &str) -> &str {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
            .unwrap_or_else(|| panic!("{self:?} has a {name} field"))
    }

    fn has(&self, kind: &str, fields: &[(&str, &str)]) -> bool {
        self.kind == kind
            && fields
                .iter()
                .all(|(name, value)| self.field(name) == *value)
    }

    fn state(&self) -> u16 {
        let state = self.field("state");
        let digits = state.strip_prefix("0x").expect("the state is hexadecimal");

        u16::from_str_radix(digits, 16).expect("the state is a number")
    }
}

// Runs one ruta command and returns the input events the app logged after it, once `count` of
// them are of `kind`.
fn events_after(sandbox: &Sandbox, arguments: &[&str], kind: &str, count: usize) -> Vec<Event> {
    let before = sandbox.succeed(&["log"]).len();

    sandbox.succeed(arguments);

    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let log = sandbox.succeed(&["log"]);
        let events: Vec<Event> = log[before..].lines().filter_map(Event::parse).collect();
        let seen = events.iter().filter(|event| event.kind == kind).count();

        if seen >= count || Instant::now() >= deadline {
            return events;
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn matching<'a>(events: &'a [Event], kind: &str, fields: &[(&str, &str)]) -> Vec<&'a Event> {
    events
        .iter()
        .filter(|event| event.has(kind, fields))
        .collect()
}

fn only<'a>(events: &'a [Event], kind: &str, fields: &[(&str, &str)]) -> &'a Event {
    let found = matching(events, kind, fields);

    match found.as_slice() {
        [event] => event,
        _ => panic!("expected one {kind} at {fields:?}, found {found:?} in {events:?}"),
    }
}

fn keysyms(events: &[Event], kind: &str) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.kind == kind)
        .map(|event| event.field("keysym").to_string())
        .collect()
}

fn wheel_buttons(events: &[Event]) -> Vec<&str> {
    matching(events, "button-press", &[("x", "300"), ("y", "200")])
        .into_iter()
        .map(|event| event.field("button"))
        .collect()
}
