use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub(crate) const USAGE: &str = "\
Usage:
  ruta up [--headed] [--] [arguments]
  ruta restart [--headed] [--] [arguments]
  ruta shot [file]
  ruta log
  ruta opened
  ruta down

ruta reads ruta.toml from the current folder or a folder above it. Arguments after up go to the
app; --headed counts only before the first of them, and -- ends ruta's options. restart without
arguments reuses the arguments and the mode of the last up.

Set RUTA_NAME=<name> to run a second sandbox side by side.";

const HEADED: &str = "--headed";
const END_OF_OPTIONS: &str = "--";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Sandbox {
    Up {
        arguments: Vec<String>,
        headed: bool,
    },
    Restart {
        arguments: Option<Vec<String>>,
        headed: Option<bool>,
    },
    Shot {
        file: Option<PathBuf>,
    },
    Log,
    Opened,
    Down,
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{USAGE}")]
pub(crate) struct UsageError;

pub(crate) fn parse(arguments: &[String]) -> Result<Sandbox, UsageError> {
    match arguments {
        [command, rest @ ..] if command == "up" => {
            let options = split_options(rest);

            Ok(Sandbox::Up {
                arguments: options.arguments,
                headed: options.headed,
            })
        }
        [command, rest @ ..] if command == "restart" => {
            let options = split_options(rest);
            let given = options.ended || !options.arguments.is_empty();

            Ok(Sandbox::Restart {
                arguments: given.then_some(options.arguments),
                headed: options.headed.then_some(true),
            })
        }
        [command] if command == "shot" => Ok(Sandbox::Shot { file: None }),
        [command, file] if command == "shot" => Ok(Sandbox::Shot {
            file: Some(PathBuf::from(file)),
        }),
        [command] if command == "log" => Ok(Sandbox::Log),
        [command] if command == "opened" => Ok(Sandbox::Opened),
        [command] if command == "down" => Ok(Sandbox::Down),
        _ => Err(UsageError),
    }
}

struct Options {
    headed: bool,
    ended: bool,
    arguments: Vec<String>,
}

fn split_options(arguments: &[String]) -> Options {
    let mut options = Options {
        headed: false,
        ended: false,
        arguments: Vec::new(),
    };

    for (index, argument) in arguments.iter().enumerate() {
        match argument.as_str() {
            HEADED => options.headed = true,
            END_OF_OPTIONS => {
                options.ended = true;
                options.arguments = arguments[index + 1..].to_vec();
                break;
            }
            _ => {
                options.arguments = arguments[index..].to_vec();
                break;
            }
        }
    }

    options
}

fn find_program(name: &str, path: &str) -> Option<PathBuf> {
    path.split(':')
        .filter(|directory| !directory.is_empty())
        .map(|directory| Path::new(directory).join(name))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    let runnable = rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok();

    runnable && fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
}

fn tool_error(name: &str) -> String {
    let package = match name {
        "xdotool" => "the xdotool package",
        "weston" => "the weston package",
        "Xwayland" => "the xorg-xwayland package",
        "import" => "the imagemagick package",
        _ => name,
    };

    format!("The tool \"{name}\" is missing. Install {package} and try again.")
}

#[derive(Debug, Error)]
#[error("{}", tool_error(name))]
pub(crate) struct MissingTool {
    pub(crate) name: String,
}

pub(crate) fn require_program(name: &str) -> Result<(), MissingTool> {
    require_program_in(name, &std::env::var("PATH").unwrap_or_default())
}

fn require_program_in(name: &str, path: &str) -> Result<(), MissingTool> {
    if find_program(name, path).is_some() {
        return Ok(());
    }

    Err(MissingTool {
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
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
            strings(&["click", "1", "2"]),
            strings(&["nonsense"]),
        ] {
            assert_eq!(parse(&arguments), Err(UsageError), "{arguments:?}");
        }
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

        // 0o641 lets only others run a file that the current user owns.
        for mode in [0o644, 0o641] {
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
        assert!(tool_error("weston").contains("the weston package"));
    }
}
