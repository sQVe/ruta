use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    Weston,
    Xwayland,
    Xdotool,
    Import,
    Magick,
    Cat,
}

impl Tool {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Weston => "weston",
            Self::Xwayland => "Xwayland",
            Self::Xdotool => "xdotool",
            Self::Import => "import",
            Self::Magick => "magick",
            Self::Cat => "cat",
        }
    }

    fn package(self) -> &'static str {
        match self {
            Self::Weston => "the weston package",
            Self::Xwayland => "the xorg-xwayland package",
            Self::Xdotool => "the xdotool package",
            Self::Import | Self::Magick => "the imagemagick package",
            Self::Cat => "the coreutils package",
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("The tool \"{}\" is missing. Install {} and try again.", tool.name(), tool.package())]
pub(crate) struct MissingTool {
    pub(crate) tool: Tool,
}

pub(crate) fn require_program(tool: Tool) -> Result<(), MissingTool> {
    require_program_in(tool, &std::env::var("PATH").unwrap_or_default())
}

pub(crate) fn require_program_in(tool: Tool, path: &str) -> Result<(), MissingTool> {
    if find_program(tool.name(), path).is_some() {
        return Ok(());
    }

    Err(MissingTool { tool })
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

#[cfg(test)]
mod tests;
