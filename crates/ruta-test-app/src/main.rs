mod layout;
mod modes;
mod tree;

use std::io::Write as _;

use anyhow::Context as _;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ButtonPressEvent, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
    KeyPressEvent, MotionNotifyEvent, PropMode, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_NAME,
        UTF8_STRING,
    }
}

const USAGE: &str = "Usage: ruta-test-app <title> <width> <height> \
                     [--geometry <x>,<y>,<width>,<height>,<scale>] \
                     [--panic-after <a,b>] [--exit-after <a,b>] [--launch-child] [arguments]";

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
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let mut window = parse_arguments(arguments.clone())?;
    let (modes, remaining) = modes::parse(&window.arguments)?;
    window.arguments = remaining;

    if modes.launch_child {
        return modes::launch_child(arguments);
    }

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
    let gc = connection.generate_id()?;
    connection.create_gc(gc, id, &CreateGCAux::new().foreground(screen.black_pixel))?;
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
    draw(&connection, id, gc)?;
    print_ready(&window)?;

    if let Some(path) = &modes.child_ready {
        modes::announce_ready(path)?;
    }

    let mut tree = tree::Tree::new(&window.title, (window.width, window.height));
    let mut pressed = Vec::new();

    loop {
        let event = connection.wait_for_event()?;

        if let Some(line) = describe_input(&connection, &event)? {
            write_line(&line)?;
        }

        let keysym = match &event {
            Event::KeyPress(key) => keysym(&connection, key)?,
            _ => 0,
        };

        if let Some(line) = describe_change(keysym, &event, &mut tree)? {
            write_line(&line)?;

            if let Some(name) = line.strip_prefix("press name=") {
                pressed.push(name.to_string());

                let pressed = pressed.iter().map(String::as_str).collect::<Vec<_>>();

                if let Some(mode) = modes.after_press(&pressed) {
                    apply_mode(mode, &pressed)?;
                }
            }
        }

        if let Event::Expose(_) = event {
            draw(&connection, id, gc)?;
        }
    }
}

fn apply_mode(mode: modes::Mode, pressed: &[&str]) -> anyhow::Result<()> {
    let sequence = pressed.join(",");

    match mode {
        modes::Mode::Panic => {
            panic!("ruta-test-app: panicking after the button sequence {sequence}")
        }
        modes::Mode::Exit => {
            write_line(&format!("exiting after {sequence}"))?;
            std::process::exit(0);
        }
    }
}

fn write_line(line: &str) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout().lock();

    writeln!(stdout, "ruta-test-app: {line}")?;
    stdout.flush()?;

    Ok(())
}

fn draw(
    connection: &impl x11rb::connection::Connection,
    window: u32,
    gc: u32,
) -> anyhow::Result<()> {
    connection.poly_fill_rectangle(window, gc, &layout::drawn_rectangles())?;
    connection.flush()?;

    Ok(())
}

fn describe_change(
    keysym: u32,
    event: &Event,
    tree: &mut tree::Tree,
) -> anyhow::Result<Option<String>> {
    match event {
        Event::ButtonPress(button) if button.detail == 1 => {
            tree.press();

            let x = i32::from(button.event_x);
            let y = i32::from(button.event_y);

            if let Some(name) = layout::hit_button(x, y) {
                return Ok(Some(format!("press name={name}")));
            }

            if layout::hit_text_field(x, y) {
                tree.focus_text_field();

                return Ok(Some("focus name=text".to_string()));
            }

            Ok(None)
        }
        Event::KeyPress(_) => {
            if layout::printable(keysym).is_none() || !tree.type_character(keysym) {
                return Ok(None);
            }

            Ok(Some(format!(
                "text {}",
                serde_json::to_string(&tree.text())?
            )))
        }
        _ => Ok(None),
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
    let keysym = keysym(connection, key)?;

    Ok(format!(
        "{kind} keycode={} keysym={keysym:#x} state={:#x}",
        key.detail,
        u16::from(key.state)
    ))
}

fn keysym(
    connection: &impl x11rb::connection::Connection,
    key: &KeyPressEvent,
) -> anyhow::Result<u32> {
    let mapping = connection.get_keyboard_mapping(key.detail, 1)?.reply()?;

    Ok(mapping.keysyms.first().copied().unwrap_or(0))
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

    fn button_press(detail: u8, x: i16, y: i16) -> Event {
        Event::ButtonPress(ButtonPressEvent {
            detail,
            event_x: x,
            event_y: y,
            ..ButtonPressEvent::default()
        })
    }

    fn centre(control: &layout::Control) -> (i16, i16) {
        let bounds = &control.bounds;
        let x = i16::try_from(bounds.x + bounds.width / 2).expect("a small coordinate");
        let y = i16::try_from(bounds.y + bounds.height / 2).expect("a small coordinate");

        (x, y)
    }

    #[test]
    fn a_press_on_a_button_reports_its_name() {
        let mut tree = tree::Tree::new("demo", (640, 480));
        let (x, y) = centre(&layout::BUTTONS[0]);

        let line = describe_change(0, &button_press(1, x, y), &mut tree).expect("describe");

        assert_eq!(line, Some("press name=alpha".to_string()));
    }

    #[test]
    fn a_press_on_the_text_field_focuses_it() {
        let mut tree = tree::Tree::new("demo", (640, 480));
        let (x, y) = centre(&layout::TEXT_FIELD);

        let line = describe_change(0, &button_press(1, x, y), &mut tree).expect("describe");

        assert_eq!(line, Some("focus name=text".to_string()));
    }

    #[test]
    fn a_press_outside_every_control_reports_nothing() {
        let mut tree = tree::Tree::new("demo", (640, 480));

        let line = describe_change(0, &button_press(1, 500, 400), &mut tree).expect("describe");

        assert_eq!(line, None);
    }

    #[test]
    fn a_press_with_another_button_reports_nothing() {
        let mut tree = tree::Tree::new("demo", (640, 480));
        let (x, y) = centre(&layout::BUTTONS[0]);

        let line = describe_change(0, &button_press(4, x, y), &mut tree).expect("describe");

        assert_eq!(line, None);
    }

    #[test]
    fn a_press_on_a_button_clears_the_text_field_focus() {
        let mut tree = tree::Tree::new("demo", (640, 480));
        tree.focus_text_field();
        let (x, y) = centre(&layout::BUTTONS[0]);

        let line = describe_change(0, &button_press(1, x, y), &mut tree).expect("describe");
        assert_eq!(line, Some("press name=alpha".to_string()));

        let typed = describe_change(0x68, &Event::KeyPress(KeyPressEvent::default()), &mut tree)
            .expect("describe");

        assert_eq!(typed, None);
    }

    #[test]
    fn a_printable_key_types_only_while_the_text_field_has_focus() {
        let mut tree = tree::Tree::new("demo", (640, 480));
        let key = Event::KeyPress(KeyPressEvent::default());

        let unfocused = describe_change(0x68, &key, &mut tree).expect("describe");
        assert_eq!(unfocused, None);

        tree.focus_text_field();

        let typed = describe_change(0x68, &key, &mut tree).expect("describe");
        assert_eq!(typed, Some("text \"h\"".to_string()));
    }
}
