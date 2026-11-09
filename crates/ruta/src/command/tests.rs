use super::*;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
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
                headed: None,
            },
        ),
        (
            strings(&["restart", "--headed", "smoke-3"]),
            Sandbox::Restart {
                arguments: Some(strings(&["smoke-3"])),
                headed: Some(true),
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
        assert_eq!(parse(&arguments), Ok(expected), "{arguments:?}");
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
        assert_eq!(parse(&arguments), Ok(expected), "{arguments:?}");
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
        Ok(Sandbox::Up {
            arguments: strings(&["smoke-2", "--headed"]),
            headed: false,
        })
    );
    assert_eq!(
        parse(&strings(&["restart", "smoke-2", "--headed"])),
        Ok(Sandbox::Restart {
            arguments: Some(strings(&["smoke-2", "--headed"])),
            headed: None,
        })
    );
}

#[test]
fn end_of_options_passes_the_rest_to_the_app() {
    assert_eq!(
        parse(&strings(&["up", "--headed", "--", "--headed", "--"])),
        Ok(Sandbox::Up {
            arguments: strings(&["--headed", "--"]),
            headed: true,
        })
    );
}

#[test]
fn restart_with_only_end_of_options_clears_the_arguments() {
    assert_eq!(
        parse(&strings(&["restart", "--"])),
        Ok(Sandbox::Restart {
            arguments: Some(Vec::new()),
            headed: None,
        })
    );
    assert_eq!(
        parse(&strings(&["restart", "--headed"])),
        Ok(Sandbox::Restart {
            arguments: None,
            headed: Some(true),
        })
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

#[test]
fn find_program_uses_the_given_path() {
    let directory = crate::test_support::temporary_directory();
    let program = directory.path().join("fake-tool");
    std::fs::write(&program, "#!/bin/sh\n").expect("write the fake tool");
    std::fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("make the tool runnable");

    let found = find_program("fake-tool", &directory.path().to_string_lossy());
    assert_eq!(found, Some(program));
    assert_eq!(find_program("fake-tool", "/nonexistent"), None);
}

#[test]
fn find_program_skips_a_file_that_is_not_executable() {
    use std::os::unix::fs::PermissionsExt as _;

    let plain = crate::test_support::temporary_directory();
    let runnable = crate::test_support::temporary_directory();
    let skipped = plain.path().join("fake-tool");
    let program = runnable.path().join("fake-tool");
    for path in [&skipped, &program] {
        std::fs::write(path, "#!/bin/sh\n").expect("write the fake tool");
    }
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
        .expect("make the tool runnable");
    let plain_path = plain.path().to_string_lossy().into_owned();
    let both = format!("{plain_path}:{}", runnable.path().display());

    // 0o641 lets only others run a file that the current user owns. Root may run any file with
    // an execute bit, so that case applies only to other users.
    let modes: &[u32] = if rustix::process::geteuid().is_root() {
        &[0o644]
    } else {
        &[0o644, 0o641]
    };

    for &mode in modes {
        std::fs::set_permissions(&skipped, std::fs::Permissions::from_mode(mode))
            .expect("set the tool mode");

        assert_eq!(find_program("fake-tool", &plain_path), None, "{mode:o}");
        assert_eq!(
            find_program("fake-tool", &both),
            Some(program.clone()),
            "{mode:o}"
        );
    }
}

#[test]
fn require_program_reports_a_missing_tool() {
    let error = require_program_in("no-such-tool", "/nonexistent").expect_err("missing tool");

    assert!(matches!(error, MissingTool { ref name } if name == "no-such-tool"));
    assert_eq!(
        error.to_string(),
        "The tool \"no-such-tool\" is missing. Install no-such-tool and try again."
    );
}

#[test]
fn tool_error_names_the_tool_and_package() {
    assert!(tool_error("xdotool").contains("xdotool"));
    assert!(tool_error("Xwayland").contains("xorg-xwayland"));
    assert!(tool_error("import").contains("imagemagick"));
    assert!(tool_error("cat").contains("coreutils"));
    assert!(tool_error("weston").contains("the weston package"));
}
