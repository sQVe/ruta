use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thiserror::Error;

const ROOT_PARENT: &str = "/var/tmp";
const ROOT_PREFIX: &str = "ruta-";
const SOCKET_PREFIX: &str = "ruta-";

pub(crate) fn root_path(name: &str) -> PathBuf {
    Path::new(ROOT_PARENT).join(format!("{ROOT_PREFIX}{name}"))
}

pub(crate) fn socket_name(name: &str) -> String {
    format!("{SOCKET_PREFIX}{name}")
}

pub(crate) struct RootMetadata {
    pub(crate) is_symlink: bool,
    pub(crate) is_dir: bool,
    pub(crate) owner: u32,
    pub(crate) mode: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootReason {
    Symlink,
    NotDirectory,
    WrongOwner,
    WrongMode { mode: u32 },
}

impl std::fmt::Display for RootReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RootReason::Symlink => formatter.write_str("it is a symlink"),
            RootReason::NotDirectory => formatter.write_str("it is not a folder"),
            RootReason::WrongOwner => formatter.write_str("it is not owned by the current user"),
            RootReason::WrongMode { mode } => write!(formatter, "its mode is {mode:o}, not 700"),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum RootError {
    #[error("refusing {}: {reason}", path.display())]
    Unsafe { path: PathBuf, reason: RootReason },
    #[error("refusing the symlinked sandbox path {}", path.display())]
    LinkedPath { path: PathBuf },
    #[error("refusing to remove {}: it is not a sandbox folder", path.display())]
    NotSandboxFolder { path: PathBuf },
    #[error("refusing the symlinked opener {}", path.display())]
    LinkedOpener { path: PathBuf },
}

pub(crate) fn root_metadata_problem(
    metadata: &RootMetadata,
    current_uid: u32,
) -> Option<RootReason> {
    if metadata.is_symlink {
        return Some(RootReason::Symlink);
    }

    if !metadata.is_dir {
        return Some(RootReason::NotDirectory);
    }

    if metadata.owner != current_uid {
        return Some(RootReason::WrongOwner);
    }

    if metadata.mode & 0o777 != 0o700 {
        return Some(RootReason::WrongMode {
            mode: metadata.mode & 0o777,
        });
    }

    None
}

pub(crate) fn linked_paths(root: &Path) -> Vec<PathBuf> {
    [
        "display",
        "weston.pid",
        "xwayland.pid",
        "pid",
        "window",
        "arguments",
        "mode",
        "bin",
        "home",
        "opened",
        "app.log",
        "weston.log",
        "ready.json",
        "shot.png",
        "run",
        "config",
        "state",
        "data",
        "cache",
    ]
    .iter()
    .map(|name| root.join(name))
    .collect()
}

pub(crate) fn remove_root_allowed(root: &Path) -> bool {
    let parent_is_temporary = root.parent() == Some(Path::new(ROOT_PARENT));
    let base = root.file_name().and_then(|name| name.to_str());

    parent_is_temporary
        && base.is_some_and(|name| name.starts_with(ROOT_PREFIX) && name.len() > ROOT_PREFIX.len())
}

pub(crate) fn require_trusted_root(root: &Path) -> anyhow::Result<()> {
    refuse_unsafe_root(root)?;
    refuse_linked_paths(root)?;

    Ok(())
}

pub(crate) fn prepare_root(root: &Path) -> anyhow::Result<()> {
    refuse_unsafe_root(root)?;

    if !root.exists() {
        fs::create_dir(root).with_context(|| format!("Could not create {}", root.display()))?;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("Could not secure {}", root.display()))?;
    }

    refuse_unsafe_root(root)?;

    for folder in ["run", "config", "state", "data", "cache", "home", "bin"] {
        let path = root.join(folder);

        if !path.exists() {
            fs::create_dir(&path)
                .with_context(|| format!("Could not create {}", path.display()))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .with_context(|| format!("Could not secure {}", path.display()))?;
        }
    }

    refuse_linked_paths(root)?;

    Ok(())
}

pub(crate) fn refuse_unsafe_root(root: &Path) -> anyhow::Result<()> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("Could not read {}", root.display()));
        }
    };

    let described = RootMetadata {
        is_symlink: metadata.file_type().is_symlink(),
        is_dir: metadata.is_dir(),
        owner: metadata.uid(),
        mode: metadata.permissions().mode(),
    };

    match root_metadata_problem(&described, current_uid()?) {
        Some(reason) => Err(RootError::Unsafe {
            path: root.to_path_buf(),
            reason,
        }
        .into()),
        None => Ok(()),
    }
}

pub(crate) fn refuse_linked_paths(root: &Path) -> Result<(), RootError> {
    for path in linked_paths(root) {
        if is_symlink(&path) {
            return Err(RootError::LinkedPath { path });
        }
    }

    Ok(())
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

fn current_uid() -> anyhow::Result<u32> {
    fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .with_context(|| "Could not read the current user id")
}

pub(crate) fn remove_root(root: &Path) -> anyhow::Result<()> {
    if !remove_root_allowed(root) {
        return Err(RootError::NotSandboxFolder {
            path: root.to_path_buf(),
        }
        .into());
    }

    fs::remove_dir_all(root).with_context(|| format!("Could not remove {}", root.display()))
}

pub(crate) fn write_unlinked(path: &Path, contents: &str) -> anyhow::Result<()> {
    if is_symlink(path) {
        return Err(RootError::LinkedPath {
            path: path.to_path_buf(),
        }
        .into());
    }

    fs::write(path, contents).with_context(|| format!("Could not write {}", path.display()))
}

pub(crate) fn install_openers(root: &Path) -> anyhow::Result<()> {
    let opened = root.join("opened");

    for opener in ["xdg-open", "open"] {
        let path = root.join("bin").join(opener);

        if is_symlink(&path) {
            return Err(RootError::LinkedOpener { path }.into());
        }

        let log = shell_quote(&opened.to_string_lossy());
        let script = format!(
            "#!/bin/sh\nif [ -L {log} ]; then\n  echo 'refusing the symlinked opened log' >&2\n  exit 1\nfi\nprintf '%s\\n' \"$*\" >>{log}\n"
        );

        fs::write(&path, script).with_context(|| format!("Could not write {}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("Could not make {} runnable", path.display()))?;
    }

    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests;
