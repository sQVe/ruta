use std::path::PathBuf;
use std::str::FromStr;

use thiserror::Error;

pub(crate) const USAGE: &str = "\
Usage:
  ruta up [--headed] [--] [arguments]
  ruta restart [--headed] [--] [arguments]
  ruta capture --out <folder> [--] [arguments]
  ruta shot [file]
  ruta move <x0> <y0> <x1> <y1>
  ruta click <x> <y> [button]
  ruta drag <x0> <y0> <x1> <y1>
  ruta type <text>
  ruta key <keysym> [modifier]...
  ruta scroll <x> <y> [notches]
  ruta hold <keysym>...
  ruta release <keysym>...
  ruta status
  ruta guide
  ruta log
  ruta opened
  ruta down

ruta reads ruta.toml from the current folder or a folder above it. Arguments after up go to the
app; --headed counts only before the first of them, and -- ends ruta's options. restart without
arguments reuses the arguments and the mode of the last up.

Input goes to the app window, in window pixels. scroll turns the wheel 1 notch by default; a
positive count scrolls down and a negative one up. hold keeps keys down for later commands until
release lets them go.

status exits 0 only when the sandbox is up and was started from this project folder. guide prints
the rules for the session that starts a sandbox and for a session that only drives it.

Set RUTA_NAME=<name> to run a second sandbox side by side.";

pub(crate) const GUIDE: &str = "\
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
- Write under /var/tmp/ruta-<name> only where your brief says so.";

const HEADED: &str = "--headed";
const END_OF_OPTIONS: &str = "--";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Guide,
    Sandbox(Sandbox),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Sandbox {
    Up {
        arguments: Vec<String>,
        headed: bool,
    },
    Restart {
        arguments: Option<Vec<String>>,
        force_headed: bool,
    },
    Capture {
        out: PathBuf,
        arguments: Vec<String>,
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
    Status,
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

pub(crate) fn parse(arguments: &[String]) -> Result<Command, ParseError> {
    if arguments == ["guide"] {
        return Ok(Command::Guide);
    }

    parse_sandbox(arguments).map(Command::Sandbox)
}

fn parse_sandbox(arguments: &[String]) -> Result<Sandbox, ParseError> {
    let (command, rest) = arguments.split_first().ok_or(ParseError::Usage)?;

    match (command.as_str(), rest) {
        ("up", rest) => {
            let options = split_options(rest);

            Ok(Sandbox::Up {
                arguments: options.arguments.unwrap_or_default(),
                headed: options.headed,
            })
        }
        ("restart", rest) => {
            let options = split_options(rest);

            Ok(Sandbox::Restart {
                arguments: options.arguments,
                force_headed: options.headed,
            })
        }
        ("capture", [option, folder, rest @ ..]) if option == "--out" => {
            if folder.is_empty() || folder.starts_with('-') {
                return Err(ParseError::Usage);
            }

            let arguments = rest
                .strip_prefix(&[END_OF_OPTIONS.to_string()])
                .unwrap_or(rest);

            Ok(Sandbox::Capture {
                out: PathBuf::from(folder),
                arguments: arguments.to_vec(),
            })
        }
        ("shot", []) => Ok(Sandbox::Shot { file: None }),
        ("shot", [file]) => Ok(Sandbox::Shot {
            file: Some(PathBuf::from(file)),
        }),
        ("move", [x0, y0, x1, y1]) => Ok(Sandbox::Move {
            from: (number(x0, "x0")?, number(y0, "y0")?),
            to: (number(x1, "x1")?, number(y1, "y1")?),
        }),
        ("drag", [x0, y0, x1, y1]) => Ok(Sandbox::Drag {
            from: (number(x0, "x0")?, number(y0, "y0")?),
            to: (number(x1, "x1")?, number(y1, "y1")?),
        }),
        ("click", [x, y]) => Ok(Sandbox::Click {
            x: number(x, "x")?,
            y: number(y, "y")?,
            button: 1,
        }),
        ("click", [x, y, button]) => Ok(Sandbox::Click {
            x: number(x, "x")?,
            y: number(y, "y")?,
            button: number(button, "button")?,
        }),
        ("type", text) if !text.is_empty() => Ok(Sandbox::Type {
            text: text.join(" "),
        }),
        ("key", [key, modifiers @ ..]) => Ok(Sandbox::Key {
            key: key.clone(),
            modifiers: modifiers.to_vec(),
        }),
        ("scroll", [x, y]) => Ok(Sandbox::Scroll {
            x: number(x, "x")?,
            y: number(y, "y")?,
            notches: 1,
        }),
        ("scroll", [x, y, notches]) => scroll(x, y, notches),
        ("hold", keys) if !keys.is_empty() => Ok(Sandbox::Hold {
            keys: keys.to_vec(),
        }),
        ("release", keys) if !keys.is_empty() => Ok(Sandbox::Release {
            keys: keys.to_vec(),
        }),
        ("status", []) => Ok(Sandbox::Status),
        ("log", []) => Ok(Sandbox::Log),
        ("opened", []) => Ok(Sandbox::Opened),
        ("down", []) => Ok(Sandbox::Down),
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
    arguments: Option<Vec<String>>,
}

fn split_options(arguments: &[String]) -> Options {
    let mut options = Options {
        headed: false,
        arguments: None,
    };

    for (index, argument) in arguments.iter().enumerate() {
        match argument.as_str() {
            HEADED => options.headed = true,
            END_OF_OPTIONS => {
                options.arguments = Some(arguments[index + 1..].to_vec());
                break;
            }
            _ => {
                options.arguments = Some(arguments[index..].to_vec());
                break;
            }
        }
    }

    options
}

#[cfg(test)]
mod tests;
