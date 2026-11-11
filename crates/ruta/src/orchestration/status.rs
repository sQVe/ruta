use std::path::{Path, PathBuf};

use thiserror::Error;

use super::up_display;
use crate::config::Config;
use crate::root::recorded_folder;

#[derive(Debug, Error, PartialEq, Eq)]
pub(super) enum StatusError {
    #[error("ruta {name} is not up")]
    NotUp { name: String },
    #[error("ruta {name} has no folder record; start it again with this ruta")]
    NoFolderRecord { name: String },
    #[error("ruta {name} serves {}, not {}", recorded.display(), folder.display())]
    OtherFolder {
        name: String,
        recorded: PathBuf,
        folder: PathBuf,
    },
}

// Proves only which project folder started the sandbox, and changes nothing in the root.
pub(super) fn status(
    config: &Config,
    root: &Path,
    window_exists: impl FnOnce(&Path, &str) -> anyhow::Result<bool>,
) -> anyhow::Result<String> {
    let name = &config.name;

    if up_display(root, window_exists)?.is_none() {
        return Err(StatusError::NotUp { name: name.clone() }.into());
    }

    let Some(recorded) = recorded_folder(root)? else {
        return Err(StatusError::NoFolderRecord { name: name.clone() }.into());
    };

    if recorded != config.folder {
        return Err(StatusError::OtherFolder {
            name: name.clone(),
            recorded,
            folder: config.folder.clone(),
        }
        .into());
    }

    Ok(format!(
        "ruta {name} is up and serves {}\n",
        config.folder.display()
    ))
}
