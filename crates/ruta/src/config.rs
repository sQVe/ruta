use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thiserror::Error;
use toml::{Table, Value};

const FILE_NAME: &str = "ruta.toml";

const NAME_LIMIT: usize = 32;
const READY_LINE: &str = "json";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Config {
    pub(crate) name: String,
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

fn find(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|folder| folder.join(FILE_NAME))
        .find(|path| path.is_file())
}

fn parse(text: &str, folder: &Path, name_override: Option<&str>) -> Result<Config, ConfigError> {
    let table: Table = text
        .parse()
        .map_err(|error: toml::de::Error| ConfigError::Syntax {
            message: error.message().to_string(),
        })?;

    let section = Section {
        table: &table,
        path: String::new(),
    };
    section.refuse_unknown_keys(&["name", "app", "ready", "environment"])?;

    let name = section.read("name", |value, key| {
        let name = string(value, key)?;
        check_name(key, name)?;

        Ok(name)
    })?;
    let name = match name_override {
        Some(name) => {
            check_name("RUTA_NAME", name)?;
            name
        }
        None => name,
    }
    .to_string();
    let app = parse_app(section.section("app")?, folder)?;
    let ready = parse_ready(section.section("ready")?, folder)?;
    let environment = match section.optional_section("environment")? {
        Some(section) => parse_environment(section)?,
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

pub(crate) fn check_name(key: &str, name: &str) -> Result<(), ConfigError> {
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

    Ok(())
}

fn parse_app(section: Section<'_>, folder: &Path) -> Result<App, ConfigError> {
    section.refuse_unknown_keys(&["command", "title", "width", "height"])?;

    Ok(App {
        command: section.read("command", |value, key| command_line(value, key, folder))?,
        title: section.read("title", string)?.to_string(),
        width: section.read("width", size)?,
        height: section.read("height", size)?,
    })
}

fn parse_ready(section: Section<'_>, folder: &Path) -> Result<Ready, ConfigError> {
    section.refuse_unknown_keys(&["line", "command"])?;

    match (section.table.get("line"), section.table.get("command")) {
        (Some(_), Some(_)) => Err(ConfigError::BothReadyForms),
        (None, None) => Err(ConfigError::NoReadyForm),
        (Some(line), None) => {
            let key = section.key("line");

            match string(line, &key)? {
                READY_LINE => Ok(Ready::Line),
                _ => Err(ConfigError::WrongType {
                    key,
                    expected: "\"json\"",
                }),
            }
        }
        (None, Some(command)) => Ok(Ready::Command(command_line(
            command,
            &section.key("command"),
            folder,
        )?)),
    }
}

fn parse_environment(section: Section<'_>) -> Result<Vec<(String, String)>, ConfigError> {
    let mut environment = Vec::new();

    for (key, value) in section.table {
        if key.is_empty() || key.contains(['=', '\0']) {
            return Err(ConfigError::BadEnvironmentKey { key: key.clone() });
        }

        if crate::environment::is_reserved(key) {
            return Err(ConfigError::ReservedEnvironment { key: key.clone() });
        }

        let value = string(value, &section.key(key))?;
        environment.push((key.clone(), value.to_string()));
    }

    Ok(environment)
}

struct Section<'a> {
    table: &'a Table,
    path: String,
}

impl<'a> Section<'a> {
    fn key(&self, key: &str) -> String {
        if self.path.is_empty() {
            return key.to_string();
        }

        format!("{}.{key}", self.path)
    }

    fn refuse_unknown_keys(&self, allowed: &[&str]) -> Result<(), ConfigError> {
        match self
            .table
            .keys()
            .find(|key| !allowed.contains(&key.as_str()))
        {
            Some(key) => Err(ConfigError::UnknownKey { key: self.key(key) }),
            None => Ok(()),
        }
    }

    fn read<T>(
        &self,
        key: &str,
        reader: impl FnOnce(&'a Value, &str) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        let path = self.key(key);
        let value = self
            .table
            .get(key)
            .ok_or_else(|| ConfigError::MissingKey { key: path.clone() })?;

        reader(value, &path)
    }

    fn section(&self, key: &str) -> Result<Section<'a>, ConfigError> {
        Ok(Section {
            table: self.read(key, table_value)?,
            path: self.key(key),
        })
    }

    fn optional_section(&self, key: &str) -> Result<Option<Section<'a>>, ConfigError> {
        self.table
            .get(key)
            .map(|value| {
                let path = self.key(key);
                let table = table_value(value, &path)?;

                Ok(Section { table, path })
            })
            .transpose()
    }
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
        .map(|item| item.as_str().ok_or_else(wrong_type))
        .collect::<Result<Vec<_>, _>>()?;

    match words.split_first() {
        Some((program, arguments)) if !program.is_empty() => Ok(CommandLine {
            program: resolve_program(program, folder),
            arguments: arguments.iter().map(|word| (*word).to_string()).collect(),
        }),
        _ => Err(ConfigError::EmptyCommand {
            key: key.to_string(),
        }),
    }
}

fn resolve_program(program: &str, folder: &Path) -> PathBuf {
    let path = Path::new(program);

    if path.is_relative() && program.contains('/') {
        return folder.join(path);
    }

    path.to_path_buf()
}

#[cfg(test)]
mod tests;
