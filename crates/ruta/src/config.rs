use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thiserror::Error;
use toml::{Table, Value};

pub(crate) const FILE_NAME: &str = "ruta.toml";

const NAME_LIMIT: usize = 32;
const TOP_KEYS: &[&str] = &["name", "app", "ready", "environment"];
const APP_KEYS: &[&str] = &["command", "title", "width", "height"];
const READY_KEYS: &[&str] = &["line", "command"];
const READY_LINE: &str = "json";

// ruta sets these itself, and a caller value could point the app at the real display.
const RESERVED_KEYS: &[&str] = &[
    "DISPLAY",
    "HOME",
    "PATH",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "LIBEI_SOCKET",
    "SWAYSOCK",
];
const RESERVED_PREFIXES: &[&str] = &["WAYLAND_", "XDG_", "RUTA_"];

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Config {
    pub(crate) name: String,
    // The folder that holds `ruta.toml`. The app and the ready command run here.
    pub(crate) folder: PathBuf,
    pub(crate) app: App,
    pub(crate) ready: Ready,
    pub(crate) environment: Vec<(String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct App {
    pub(crate) command: CommandLine,
    pub(crate) title: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandLine {
    pub(crate) program: PathBuf,
    pub(crate) arguments: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ready {
    Line,
    Command(CommandLine),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum ConfigError {
    #[error("No {FILE_NAME} in {} or a folder above it.", start.display())]
    NotFound { start: PathBuf },
    #[error("{message}")]
    Syntax { message: String },
    #[error("unknown key \"{key}\"")]
    UnknownKey { key: String },
    #[error("missing key \"{key}\"")]
    MissingKey { key: String },
    #[error("\"{key}\" must be {expected}")]
    WrongType { key: String, expected: &'static str },
    #[error("set one of \"ready.line\" and \"ready.command\", not both")]
    BothReadyForms,
    #[error("set one of \"ready.line\" and \"ready.command\"")]
    NoReadyForm,
    #[error("\"{key}\" must not be empty")]
    EmptyCommand { key: String },
    #[error("\"{key}\" must be larger than 0")]
    ZeroSize { key: String },
    #[error(
        "\"{key}\" must be one name of lowercase letters, digits, and dashes, at most \
         {NAME_LIMIT} bytes, not \"{name}\""
    )]
    BadName { key: String, name: String },
    #[error("\"environment.{key}\" is set by ruta and cannot be changed")]
    ReservedEnvironment { key: String },
    #[error("\"environment.{key}\" is not a valid variable name")]
    BadEnvironmentKey { key: String },
}

// Finds `ruta.toml` in `start` or a folder above it and parses it. `name_override` comes from
// `RUTA_NAME` and replaces the configured name.
pub(crate) fn load(start: &Path, name_override: Option<&str>) -> anyhow::Result<Config> {
    let path = find(start).ok_or_else(|| ConfigError::NotFound {
        start: start.to_path_buf(),
    })?;
    let text =
        fs::read_to_string(&path).with_context(|| format!("Could not read {}", path.display()))?;
    let folder = path
        .parent()
        .expect("a found config file lives in a folder");

    parse(&text, folder, name_override).with_context(|| format!("Invalid {}", path.display()))
}

pub(crate) fn find(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|folder| folder.join(FILE_NAME))
        .find(|path| path.is_file())
}

pub(crate) fn parse(
    text: &str,
    folder: &Path,
    name_override: Option<&str>,
) -> Result<Config, ConfigError> {
    let table: Table = text
        .parse()
        .map_err(|error: toml::de::Error| ConfigError::Syntax {
            message: error.message().to_string(),
        })?;

    refuse_unknown_keys(&table, "", TOP_KEYS)?;

    let name = valid_name("name", string(required(&table, "", "name")?, "name")?)?;
    let name = match name_override {
        Some(name) => valid_name("RUTA_NAME", name)?,
        None => name,
    };
    let app = parse_app(table_at(&table, "app")?, folder)?;
    let ready = parse_ready(table_at(&table, "ready")?, folder)?;
    let environment = match table.get("environment") {
        Some(value) => parse_environment(table_value(value, "environment")?)?,
        None => Vec::new(),
    };

    Ok(Config {
        name,
        folder: folder.to_path_buf(),
        app,
        ready,
        environment,
    })
}

// Checks the rule that keeps a sandbox name one safe path and socket component.
pub(crate) fn valid_name(key: &str, name: &str) -> Result<String, ConfigError> {
    let mut characters = name.chars();
    let valid_first = characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit());
    let valid_rest = characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });

    if !valid_first || !valid_rest || name.len() > NAME_LIMIT {
        return Err(ConfigError::BadName {
            key: key.to_string(),
            name: name.to_string(),
        });
    }

    Ok(name.to_string())
}

fn parse_app(table: &Table, folder: &Path) -> Result<App, ConfigError> {
    refuse_unknown_keys(table, "app", APP_KEYS)?;

    Ok(App {
        command: command_line(required(table, "app", "command")?, "app.command", folder)?,
        title: string(required(table, "app", "title")?, "app.title")?.to_string(),
        width: size(required(table, "app", "width")?, "app.width")?,
        height: size(required(table, "app", "height")?, "app.height")?,
    })
}

fn parse_ready(table: &Table, folder: &Path) -> Result<Ready, ConfigError> {
    refuse_unknown_keys(table, "ready", READY_KEYS)?;

    match (table.get("line"), table.get("command")) {
        (Some(_), Some(_)) => Err(ConfigError::BothReadyForms),
        (None, None) => Err(ConfigError::NoReadyForm),
        (Some(line), None) => match string(line, "ready.line")? {
            READY_LINE => Ok(Ready::Line),
            _ => Err(ConfigError::WrongType {
                key: "ready.line".to_string(),
                expected: "\"json\"",
            }),
        },
        (None, Some(command)) => Ok(Ready::Command(command_line(
            command,
            "ready.command",
            folder,
        )?)),
    }
}

fn parse_environment(table: &Table) -> Result<Vec<(String, String)>, ConfigError> {
    let mut environment = Vec::new();

    for (key, value) in table {
        if key.is_empty() || key.contains(['=', '\0']) {
            return Err(ConfigError::BadEnvironmentKey { key: key.clone() });
        }

        if is_reserved(key) {
            return Err(ConfigError::ReservedEnvironment { key: key.clone() });
        }

        let value = string(value, &format!("environment.{key}"))?;
        environment.push((key.clone(), value.to_string()));
    }

    Ok(environment)
}

fn is_reserved(key: &str) -> bool {
    RESERVED_KEYS.contains(&key)
        || RESERVED_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

fn refuse_unknown_keys(table: &Table, prefix: &str, allowed: &[&str]) -> Result<(), ConfigError> {
    match table.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(ConfigError::UnknownKey {
            key: qualified(prefix, key),
        }),
        None => Ok(()),
    }
}

fn qualified(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        return key.to_string();
    }

    format!("{prefix}.{key}")
}

fn required<'a>(table: &'a Table, prefix: &str, key: &str) -> Result<&'a Value, ConfigError> {
    table.get(key).ok_or_else(|| ConfigError::MissingKey {
        key: qualified(prefix, key),
    })
}

fn table_at<'a>(table: &'a Table, key: &str) -> Result<&'a Table, ConfigError> {
    table_value(required(table, "", key)?, key)
}

fn table_value<'a>(value: &'a Value, key: &str) -> Result<&'a Table, ConfigError> {
    value.as_table().ok_or_else(|| ConfigError::WrongType {
        key: key.to_string(),
        expected: "a table",
    })
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, ConfigError> {
    value.as_str().ok_or_else(|| ConfigError::WrongType {
        key: key.to_string(),
        expected: "a string",
    })
}

fn size(value: &Value, key: &str) -> Result<u32, ConfigError> {
    let wrong_type = || ConfigError::WrongType {
        key: key.to_string(),
        expected: "a whole number of pixels",
    };
    let size = value.as_integer().ok_or_else(wrong_type)?;
    let size = u32::try_from(size).map_err(|_| wrong_type())?;

    if size == 0 {
        return Err(ConfigError::ZeroSize {
            key: key.to_string(),
        });
    }

    Ok(size)
}

fn command_line(value: &Value, key: &str, folder: &Path) -> Result<CommandLine, ConfigError> {
    let wrong_type = || ConfigError::WrongType {
        key: key.to_string(),
        expected: "a list of strings",
    };
    let items = value.as_array().ok_or_else(wrong_type)?;
    let words = items
        .iter()
        .map(|item| item.as_str().map(str::to_string).ok_or_else(wrong_type))
        .collect::<Result<Vec<_>, _>>()?;

    let Some((program, arguments)) = words.split_first() else {
        return Err(ConfigError::EmptyCommand {
            key: key.to_string(),
        });
    };

    if program.is_empty() {
        return Err(ConfigError::EmptyCommand {
            key: key.to_string(),
        });
    }

    Ok(CommandLine {
        program: resolve_program(program, folder),
        arguments: arguments.to_vec(),
    })
}

// A bare program name stays a PATH lookup; a relative path is relative to the config folder.
fn resolve_program(program: &str, folder: &Path) -> PathBuf {
    let path = Path::new(program);

    if path.is_relative() && program.contains('/') {
        return folder.join(path);
    }

    path.to_path_buf()
}

#[cfg(test)]
mod tests;
