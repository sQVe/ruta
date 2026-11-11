use super::*;

use crate::test_support::strings;

#[test]
fn capture_requires_out_and_passes_app_arguments() {
    let parsed = parse(&strings(&["capture", "--out", "shots", "--", "smoke-2"]));
    assert_eq!(
        parsed,
        Ok(Command::Sandbox(Sandbox::Capture {
            out: PathBuf::from("shots"),
            arguments: strings(&["smoke-2"]),
        }))
    );
    for arguments in [
        strings(&["capture"]),
        strings(&["capture", "--out"]),
        strings(&["capture", "shots"]),
        strings(&["capture", "--wrong", "shots"]),
        strings(&["nonsense", "--out", "shots"]),
        strings(&["capture", "--headed", "--out", "shots"]),
        strings(&["capture", "--out", "--"]),
        strings(&["capture", "--out", ""]),
    ] {
        assert_eq!(parse(&arguments), Err(ParseError::Usage), "{arguments:?}");
    }
    for arguments in [strings(&[]), strings(&["--headed", "--"])] {
        let mut input = strings(&["capture", "--out", "shots"]);
        input.extend(arguments.clone());
        assert_eq!(
            parse(&input),
            Ok(Command::Sandbox(Sandbox::Capture {
                out: PathBuf::from("shots"),
                arguments,
            }))
        );
    }
}

#[test]
fn usage_lists_capture() {
    assert!(USAGE.contains("ruta capture --out <folder>"));
}

#[test]
fn usage_lists_status_and_guide() {
    for command in ["status", "guide"] {
        assert!(
            USAGE.contains(&format!("\n  ruta {command}\n")),
            "{command}: {USAGE}"
        );
    }
}

#[test]
fn parse_reads_guide_apart_from_the_sandbox_commands() {
    assert_eq!(parse(&strings(&["guide"])), Ok(Command::Guide));
    assert_eq!(
        parse(&strings(&["status"])),
        Ok(Command::Sandbox(Sandbox::Status))
    );
    assert_eq!(
        parse(&strings(&["down"])),
        Ok(Command::Sandbox(Sandbox::Down))
    );

    for arguments in [
        strings(&[]),
        strings(&["guide", "x"]),
        strings(&["status", "x"]),
    ] {
        assert_eq!(parse(&arguments), Err(ParseError::Usage), "{arguments:?}");
    }
}

#[test]
fn guide_keeps_its_text() {
    assert_eq!(
        GUIDE,
        "\
Every session:
- Run every command from the project folder, with RUTA_NAME=<name> set. Without RUTA_NAME, every
  worktree of a repository shares the one name in ruta.toml.
- A name starts with a lowercase letter or a digit, and holds at most 32 lowercase letters,
  digits, and hyphens.
- Touch no sandbox with another name, and no other /var/tmp/ruta-* folder.
- The shot is the only view of the window. Pass shot a file outside the sandbox folder.
- Never call xdotool, import, or ydotool, or start the app, yourself. They reach the real display.
- Follow each hold with a release of the same keys.

The session that starts the sandbox:
- Pick a name that no other running sandbox has.
- ruta up prints `ruta <name> is up on <display> (log: /var/tmp/ruta-<name>/app.log)`.
- ruta down prints `ruta <name> is gone`. Run it when the work is done.
- up --headed and restart --headed open a window on the user's real screen. Ask the user first.

A session that did not start the sandbox:
- Use only the name you were given.
- `ruta status` must exit 0 before any input. It then prints
  `ruta <name> is up and serves <folder>`.
- Never run up, restart, down, or capture. Only the session that started the sandbox runs them.
- You may run status, shot, move, click, drag, scroll, type, key, hold, release, log, and opened.
- Write under /var/tmp/ruta-<name> only where your brief says so."
    );
}

#[test]
fn guide_sorts_every_sandbox_command_of_the_usage_text() {
    let mut commands: Vec<&str> = USAGE
        .lines()
        .filter_map(|line| line.strip_prefix("  ruta "))
        .filter_map(|line| line.split(' ').next())
        .filter(|command| *command != "guide")
        .collect();
    commands.sort_unstable();
    let refused = ["up", "restart", "down", "capture"];
    let allowed = [
        "status", "shot", "move", "click", "drag", "scroll", "type", "key", "hold", "release",
        "log", "opened",
    ];
    let mut sorted: Vec<&str> = refused.into_iter().chain(allowed).collect();
    sorted.sort_unstable();

    assert_eq!(commands, sorted);
    assert!(GUIDE.contains(&format!(
        "- Never run {}, or {}.",
        refused[..3].join(", "),
        refused[3]
    )));
    assert!(GUIDE.contains(&format!(
        "- You may run {}, and {}.",
        allowed[..11].join(", "),
        allowed[11]
    )));
}

#[test]
fn parse_reads_every_command() {
    let cases = [
        (
            strings(&["up"]),
            Sandbox::Up {
                arguments: Vec::new(),
                headed: false,
            },
        ),
        (
            strings(&["up", "--headed", "smoke-2", "--fast"]),
            Sandbox::Up {
                arguments: strings(&["smoke-2", "--fast"]),
                headed: true,
            },
        ),
        (
            strings(&["restart"]),
            Sandbox::Restart {
                arguments: None,
                force_headed: false,
            },
        ),
        (
            strings(&["restart", "--headed", "smoke-3"]),
            Sandbox::Restart {
                arguments: Some(strings(&["smoke-3"])),
                force_headed: true,
            },
        ),
        (strings(&["shot"]), Sandbox::Shot { file: None }),
        (
            strings(&["shot", "/tmp/a.png"]),
            Sandbox::Shot {
                file: Some(PathBuf::from("/tmp/a.png")),
            },
        ),
        (strings(&["log"]), Sandbox::Log),
        (strings(&["opened"]), Sandbox::Opened),
        (strings(&["down"]), Sandbox::Down),
    ];

    for (arguments, expected) in cases {
        assert_eq!(
            parse(&arguments),
            Ok(Command::Sandbox(expected)),
            "{arguments:?}"
        );
    }
}

#[test]
fn parse_reads_every_input_command() {
    let cases = [
        (
            strings(&["move", "1", "-2", "30", "40"]),
            Sandbox::Move {
                from: (1, -2),
                to: (30, 40),
            },
        ),
        (
            strings(&["click", "10", "20"]),
            Sandbox::Click {
                x: 10,
                y: 20,
                button: 1,
            },
        ),
        (
            strings(&["click", "10", "20", "3"]),
            Sandbox::Click {
                x: 10,
                y: 20,
                button: 3,
            },
        ),
        (
            strings(&["drag", "1", "2", "3", "4"]),
            Sandbox::Drag {
                from: (1, 2),
                to: (3, 4),
            },
        ),
        (
            strings(&["type", "hello", "world"]),
            Sandbox::Type {
                text: "hello world".to_string(),
            },
        ),
        (
            strings(&["key", "Return"]),
            Sandbox::Key {
                key: "Return".to_string(),
                modifiers: Vec::new(),
            },
        ),
        (
            strings(&["key", "s", "ctrl", "shift"]),
            Sandbox::Key {
                key: "s".to_string(),
                modifiers: strings(&["ctrl", "shift"]),
            },
        ),
        (
            strings(&["scroll", "5", "6"]),
            Sandbox::Scroll {
                x: 5,
                y: 6,
                notches: 1,
            },
        ),
        (
            strings(&["scroll", "5", "6", "-2"]),
            Sandbox::Scroll {
                x: 5,
                y: 6,
                notches: -2,
            },
        ),
        (
            strings(&["hold", "Shift_L", "Control_L"]),
            Sandbox::Hold {
                keys: strings(&["Shift_L", "Control_L"]),
            },
        ),
        (
            strings(&["release", "Shift_L"]),
            Sandbox::Release {
                keys: strings(&["Shift_L"]),
            },
        ),
    ];

    for (arguments, expected) in cases {
        assert_eq!(
            parse(&arguments),
            Ok(Command::Sandbox(expected)),
            "{arguments:?}"
        );
    }
}

#[test]
fn scroll_refuses_zero_notches() {
    assert_eq!(
        parse(&strings(&["scroll", "5", "6", "0"])),
        Err(ParseError::NoNotches)
    );
}

#[test]
fn usage_lists_every_input_command() {
    for command in [
        "move", "click", "drag", "type", "key", "scroll", "hold", "release",
    ] {
        assert!(
            USAGE.contains(&format!("\n  ruta {command} ")),
            "{command}: {USAGE}"
        );
    }
}

#[test]
fn headed_after_the_first_app_argument_goes_to_the_app() {
    assert_eq!(
        parse(&strings(&["up", "smoke-2", "--headed"])),
        Ok(Command::Sandbox(Sandbox::Up {
            arguments: strings(&["smoke-2", "--headed"]),
            headed: false,
        }))
    );
    assert_eq!(
        parse(&strings(&["restart", "smoke-2", "--headed"])),
        Ok(Command::Sandbox(Sandbox::Restart {
            arguments: Some(strings(&["smoke-2", "--headed"])),
            force_headed: false,
        }))
    );
}

#[test]
fn end_of_options_passes_the_rest_to_the_app() {
    assert_eq!(
        parse(&strings(&["up", "--headed", "--", "--headed", "--"])),
        Ok(Command::Sandbox(Sandbox::Up {
            arguments: strings(&["--headed", "--"]),
            headed: true,
        }))
    );
}

#[test]
fn restart_with_only_end_of_options_clears_the_arguments() {
    assert_eq!(
        parse(&strings(&["restart", "--"])),
        Ok(Command::Sandbox(Sandbox::Restart {
            arguments: Some(Vec::new()),
            force_headed: false,
        }))
    );
    assert_eq!(
        parse(&strings(&["restart", "--headed"])),
        Ok(Command::Sandbox(Sandbox::Restart {
            arguments: None,
            force_headed: true,
        }))
    );
}

#[test]
fn parse_rejects_bad_shapes() {
    for arguments in [
        strings(&[]),
        strings(&["shot", "a.png", "b.png"]),
        strings(&["log", "extra"]),
        strings(&["opened", "extra"]),
        strings(&["down", "extra"]),
        strings(&["move", "1", "2", "3"]),
        strings(&["click", "1"]),
        strings(&["click", "1", "2", "3", "4"]),
        strings(&["drag", "1", "2", "3", "4", "5"]),
        strings(&["type"]),
        strings(&["key"]),
        strings(&["scroll", "1"]),
        strings(&["scroll", "1", "2", "3", "4"]),
        strings(&["hold"]),
        strings(&["release"]),
        strings(&["--help"]),
        strings(&["nonsense"]),
    ] {
        assert_eq!(parse(&arguments), Err(ParseError::Usage), "{arguments:?}");
    }
}

#[test]
fn parse_names_a_bad_number() {
    let cases = [
        (strings(&["click", "x", "2"]), "x", "x"),
        (strings(&["click", "1", "2.5"]), "y", "2.5"),
        (strings(&["click", "1", "2", "left"]), "button", "left"),
        (strings(&["move", "1", "2", "3", "far"]), "y1", "far"),
        (strings(&["drag", "1", "2", "3", "y1"]), "y1", "y1"),
        (strings(&["drag", "a", "2", "3", "4"]), "x0", "a"),
        (strings(&["scroll", "1", "2", "many"]), "notches", "many"),
        (strings(&["scroll", "1", "down"]), "y", "down"),
    ];

    for (arguments, argument, value) in cases {
        assert_eq!(
            parse(&arguments),
            Err(ParseError::NotANumber {
                argument,
                value: value.to_string(),
            }),
            "{arguments:?}"
        );
    }
}

#[test]
fn a_bad_number_error_names_the_argument_and_value() {
    let error = parse(&strings(&["click", "x", "2"])).expect_err("x is not a number");

    assert_eq!(error.to_string(), "x must be a whole number, not \"x\"");
}
