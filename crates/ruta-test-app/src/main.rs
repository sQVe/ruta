//! Opens one X11 window and prints a JSON ready line, for ruta's end-to-end tests.
//!
//! Usage: `ruta-test-app <title> <width> <height> [arguments]`. The ready line repeats the title,
//! the size, and the extra arguments.

use std::io::Write as _;

use anyhow::Context as _;
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_NAME,
        UTF8_STRING,
    }
}

struct Window {
    title: String,
    width: u16,
    height: u16,
    arguments: Vec<String>,
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
        .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY);

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
        connection.wait_for_event()?;
    }
}

fn parse_arguments(arguments: Vec<String>) -> anyhow::Result<Window> {
    let [title, width, height, rest @ ..] = arguments.as_slice() else {
        anyhow::bail!("Usage: ruta-test-app <title> <width> <height> [arguments]");
    };

    Ok(Window {
        title: title.clone(),
        width: width.parse().context("The width must be a whole number")?,
        height: height
            .parse()
            .context("The height must be a whole number")?,
        arguments: rest.to_vec(),
    })
}

fn wait_for_expose(connection: &impl x11rb::connection::Connection) -> anyhow::Result<()> {
    loop {
        if let Event::Expose(_) = connection.wait_for_event()? {
            return Ok(());
        }
    }
}

fn print_ready(window: &Window) -> anyhow::Result<()> {
    let ready = serde_json::json!({
        "title": window.title,
        "width": window.width,
        "height": window.height,
        "arguments": window.arguments,
    });
    let mut stdout = std::io::stdout().lock();

    writeln!(stdout, "{ready}")?;
    writeln!(stdout, "ruta-test-app: running")?;
    stdout.flush()?;

    Ok(())
}
