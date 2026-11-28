use std::fs;
use std::time::Duration;

use crate::support::*;

const SHIFT: u16 = 0x1;
const BUTTON_1: u16 = 0x100;
const SHIFT_L: &str = "0xffe1";
// Matches the settle the production focus code waits, because XWM reverts a client-set focus one
// round trip later.
const FOCUS_SETTLE: Duration = Duration::from_millis(150);
const FOCUS_COMMANDS: &[&[&str]] = &[
    &["move", "10", "10", "20", "20"],
    &["click", "100", "120"],
    &["drag", "50", "60", "200", "210"],
    &["type", "hi"],
    &["key", "Return"],
    &["scroll", "300", "200", "3"],
    &["hold", "Shift_L"],
    &["release", "Shift_L"],
];

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn input_commands_reach_the_window_at_window_coordinates() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("input");
    sandbox.succeed(&["up"]);
    sandbox.wait_for_log("ruta-test-app: running");
    let window = recorded_window(&sandbox);
    assert_eq!(
        focused_window(&sandbox),
        window,
        "up leaves the input focus on the app window"
    );

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

    assert_focus_stays_on_the_app_window(&sandbox, &window);
    assert_a_focus_probe_still_reaches_the_app(&sandbox, &window);
    assert_one_command_restores_a_focus_that_started_elsewhere(&sandbox, &window);

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}

fn assert_focus_stays_on_the_app_window(sandbox: &Sandbox, window: &str) {
    assert_no_window_decorations(sandbox, window);
    assert_eq!(
        focused_window(sandbox),
        window,
        "the input focus stays on the app window"
    );

    for arguments in FOCUS_COMMANDS {
        sandbox.succeed(arguments);

        assert_eq!(
            focused_window(sandbox),
            window,
            "{arguments:?} leaves the input focus on the app window"
        );
    }
}

fn assert_a_focus_probe_still_reaches_the_app(sandbox: &Sandbox, window: &str) {
    sandbox_xdotool(sandbox, &["windowfocus", window]);
    std::thread::sleep(FOCUS_SETTLE);
    assert_eq!(
        focused_window(sandbox),
        window,
        "the probe leaves the raw input focus on the app window"
    );

    let typed = events_after_xdotool(sandbox, &["type", "--", "hi"], "key-release", 2);
    assert_eq!(
        keysyms(&typed, "key-press"),
        ["0x68", "0x69"],
        "raw type reaches the app after the focus probe: {typed:?}"
    );

    let key = events_after_xdotool(sandbox, &["key", "Return"], "key-release", 1);
    assert_eq!(
        keysyms(&key, "key-press"),
        ["0xff0d"],
        "raw key reaches the app after the focus probe: {key:?}"
    );
}

fn assert_one_command_restores_a_focus_that_started_elsewhere(sandbox: &Sandbox, window: &str) {
    for arguments in FOCUS_COMMANDS {
        // PointerRoot is where a sandbox at rest sits on the old display stack, so the command must
        // put the focus on the app itself instead of relying on the pointer.
        sandbox_xdotool(sandbox, &["windowfocus", "1"]);
        std::thread::sleep(FOCUS_SETTLE);
        assert_eq!(
            focused_window(sandbox),
            "1",
            "{arguments:?} starts on PointerRoot, not the app window"
        );

        if arguments[0] == "type" {
            let typed = events_after(sandbox, arguments, "key-release", 2);
            assert_eq!(
                keysyms(&typed, "key-press"),
                ["0x68", "0x69"],
                "type reaches the app after the focus started elsewhere: {typed:?}"
            );
        } else {
            sandbox.succeed(arguments);
        }

        assert_eq!(
            focused_window(sandbox),
            window,
            "{arguments:?} restores the raw input focus to the app window"
        );
    }
}

fn recorded_window(sandbox: &Sandbox) -> String {
    fs::read_to_string(sandbox.root().join("window"))
        .expect("read the window record")
        .trim()
        .to_string()
}

fn focused_window(sandbox: &Sandbox) -> String {
    sandbox_xdotool(sandbox, &["getwindowfocus", "-f"])
}

// XWM gives every managed window a frame, and a decorated frame is larger than the client. With
// decorations off the frame is exactly the client size.
fn assert_no_window_decorations(sandbox: &Sandbox, window: &str) {
    let display = display_geometry(sandbox);
    let visible = sandbox_xdotool(sandbox, &["search", "--onlyvisible", "--name", ".*"]);

    for other in visible.lines() {
        if other == window {
            continue;
        }

        let (width, height) = window_size(sandbox, other);

        if (width, height) == display {
            continue;
        }

        assert!(
            width <= WINDOW_SIZE.0 && height <= WINDOW_SIZE.1,
            "window {other} is larger than the app window, so the app has a frame: {width}x{height}"
        );
    }
}

fn display_geometry(sandbox: &Sandbox) -> (u32, u32) {
    let output = sandbox_xdotool(sandbox, &["getdisplaygeometry"]);
    let (width, height) = output.split_once(' ').expect("a width and height");

    (
        width.parse().expect("a width"),
        height.parse().expect("a height"),
    )
}

fn window_size(sandbox: &Sandbox, window: &str) -> (u32, u32) {
    let output = sandbox_xdotool(sandbox, &["getwindowgeometry", "--shell", window]);
    let value = |key: &str| -> u32 {
        output
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or_else(|| panic!("{output:?} has no {key}"))
    };

    (value("WIDTH"), value("HEIGHT"))
}

#[test]
fn input_commands_without_a_sandbox_fail_before_any_input() {
    in_test_namespace!();

    let project = Project::new(&Config {
        command: Some(r#"["true"]"#),
        ..Config::default()
    });
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
        let output = Sandbox::fail_command(sandbox.command(arguments).env("PATH", tools.path()));
        let stderr = String::from_utf8_lossy(&output.stderr);

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

fn events_after(sandbox: &Sandbox, arguments: &[&str], kind: &str, count: usize) -> Vec<Event> {
    events_after_run(sandbox, kind, count, || {
        sandbox.succeed(arguments);
    })
}

fn events_after_xdotool(
    sandbox: &Sandbox,
    arguments: &[&str],
    kind: &str,
    count: usize,
) -> Vec<Event> {
    events_after_run(sandbox, kind, count, || {
        sandbox_xdotool(sandbox, arguments);
    })
}

fn events_after_run(sandbox: &Sandbox, kind: &str, count: usize, run: impl FnOnce()) -> Vec<Event> {
    let before = sandbox.succeed(&["log"]).len();

    run();

    let mut events = Vec::new();
    poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
        let log = sandbox.succeed(&["log"]);
        events = log[before..].lines().filter_map(Event::parse).collect();
        let seen = events.iter().filter(|event| event.kind == kind).count();

        seen >= count
    });

    events
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
