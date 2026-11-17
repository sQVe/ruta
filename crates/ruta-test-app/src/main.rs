mod args;
mod layout;
mod modes;
#[cfg(test)]
mod test_support;
#[cfg(test)]
use test_support::namespace;
mod tree;

use std::io::Write as _;

use anyhow::Context as _;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ButtonPressEvent, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
    KeyButMask, KeyPressEvent, MotionNotifyEvent, PropMode, WindowClass,
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
enum Change {
    Pressed(&'static str),
    FocusedText,
    Text(String),
}

impl Change {
    fn line(&self) -> anyhow::Result<String> {
        let line = match self {
            Self::Pressed(name) => format!("press name={name}"),
            Self::FocusedText => "focus name=text".to_string(),
            Self::Text(text) => format!("text {}", serde_json::to_string(text)?),
        };

        Ok(line)
    }
}

fn main() -> anyhow::Result<()> {
    let options = args::parse(std::env::args().skip(1).collect())?;
    let window = options.window;
    let modes = options.modes;

    if modes.launch_child {
        return modes::launch_child(options.child_arguments);
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
    let graphics_context = connection.generate_id()?;
    connection.create_gc(
        graphics_context,
        id,
        &CreateGCAux::new().foreground(screen.black_pixel),
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
    draw(&connection, id, graphics_context)?;
    print_ready(&window)?;

    if let Some(path) = &modes.child_ready {
        modes::announce_ready(path)?;
    }

    let mut tree = tree::Tree::new(&window.title, (window.width, window.height));
    let mut pressed = Vec::new();

    loop {
        let event = connection.wait_for_event()?;
        let keysym = match &event {
            Event::KeyPress(key) | Event::KeyRelease(key) => keysym(&connection, key)?,
            _ => 0,
        };

        if let Some(line) = describe_input(keysym, &event) {
            write_line(&line)?;
        }

        if let Some(change) = describe_change(keysym, &event, &mut tree)? {
            write_line(&change.line()?)?;

            if let Change::Pressed(name) = change {
                pressed.push(name.to_string());

                if let Some(mode) = modes.after_press(&pressed) {
                    apply_mode(mode, &pressed)?;
                }
            }
        }

        if let Event::Expose(_) = event {
            draw(&connection, id, graphics_context)?;
        }
    }
}

fn apply_mode(mode: modes::Mode, pressed: &[String]) -> anyhow::Result<()> {
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
    graphics_context: u32,
) -> anyhow::Result<()> {
    connection.poly_fill_rectangle(window, graphics_context, &layout::drawn_rectangles())?;
    connection.flush()?;

    Ok(())
}

fn describe_change(
    keysym: u32,
    event: &Event,
    tree: &mut tree::Tree,
) -> anyhow::Result<Option<Change>> {
    match event {
        Event::ButtonPress(button) if button.detail == 1 => {
            tree.press();

            let x = i32::from(button.event_x);
            let y = i32::from(button.event_y);

            if let Some(name) = layout::hit_button(x, y) {
                return Ok(Some(Change::Pressed(name)));
            }

            if layout::hit_text_field(x, y) {
                tree.focus_text_field();

                return Ok(Some(Change::FocusedText));
            }

            Ok(None)
        }
        Event::KeyPress(_) => {
            if layout::printable(keysym).is_none() || !tree.type_character(keysym) {
                return Ok(None);
            }

            Ok(Some(Change::Text(tree.text())))
        }
        _ => Ok(None),
    }
}

// One log line per input event, in window coordinates. The state is the X modifier and button mask
// before the event.
fn describe_input(keysym: u32, event: &Event) -> Option<String> {
    let line = match event {
        Event::MotionNotify(motion) => describe_motion(motion),
        Event::ButtonPress(button) => describe_button("button-press", button),
        Event::ButtonRelease(button) => describe_button("button-release", button),
        Event::KeyPress(key) => describe_key("key-press", key, keysym),
        Event::KeyRelease(key) => describe_key("key-release", key, keysym),
        _ => return None,
    };

    Some(line)
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
fn describe_key(kind: &str, key: &KeyPressEvent, keysym: u32) -> String {
    format!(
        "{kind} keycode={} keysym={keysym:#x} state={:#x}",
        key.detail,
        u16::from(key.state)
    )
}

fn keysym(
    connection: &impl x11rb::connection::Connection,
    key: &KeyPressEvent,
) -> anyhow::Result<u32> {
    let mapping = connection.get_keyboard_mapping(key.detail, 1)?.reply()?;

    Ok(effective_keysym(&mapping.keysyms, u16::from(key.state)))
}

fn effective_keysym(keysyms: &[u32], state: u16) -> u32 {
    let shifted = state & u16::from(KeyButMask::SHIFT) != 0;
    let has_shifted_keysym = keysyms.get(1).is_some_and(|keysym| *keysym != 0);
    let index = usize::from(shifted && has_shifted_keysym);

    keysyms.get(index).copied().unwrap_or(0)
}

fn wait_for_expose(connection: &impl x11rb::connection::Connection) -> anyhow::Result<()> {
    loop {
        if let Event::Expose(_) = connection.wait_for_event()? {
            return Ok(());
        }
    }
}

fn print_ready(window: &args::Window) -> anyhow::Result<()> {
    let ready = args::ready_line(window);
    let mut stdout = std::io::stdout().lock();

    writeln!(stdout, "{ready}")?;
    writeln!(stdout, "ruta-test-app: running")?;
    stdout.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        assert_eq!(line, Some(Change::Pressed("alpha")));
    }

    #[test]
    fn a_press_on_the_text_field_focuses_it() {
        let mut tree = tree::Tree::new("demo", (640, 480));
        let (x, y) = centre(&layout::TEXT_FIELD);

        let line = describe_change(0, &button_press(1, x, y), &mut tree).expect("describe");

        assert_eq!(line, Some(Change::FocusedText));
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
        assert_eq!(line, Some(Change::Pressed("alpha")));

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
        assert_eq!(typed, Some(Change::Text("h".to_string())));
    }

    #[test]
    fn changes_keep_their_log_lines() {
        assert_eq!(
            Change::Pressed("alpha").line().expect("line"),
            "press name=alpha"
        );
        assert_eq!(Change::FocusedText.line().expect("line"), "focus name=text");
        assert_eq!(
            Change::Text("h\"i".to_string()).line().expect("line"),
            "text \"h\\\"i\""
        );
    }

    #[test]
    fn shift_selects_the_second_keysym() {
        let shift = u16::from(KeyButMask::SHIFT);

        assert_eq!(effective_keysym(&[0x68, 0x48], 0), 0x68);
        assert_eq!(effective_keysym(&[0x68, 0x48], shift), 0x48);
        assert_eq!(effective_keysym(&[0x68], shift), 0x68);
        assert_eq!(effective_keysym(&[0xffe1, 0], shift), 0xffe1);
        assert_eq!(effective_keysym(&[], 0), 0);
    }
}
