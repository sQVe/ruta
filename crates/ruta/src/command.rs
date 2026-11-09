use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use thiserror::Error;

pub(crate) const USAGE: &str = "\
Usage:
  ruta up [--headed] [--] [arguments]
  ruta restart [--headed] [--] [arguments]
  ruta shot [file]
  ruta move <x0> <y0> <x1> <y1>
  ruta click <x> <y> [button]
  ruta drag <x0> <y0> <x1> <y1>
  ruta type <text>
  ruta key <keysym> [modifier]...
  ruta scroll <x> <y> [notches]
  ruta hold <keysym>...
  ruta release <keysym>...
  ruta log
  ruta opened
  ruta down

ruta reads ruta.toml from the current folder or a folder above it. Arguments after up go to the
app; --headed counts only before the first of them, and -- ends ruta's options. restart without
arguments reuses the arguments and the mode of the last up.

Input goes to the app window, in window pixels. scroll turns the wheel 1 notch by default; a
positive count scrolls down and a negative one up. hold keeps keys down for later commands until
release lets them go.

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
    Move {
        from: (i32, i32),
        to: (i32, i32),
    },
    Click {
        x: i32,
        y: i32,
        button: u8,
    },
    Drag {
        from: (i32, i32),
        to: (i32, i32),
    },
    Type {
        text: String,
    },
    Key {
        key: String,
        modifiers: Vec<String>,
    },
    Scroll {
        x: i32,
        y: i32,
        notches: i32,
    },
    Hold {
        keys: Vec<String>,
    },
    Release {
        keys: Vec<String>,
    },
    Log,
    Opened,
    Down,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ParseError {
    #[error("{USAGE}")]
    Usage,
    #[error("{argument} must be a whole number, not \"{value}\"")]
    NotANumber {
        argument: &'static str,
        value: String,
    },
    #[error("scroll needs a nonzero number of notches")]
    NoNotches,
}

pub(crate) fn parse(arguments: &[String]) -> Result<Sandbox, ParseError> {
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
        [command, x0, y0, x1, y1] if command == "move" => Ok(Sandbox::Move {
            from: (number(x0, "x0")?, number(y0, "y0")?),
            to: (number(x1, "x1")?, number(y1, "y1")?),
        }),
        [command, x0, y0, x1, y1] if command == "drag" => Ok(Sandbox::Drag {
            from: (number(x0, "x0")?, number(y0, "y0")?),
            to: (number(x1, "x1")?, number(y1, "y1")?),
        }),
        [command, x, y] if command == "click" => Ok(Sandbox::Click {
            x: number(x, "x")?,
            y: number(y, "y")?,
            button: 1,
        }),
        [command, x, y, button] if command == "click" => Ok(Sandbox::Click {
            x: number(x, "x")?,
            y: number(y, "y")?,
            button: number(button, "button")?,
        }),
        [command, text @ ..] if command == "type" && !text.is_empty() => Ok(Sandbox::Type {
            text: text.join(" "),
        }),
        [command, key, modifiers @ ..] if command == "key" => Ok(Sandbox::Key {
            key: key.clone(),
            modifiers: modifiers.to_vec(),
        }),
        [command, x, y] if command == "scroll" => Ok(Sandbox::Scroll {
            x: number(x, "x")?,
            y: number(y, "y")?,
            notches: 1,
        }),
        [command, x, y, notches] if command == "scroll" => scroll(x, y, notches),
        [command, keys @ ..] if command == "hold" && !keys.is_empty() => Ok(Sandbox::Hold {
            keys: keys.to_vec(),
        }),
        [command, keys @ ..] if command == "release" && !keys.is_empty() => Ok(Sandbox::Release {
            keys: keys.to_vec(),
        }),
        [command] if command == "log" => Ok(Sandbox::Log),
        [command] if command == "opened" => Ok(Sandbox::Opened),
        [command] if command == "down" => Ok(Sandbox::Down),
        _ => Err(ParseError::Usage),
    }
}

fn scroll(x: &str, y: &str, notches: &str) -> Result<Sandbox, ParseError> {
    let x = number(x, "x")?;
    let y = number(y, "y")?;
    let notches = number(notches, "notches")?;

    if notches == 0 {
        return Err(ParseError::NoNotches);
    }

    Ok(Sandbox::Scroll { x, y, notches })
}

fn number<T: FromStr>(value: &str, argument: &'static str) -> Result<T, ParseError> {
    value.parse().map_err(|_| ParseError::NotANumber {
        argument,
        value: value.to_string(),
    })
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
        "import" | "magick" => "the imagemagick package",
        "cat" => "the coreutils package",
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

pub(crate) fn require_program_in(name: &str, path: &str) -> Result<(), MissingTool> {
    if find_program(name, path).is_some() {
        return Ok(());
    }

    Err(MissingTool {
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests;
