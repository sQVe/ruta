use std::io::Write as _;

use anyhow::Context as _;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ButtonPressEvent, ConnectionExt as _, CreateWindowAux, EventMask, KeyPressEvent,
    MotionNotifyEvent, PropMode, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_NAME,
        UTF8_STRING,
    }
}

const USAGE: &str = "Usage: ruta-test-app <title> <width> <height> \
                     [--geometry <x>,<y>,<width>,<height>,<scale>] [arguments]";

#[derive(Debug, PartialEq)]
struct Window {
    title: String,
    width: u16,
    height: u16,
    geometry: Option<Geometry>,
    arguments: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

fn main() -> anyhow::Result<()> {
    let window = parse_arguments(std::env::args().skip(1).collect())?;

    writeln!(std::io::stderr(), "ruta-test-app: starting")?;

    let (connection, screen_number) = x11rb::connect(None).context("Could not reach DISPLAY")?;
    let screen = &connection.setup().roots[screen_number];
    let atoms = Atoms::new(&connection)?.reply()?;
    let id = connection.generate_id()?;
    let attributes = CreateWindowAux::new()
        .background_pixel(screen.white_pixel)
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::STRUCTURE_NOTIFY
                | EventMask::POINTER_MOTION
                | EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE
                | EventMask::KEY_PRESS
                | EventMask::KEY_RELEASE,
        );

    connection.create_window(
        screen.root_depth,
        id,
        screen.root,
        0,
        0,
        window.width,
        window.height,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &attributes,
    )?;
    connection.change_property8(
        PropMode::REPLACE,
        id,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        window.title.as_bytes(),
    )?;
    connection.change_property8(
        PropMode::REPLACE,
        id,
        atoms._NET_WM_NAME,
        atoms.UTF8_STRING,
        window.title.as_bytes(),
    )?;
    connection.map_window(id)?;
    connection.flush()?;

    wait_for_expose(&connection)?;
    print_ready(&window)?;

    loop {
        let event = connection.wait_for_event()?;

        if let Some(line) = describe_input(&connection, &event)? {
            let mut stdout = std::io::stdout().lock();

            writeln!(stdout, "ruta-test-app: {line}")?;
            stdout.flush()?;
        }
    }
}

// One log line per input event, in window coordinates. The state is the X modifier and button mask
// before the event.
fn describe_input(
    connection: &impl x11rb::connection::Connection,
    event: &Event,
) -> anyhow::Result<Option<String>> {
    let line = match event {
        Event::MotionNotify(motion) => describe_motion(motion),
        Event::ButtonPress(button) => describe_button("button-press", button),
        Event::ButtonRelease(button) => describe_button("button-release", button),
        Event::KeyPress(key) => describe_key(connection, "key-press", key)?,
        Event::KeyRelease(key) => describe_key(connection, "key-release", key)?,
        _ => return Ok(None),
    };

    Ok(Some(line))
}

fn describe_motion(motion: &MotionNotifyEvent) -> String {
    format!(
        "motion x={} y={} state={:#x}",
        motion.event_x,
        motion.event_y,
        u16::from(motion.state)
    )
}

fn describe_button(kind: &str, button: &ButtonPressEvent) -> String {
    format!(
        "{kind} x={} y={} button={} state={:#x}",
        button.event_x,
        button.event_y,
        button.detail,
        u16::from(button.state)
    )
}

// The keymap is read at each key, since another client, such as xdotool, may change it at any time.
fn describe_key(
    connection: &impl x11rb::connection::Connection,
    kind: &str,
    key: &KeyPressEvent,
) -> anyhow::Result<String> {
    let mapping = connection.get_keyboard_mapping(key.detail, 1)?.reply()?;
    let keysym = mapping.keysyms.first().copied().unwrap_or(0);

    Ok(format!(
        "{kind} keycode={} keysym={keysym:#x} state={:#x}",
        key.detail,
        u16::from(key.state)
    ))
}

fn parse_arguments(arguments: Vec<String>) -> anyhow::Result<Window> {
    let [title, width, height, rest @ ..] = arguments.as_slice() else {
        anyhow::bail!(USAGE);
    };

    Ok(Window {
        title: title.clone(),
        width: width.parse().context("The width must be a whole number")?,
        height: height
            .parse()
            .context("The height must be a whole number")?,
        geometry: parse_geometry(rest)?,
        arguments: rest.to_vec(),
    })
}

fn parse_geometry(arguments: &[String]) -> anyhow::Result<Option<Geometry>> {
    let Some(flag) = arguments
        .iter()
        .position(|argument| argument == "--geometry")
    else {
        return Ok(None);
    };
    let numbers: Option<Vec<f64>> = arguments
        .get(flag + 1)
        .map(|value| value.split(',').map(finite_number).collect())
        .unwrap_or_default();
    let Some([x, y, width, height, scale]) = numbers.as_deref() else {
        anyhow::bail!(USAGE);
    };

    Ok(Some(Geometry {
        x: *x,
        y: *y,
        width: *width,
        height: *height,
        scale: *scale,
    }))
}

fn finite_number(text: &str) -> Option<f64> {
    text.parse().ok().filter(|number: &f64| number.is_finite())
}

fn wait_for_expose(connection: &impl x11rb::connection::Connection) -> anyhow::Result<()> {
    loop {
        if let Event::Expose(_) = connection.wait_for_event()? {
            return Ok(());
        }
    }
}

fn print_ready(window: &Window) -> anyhow::Result<()> {
    let ready = ready_line(window);
    let mut stdout = std::io::stdout().lock();

    writeln!(stdout, "{ready}")?;
    writeln!(stdout, "ruta-test-app: running")?;
    stdout.flush()?;

    Ok(())
}

fn ready_line(window: &Window) -> serde_json::Value {
    let mut ready = serde_json::json!({
        "title": window.title,
        "width": window.width,
        "height": window.height,
        "arguments": window.arguments,
    });

    if let Some(geometry) = window.geometry {
        ready["origin"] = serde_json::json!({ "x": geometry.x, "y": geometry.y });
        ready["size"] = serde_json::json!({ "width": geometry.width, "height": geometry.height });
        ready["scale"] = serde_json::json!(geometry.scale);
    }

    ready
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn ready_line_without_geometry_keeps_its_shape() {
        let window = parse_arguments(strings(&["demo", "300", "200", "extra"])).expect("parse");

        assert_eq!(
            ready_line(&window),
            json!({ "title": "demo", "width": 300, "height": 200, "arguments": ["extra"] })
        );
    }

    #[test]
    fn geometry_adds_origin_size_and_scale() {
        let arguments = strings(&["demo", "300", "200", "--geometry", "10,20.5,100,50,2"]);
        let window = parse_arguments(arguments).expect("parse");

        assert_eq!(
            ready_line(&window),
            json!({
                "title": "demo",
                "width": 300,
                "height": 200,
                "arguments": ["--geometry", "10,20.5,100,50,2"],
                "origin": { "x": 10.0, "y": 20.5 },
                "size": { "width": 100.0, "height": 50.0 },
                "scale": 2.0,
            })
        );
    }

    #[test]
    fn geometry_without_a_good_value_fails_with_the_usage() {
        for value in [
            None,
            Some("10,20,100,50"),
            Some("10,20,100,50,2,3"),
            Some("10,twenty,100,50,2"),
        ] {
            let mut arguments = strings(&["demo", "300", "200", "--geometry"]);
            arguments.extend(value.map(ToString::to_string));

            let error = parse_arguments(arguments).expect_err("bad geometry");

            assert_eq!(error.to_string(), USAGE, "{value:?}");
        }
    }

    #[test]
    fn geometry_refuses_values_that_are_not_finite() {
        for bad in ["NaN", "inf", "-inf", "1e999"] {
            for field in 0..5 {
                let mut parts = ["10", "20", "100", "50", "2"];
                parts[field] = bad;
                let value = parts.join(",");
                let arguments = strings(&["demo", "300", "200", "--geometry", &value]);

                let error = parse_arguments(arguments).expect_err("not finite");

                assert_eq!(error.to_string(), USAGE, "{value}");
            }
        }
    }
}
