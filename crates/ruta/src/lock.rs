use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::root::{RootReason, current_uid, lock_path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LockMode {
    Exclusive,
    Shared,
}

#[derive(Debug)]
#[must_use = "dropping the guard releases the lock"]
pub(crate) struct SandboxLock {
    _file: File,
}

#[derive(Debug, Error)]
pub(crate) enum LockError {
    #[error("refusing {}: {reason}", path.display())]
    Unsafe { path: PathBuf, reason: RootReason },
    #[error(
        "ruta {name} is busy: another ruta command is using it. Run this command again after that one ends."
    )]
    Busy { name: String },
    #[error("Could not read {}", path.display())]
    UnreadablePath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Could not open {}", path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Could not secure {}", path.display())]
    Secure {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("Could not lock {}", path.display())]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub(crate) fn take(name: &str, mode: LockMode) -> Result<SandboxLock, LockError> {
    take_path(&lock_path(name), name, mode)
}

fn take_path(path: &Path, name: &str, mode: LockMode) -> Result<SandboxLock, LockError> {
    let file = open_lock_file(path)?;

    lock_file(&file, path, name, mode)?;

    Ok(SandboxLock { _file: file })
}

fn open_lock_file(path: &Path) -> Result<File, LockError> {
    let owner = current_uid();

    let existing = refuse_unsafe_path(path, owner)?;

    if existing {
        set_lock_mode(path)?;
    }

    let file = open_lock_inode(path)?;

    refuse_unsafe_file(path, &file, owner)?;

    Ok(file)
}

fn open_lock_inode(path: &Path) -> Result<File, LockError> {
    let created = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .mode(0o600)
        .open(path);

    match created {
        Ok(file) => {
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|source| LockError::Secure {
                    path: path.to_path_buf(),
                    source,
                })?;

            Ok(file)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(false)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(path)
            .map_err(|source| open_error(path, source)),
        Err(source) => Err(open_error(path, source)),
    }
}

fn set_lock_mode(path: &Path) -> Result<(), LockError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
        LockError::Secure {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn open_error(path: &Path, source: std::io::Error) -> LockError {
    if source.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) {
        return LockError::Unsafe {
            path: path.to_path_buf(),
            reason: RootReason::Symlink,
        };
    }

    LockError::Open {
        path: path.to_path_buf(),
        source,
    }
}

// The boolean is true when the path already names a regular file owned by `owner`. On a sticky
// directory no other user can replace it between that check and the caller's mode repair.
fn refuse_unsafe_path(path: &Path, owner: u32) -> Result<bool, LockError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(LockError::UnreadablePath {
                path: path.to_path_buf(),
                source,
            });
        }
    };

    let described = LockMetadata {
        is_symlink: metadata.file_type().is_symlink(),
        is_file: metadata.is_file(),
        owner: metadata.uid(),
    };

    refuse_unsafe_metadata(path, &described, owner)?;

    Ok(true)
}

fn refuse_unsafe_file(path: &Path, file: &File, owner: u32) -> Result<(), LockError> {
    let metadata = file
        .metadata()
        .map_err(|source| LockError::UnreadablePath {
            path: path.to_path_buf(),
            source,
        })?;

    let described = LockMetadata {
        is_symlink: false,
        is_file: metadata.is_file(),
        owner: metadata.uid(),
    };

    refuse_unsafe_metadata(path, &described, owner)
}

fn refuse_unsafe_metadata(
    path: &Path,
    metadata: &LockMetadata,
    owner: u32,
) -> Result<(), LockError> {
    match lock_metadata_problem(metadata, owner) {
        Some(reason) => Err(LockError::Unsafe {
            path: path.to_path_buf(),
            reason,
        }),
        None => Ok(()),
    }
}

fn lock_file(file: &File, path: &Path, name: &str, mode: LockMode) -> Result<(), LockError> {
    let locked = match mode {
        LockMode::Exclusive => file.try_lock(),
        LockMode::Shared => file.try_lock_shared(),
    };

    match locked {
        Ok(()) => Ok(()),
        Err(std::fs::TryLockError::WouldBlock) => Err(LockError::Busy {
            name: name.to_string(),
        }),
        Err(std::fs::TryLockError::Error(source)) => Err(LockError::Lock {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LockMetadata {
    is_symlink: bool,
    is_file: bool,
    owner: u32,
}

fn lock_metadata_problem(metadata: &LockMetadata, owner: u32) -> Option<RootReason> {
    if metadata.is_symlink {
        return Some(RootReason::Symlink);
    }

    if !metadata.is_file {
        return Some(RootReason::NotRegularFile);
    }

    if metadata.owner != owner {
        return Some(RootReason::WrongOwner);
    }

    None
}

#[cfg(test)]
mod tests;
