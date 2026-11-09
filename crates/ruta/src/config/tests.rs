use std::path::{Path, PathBuf};

use super::*;

const FOLDER: &str = "/work/project";

const VALID: &str = r#"
name = "greppa"

[app]
command = ["target/debug/greppa-scene", "--scene"]
title = "greppa"
width = 1400
height = 900

[ready]
line = "json"

[environment]
RUST_LOG = "info"
"#;

fn parse_text(text: &str) -> Result<Config, ConfigError> {
    parse(text, Path::new(FOLDER), None)
}

fn refused(text: &str, case: &str) -> ConfigError {
    match parse_text(text) {
        Ok(config) => panic!("{case}: the config was accepted: {config:?}"),
        Err(error) => error,
    }
}

#[test]
fn an_unknown_key_is_named() {
    let text = VALID.replace("title = \"greppa\"", "title = \"greppa\"\ncolour = \"red\"");

    let error = parse_text(&text).expect_err("an unknown key is refused");

    assert_eq!(
        error,
        ConfigError::UnknownKey {
            key: "app.colour".to_string()
        }
    );
}

#[test]
fn an_unknown_key_is_named_at_the_top_and_in_ready() {
    let cases = [
        ("top", format!("version = 2\n{VALID}"), "version"),
        (
            "ready",
            VALID.replace("line = \"json\"", "line = \"json\"\ntimeout = 5"),
            "ready.timeout",
        ),
    ];

    for (case, text, key) in cases {
        assert_eq!(
            refused(&text, case),
            ConfigError::UnknownKey {
                key: key.to_string()
            },
            "{case}"
        );
    }
}

#[test]
fn a_missing_key_is_named() {
    let cases = [
        ("name = \"greppa\"\n", "name"),
        ("title = \"greppa\"\n", "app.title"),
        ("width = 1400\n", "app.width"),
        ("height = 900\n", "app.height"),
        (
            "command = [\"target/debug/greppa-scene\", \"--scene\"]\n",
            "app.command",
        ),
    ];

    for (removed, key) in cases {
        let text = VALID.replace(removed, "");

        assert_eq!(
            refused(&text, key),
            ConfigError::MissingKey {
                key: key.to_string()
            },
            "{key}"
        );
    }
}

#[test]
fn a_missing_table_is_named() {
    let without_app = "name = \"greppa\"

[ready]
line = \"json\"
";
    let without_ready = "name = \"greppa\"

[app]
command = [\"app\"]
title = \"greppa\"
width = 1
height = 1
";

    for (text, key) in [(without_app, "app"), (without_ready, "ready")] {
        assert_eq!(
            refused(text, key),
            ConfigError::MissingKey {
                key: key.to_string()
            },
            "{key}"
        );
    }
}

#[test]
fn both_ready_forms_are_refused() {
    let text = VALID.replace("line = \"json\"", "line = \"json\"\ncommand = [\"probe\"]");

    assert_eq!(refused(&text, "both"), ConfigError::BothReadyForms);
}

#[test]
fn neither_ready_form_is_refused() {
    let text = VALID.replace("line = \"json\"", "");

    assert_eq!(refused(&text, "neither"), ConfigError::NoReadyForm);
}

#[test]
fn a_ready_line_other_than_json_is_refused() {
    let text = VALID.replace("line = \"json\"", "line = \"text\"");

    assert_eq!(
        refused(&text, "text"),
        ConfigError::WrongType {
            key: "ready.line".to_string(),
            expected: "\"json\"",
        }
    );
}

#[test]
fn a_bad_name_is_named() {
    for name in ["Greppa", "a/b", "a b", "", "-lead", &"a".repeat(33)] {
        let text = VALID.replace("name = \"greppa\"", &format!("name = \"{name}\""));

        assert_eq!(
            refused(&text, name),
            ConfigError::BadName {
                key: "name".to_string(),
                name: name.to_string()
            },
            "{name:?}"
        );
    }
}

#[test]
fn a_good_name_is_accepted() {
    for name in ["alpha-2", "2", &"a".repeat(32)] {
        assert_eq!(valid_name("name", name), Ok(name.to_string()), "{name}");
    }
}

#[test]
fn ruta_name_replaces_the_configured_name() {
    let config = parse(VALID, Path::new(FOLDER), Some("second")).expect("the config parses");

    assert_eq!(config.name, "second");
}

#[test]
fn a_bad_ruta_name_is_named() {
    let error = parse(VALID, Path::new(FOLDER), Some("Two")).expect_err("the name is refused");

    assert_eq!(
        error,
        ConfigError::BadName {
            key: "RUTA_NAME".to_string(),
            name: "Two".to_string()
        }
    );
}

#[test]
fn an_empty_command_is_refused() {
    let cases = [
        (
            "app",
            VALID.replace(
                "command = [\"target/debug/greppa-scene\", \"--scene\"]",
                "command = []",
            ),
            "app.command",
        ),
        (
            "app program",
            VALID.replace(
                "command = [\"target/debug/greppa-scene\", \"--scene\"]",
                "command = [\"\"]",
            ),
            "app.command",
        ),
        (
            "ready",
            VALID.replace("line = \"json\"", "command = []"),
            "ready.command",
        ),
    ];

    for (case, text, key) in cases {
        assert_eq!(
            refused(&text, case),
            ConfigError::EmptyCommand {
                key: key.to_string()
            },
            "{case}"
        );
    }
}

#[test]
fn a_zero_size_is_refused() {
    let cases = [
        ("width = 1400", "width = 0", "app.width"),
        ("height = 900", "height = 0", "app.height"),
    ];

    for (original, replacement, key) in cases {
        let text = VALID.replace(original, replacement);

        assert_eq!(
            refused(&text, key),
            ConfigError::ZeroSize {
                key: key.to_string()
            },
            "{key}"
        );
    }
}

#[test]
fn a_value_of_the_wrong_type_is_named() {
    let cases = [
        ("width = 1400", "width = -1", "app.width"),
        ("width = 1400", "width = \"wide\"", "app.width"),
        ("title = \"greppa\"", "title = 7", "app.title"),
        (
            "command = [\"target/debug/greppa-scene\", \"--scene\"]",
            "command = \"greppa\"",
            "app.command",
        ),
        (
            "command = [\"target/debug/greppa-scene\", \"--scene\"]",
            "command = [\"greppa\", 2]",
            "app.command",
        ),
        (
            "RUST_LOG = \"info\"",
            "RUST_LOG = 1",
            "environment.RUST_LOG",
        ),
    ];

    for (original, replacement, key) in cases {
        let text = VALID.replace(original, replacement);

        assert!(
            matches!(
                refused(&text, replacement),
                ConfigError::WrongType { key: refused_key, .. } if refused_key == key
            ),
            "{replacement}"
        );
    }
}

#[test]
fn an_environment_key_that_ruta_sets_is_refused() {
    for key in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "HOME",
        "PATH",
        "XDG_RUNTIME_DIR",
        "XDG_CONFIG_HOME",
        "RUTA_ROOT",
        "XAUTHORITY",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        let text = VALID.replace("RUST_LOG = \"info\"", &format!("{key} = \"x\""));

        assert_eq!(
            refused(&text, key),
            ConfigError::ReservedEnvironment {
                key: key.to_string()
            },
            "{key}"
        );
    }
}

#[test]
fn an_environment_key_that_is_not_a_variable_name_is_refused() {
    for key in ["", "A=B"] {
        let text = VALID.replace("RUST_LOG = \"info\"", &format!("\"{key}\" = \"x\""));

        assert_eq!(
            refused(&text, key),
            ConfigError::BadEnvironmentKey {
                key: key.to_string()
            },
            "{key:?}"
        );
    }
}

#[test]
fn a_syntax_error_is_refused() {
    assert!(matches!(
        refused("name = ", "syntax"),
        ConfigError::Syntax { .. }
    ));
}

#[test]
fn a_valid_config_is_parsed_and_its_relative_program_resolved() {
    let config = parse_text(VALID).expect("the config parses");

    assert_eq!(
        config,
        Config {
            name: "greppa".to_string(),
            folder: PathBuf::from(FOLDER),
            app: App {
                command: CommandLine {
                    program: PathBuf::from("/work/project/target/debug/greppa-scene"),
                    arguments: vec!["--scene".to_string()],
                },
                title: "greppa".to_string(),
                width: 1400,
                height: 900,
            },
            ready: Ready::Line,
            environment: vec![("RUST_LOG".to_string(), "info".to_string())],
        }
    );
}

#[test]
fn a_ready_command_and_no_environment_are_accepted() {
    let text = VALID
        .replace("line = \"json\"", "command = [\"./probe\", \"--ready\"]")
        .replace("[environment]\nRUST_LOG = \"info\"\n", "");

    let config = parse_text(&text).expect("the config parses");

    assert_eq!(
        config.ready,
        Ready::Command(CommandLine {
            program: PathBuf::from("/work/project/./probe"),
            arguments: vec!["--ready".to_string()],
        })
    );
    assert_eq!(config.environment, Vec::new());
}

#[test]
fn a_bare_or_absolute_program_is_kept() {
    for program in ["cargo", "/usr/bin/app"] {
        let text = VALID.replace("target/debug/greppa-scene", program);

        let config = parse_text(&text).expect("the config parses");

        assert_eq!(
            config.app.command.program,
            PathBuf::from(program),
            "{program}"
        );
    }
}

#[test]
fn load_finds_the_config_in_a_folder_above() {
    let folder = crate::test_support::temporary_directory();
    let nested = folder.path().join("a").join("b");
    std::fs::create_dir_all(&nested).expect("create the nested folder");
    std::fs::write(folder.path().join(FILE_NAME), VALID).expect("write the config");

    let config = load(&nested, None).expect("the config is found");

    assert_eq!(config.folder, folder.path());
    assert_eq!(
        config.app.command.program,
        folder.path().join("target/debug/greppa-scene")
    );
}

#[test]
fn load_reports_a_missing_config() {
    let folder = crate::test_support::temporary_directory();

    let error = load(folder.path(), None).expect_err("no config is found");

    assert_eq!(
        error.downcast_ref::<ConfigError>(),
        Some(&ConfigError::NotFound {
            start: folder.path().to_path_buf()
        })
    );
}

#[test]
fn the_config_errors_name_the_key() {
    let cases = [
        (
            ConfigError::UnknownKey {
                key: "app.colour".to_string(),
            },
            "unknown key \"app.colour\"",
        ),
        (
            ConfigError::MissingKey {
                key: "app.title".to_string(),
            },
            "missing key \"app.title\"",
        ),
        (
            ConfigError::BothReadyForms,
            "set one of \"ready.line\" and \"ready.command\", not both",
        ),
        (
            ConfigError::NoReadyForm,
            "set one of \"ready.line\" and \"ready.command\"",
        ),
        (
            ConfigError::BadName {
                key: "name".to_string(),
                name: "Two".to_string(),
            },
            "\"name\" must be one name of lowercase letters, digits, and dashes, at most 32 bytes, \
             not \"Two\"",
        ),
        (
            ConfigError::ReservedEnvironment {
                key: "DISPLAY".to_string(),
            },
            "\"environment.DISPLAY\" is set by ruta and cannot be changed",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}
