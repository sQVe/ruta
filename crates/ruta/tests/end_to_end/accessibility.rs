mod bus;
mod tree;

use std::time::Duration;

use atspi::Role;

use self::bus::A11yBus;
use self::tree::{Bounds, wait_for_control};
use crate::support::*;

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, ImageMagick, dbus, and at-spi2-core"]
fn accessibility_tree_shows_the_controls() {
    in_test_namespace!();

    let bus = A11yBus::start().expect("start the accessibility bus");
    let command = app_command(&bus);
    let project = Project::new(&Config {
        command: Some(&command),
        ..Config::default()
    });
    let sandbox = project.sandbox("a11y-tree");
    sandbox.succeed(&["up"]);
    bus.enable().expect("enable accessibility");

    for (name, role, bounds) in [
        ("alpha", Role::Button, (20, 20, 120, 40)),
        ("beta", Role::Button, (20, 80, 120, 40)),
        ("gamma", Role::Button, (20, 140, 120, 40)),
    ] {
        let node = wait_for_control(bus.accessibility_address(), app_name(), name);

        assert_eq!(node.role, role, "{name}");
        assert_eq!(bounds_of(&node.bounds), bounds, "{name}");
    }

    let text = wait_for_control(bus.accessibility_address(), app_name(), "text");

    assert_eq!(text.role, Role::Entry);
    assert_eq!(bounds_of(&text.bounds), (20, 200, 300, 40));

    sandbox.succeed(&["down"]);
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, ImageMagick, dbus, and at-spi2-core"]
fn clicking_a_reported_centre_presses_the_button() {
    in_test_namespace!();

    let bus = A11yBus::start().expect("start the accessibility bus");
    let command = app_command(&bus);
    let project = Project::new(&Config {
        command: Some(&command),
        ..Config::default()
    });
    let sandbox = project.sandbox("a11y-click");
    sandbox.succeed(&["up"]);
    bus.enable().expect("enable accessibility");

    let alpha = wait_for_control(bus.accessibility_address(), app_name(), "alpha");
    let x = alpha.bounds.x + alpha.bounds.width / 2;
    let y = alpha.bounds.y + alpha.bounds.height / 2;

    sandbox.succeed(&["click", &x.to_string(), &y.to_string()]);

    let log = sandbox.wait_for_log("ruta-test-app: press name=alpha");
    assert!(
        log.contains("ruta-test-app: press name=alpha"),
        "the click reaches alpha: {log}"
    );

    sandbox.succeed(&["down"]);
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, ImageMagick, dbus, and at-spi2-core"]
fn typing_changes_the_text_field_value() {
    in_test_namespace!();

    let bus = A11yBus::start().expect("start the accessibility bus");
    let command = app_command(&bus);
    let project = Project::new(&Config {
        command: Some(&command),
        ..Config::default()
    });
    let sandbox = project.sandbox("a11y-type");
    sandbox.succeed(&["up"]);
    bus.enable().expect("enable accessibility");

    let text = wait_for_control(bus.accessibility_address(), app_name(), "text");
    let x = text.bounds.x + text.bounds.width / 2;
    let y = text.bounds.y + text.bounds.height / 2;

    sandbox.succeed(&["click", &x.to_string(), &y.to_string()]);
    sandbox.wait_for_log("ruta-test-app: focus name=text");
    sandbox.succeed(&["type", "hi"]);

    let mut value = None;
    let typed = poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
        value = tree::read(bus.accessibility_address(), app_name())
            .ok()
            .and_then(|nodes| nodes.into_iter().find(|node| node.name == "text"))
            .and_then(|node| node.text);

        value.as_deref() == Some("hi")
    });

    assert!(
        typed,
        "the text field holds hi: {value:?}\n{}",
        sandbox.succeed(&["log"])
    );

    sandbox.succeed(&["down"]);
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, ImageMagick, dbus, and at-spi2-core"]
fn the_private_bus_leaves_no_process_behind() {
    in_test_namespace!();

    let bus = A11yBus::start().expect("start the accessibility bus");
    let session_address = bus.session_address().to_string();
    let socket = bus.socket_path();
    let command = app_command(&bus);
    let project = Project::new(&Config {
        command: Some(&command),
        ..Config::default()
    });
    let sandbox = project.sandbox("a11y-cleanup");
    sandbox.succeed(&["up"]);
    sandbox.succeed(&["down"]);

    drop(bus);

    let entry = format!("DBUS_SESSION_BUS_ADDRESS={session_address}");
    let gone = poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
        processes_with_environment(&entry).is_empty()
    });

    assert!(
        gone,
        "every process drops the private bus: {:?}",
        processes_with_environment(&entry)
    );
    assert!(!socket.exists(), "{} is removed", socket.display());
}

fn app_command(bus: &A11yBus) -> String {
    let app = test_app().to_str().expect("the test app path is UTF-8");
    let session = bus.session_address();
    let accessibility = bus.accessibility_address();

    format!(
        r#"["env", "DBUS_SESSION_BUS_ADDRESS={session}", "AT_SPI_BUS_ADDRESS={accessibility}", "{app}", "{TITLE}", "{}", "{}"]"#,
        WINDOW_SIZE.0, WINDOW_SIZE.1
    )
}

fn app_name() -> &'static str {
    test_app()
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .expect("the test app file name is UTF-8")
}

fn bounds_of(bounds: &Bounds) -> (i32, i32, i32, i32) {
    (bounds.x, bounds.y, bounds.width, bounds.height)
}
