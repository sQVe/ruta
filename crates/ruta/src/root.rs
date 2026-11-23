use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thiserror::Error;

use crate::process_stat::boot_ticks_now;

const ROOT_PARENT: &str = "/var/tmp";
const ROOT_PREFIX: &str = "ruta-";
const SOCKET_PREFIX: &str = "ruta-";

pub(crate) mod record {
    pub(crate) const DISPLAY: &str = "display";
    pub(crate) const WESTON_PID: &str = "weston.pid";
    pub(crate) const XWAYLAND_PID: &str = "xwayland.pid";
    pub(crate) const PID: &str = "pid";
    pub(crate) const WINDOW: &str = "window";
    pub(crate) const ARGUMENTS: &str = "arguments";
    pub(crate) const MODE: &str = "mode";
    pub(crate) const SIZE: &str = "size";
    pub(crate) const OPENED: &str = "opened";
    pub(crate) const APP_LOG: &str = "app.log";
    pub(crate) const WESTON_LOG: &str = "weston.log";
    pub(crate) const READY: &str = "ready.json";
    pub(crate) const SHOT: &str = "shot.png";
    pub(crate) const CREATED: &str = "created";
    pub(crate) const FOLDER: &str = "folder";
    pub(crate) const FOLDER_STAGED: &str = "folder.staged";

    pub(crate) const ALL: [&str; 16] = [
        DISPLAY,
        WESTON_PID,
        XWAYLAND_PID,
        PID,
        WINDOW,
        ARGUMENTS,
        MODE,
        SIZE,
        OPENED,
        APP_LOG,
        WESTON_LOG,
        READY,
        SHOT,
        CREATED,
        FOLDER,
        FOLDER_STAGED,
    ];
}

pub(crate) fn root_path(name: &str) -> PathBuf {
    Path::new(ROOT_PARENT).join(format!("{ROOT_PREFIX}{name}"))
}

pub(crate) fn socket_name(name: &str) -> String {
    format!("{SOCKET_PREFIX}{name}")
}

// The private folders each sandbox root holds, which the app gets as its home and XDG folders.
pub(crate) struct PrivateFolders {
    pub(crate) home: PathBuf,
    pub(crate) config: PathBuf,
    pub(crate) state: PathBuf,
    pub(crate) data: PathBuf,
    pub(crate) cache: PathBuf,
    pub(crate) runtime: PathBuf,
    pub(crate) bin: PathBuf,
}

impl PrivateFolders {
    pub(crate) fn under(root: &Path) -> Self {
        Self {
            home: root.join("home"),
            config: root.join("config"),
            state: root.join("state"),
            data: root.join("data"),
            cache: root.join("cache"),
            runtime: root.join("run"),
            bin: root.join("bin"),
        }
    }

    fn in_creation_order(&self) -> [&Path; 7] {
        [
            &self.runtime,
            &self.config,
            &self.state,
            &self.data,
            &self.cache,
            &self.home,
            &self.bin,
        ]
    }
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
    #[error("Could not read {}", path.display())]
    UnreadablePath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("refusing the symlinked sandbox path {}", path.display())]
    LinkedPath { path: PathBuf },
    #[error("refusing to remove {}: it is not a sandbox folder", path.display())]
    NotSandboxFolder { path: PathBuf },
    #[error("refusing the symlinked opener {}", path.display())]
    LinkedOpener { path: PathBuf },
    #[error(
        "The folder record {} is not a file; remove it and run 'ruta up' again.",
        path.display()
    )]
    FolderRecordBlocked { path: PathBuf },
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
    let records = record::ALL.map(|name| root.join(name));
    let folders = PrivateFolders::under(root);

    records
        .into_iter()
        .chain(folders.in_creation_order().map(Path::to_path_buf))
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

// Fails with `AlreadyExists` when the root is there, so a caller that needs a fresh root never
// shares one. The mode is set again because the umask applies to the create.
pub(crate) fn create_root(root: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))
}

pub(crate) fn prepare_root(root: &Path) -> anyhow::Result<()> {
    refuse_unsafe_root(root)?;

    if !root.exists() {
        create_root(root).with_context(|| format!("Could not create {}", root.display()))?;
        record_root_creation(root)?;
    }

    refuse_unsafe_root(root)?;

    let folders = PrivateFolders::under(root);
    let mut missing = Vec::new();

    for path in folders.in_creation_order() {
        if private_folder_missing(path)? {
            missing.push(path);
        }
    }

    refuse_linked_paths(root)?;

    for path in missing {
        create_private_folder(path)?;
    }

    Ok(())
}

fn private_folder_missing(path: &Path) -> anyhow::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => {
            return Err(RootError::UnreadablePath {
                path: path.to_path_buf(),
                source: error,
            }
            .into());
        }
    };

    if metadata.file_type().is_symlink() {
        return Ok(false);
    }

    if !metadata.is_dir() {
        return Err(RootError::Unsafe {
            path: path.to_path_buf(),
            reason: RootReason::NotDirectory,
        }
        .into());
    }

    let described = RootMetadata {
        is_symlink: false,
        is_dir: true,
        owner: metadata.uid(),
        mode: metadata.permissions().mode(),
    };

    match root_metadata_problem(&described, current_uid()?) {
        Some(reason) => Err(RootError::Unsafe {
            path: path.to_path_buf(),
            reason,
        }
        .into()),
        None => Ok(false),
    }
}

fn create_private_folder(path: &Path) -> anyhow::Result<()> {
    fs::create_dir(path).with_context(|| format!("Could not create {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("Could not secure {}", path.display()))?;

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

pub(crate) fn write_unlinked(path: &Path, contents: impl AsRef<[u8]>) -> anyhow::Result<()> {
    write_unlinked_with(
        path,
        contents,
        RootError::LinkedPath {
            path: path.to_path_buf(),
        },
    )
}

fn write_unlinked_with(
    path: &Path,
    contents: impl AsRef<[u8]>,
    linked_error: RootError,
) -> anyhow::Result<()> {
    if is_symlink(path) {
        return Err(linked_error.into());
    }

    fs::write(path, contents).with_context(|| format!("Could not write {}", path.display()))
}

// Records when the root was made, in the clock of process start times, then waits for the clock to
// pass that tick. So every process this sandbox starts has a later start time, and a process that
// started before the root, even in the same tick, does not.
pub(crate) fn record_root_creation(root: &Path) -> anyhow::Result<()> {
    let created = boot_ticks_now();

    write_unlinked(&root.join(record::CREATED), format!("{created}\n"))?;

    while boot_ticks_now() <= created {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    Ok(())
}

pub(crate) fn root_created(root: &Path) -> Option<u64> {
    fs::read_to_string(root.join(record::CREATED))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub(crate) fn stage_folder(root: &Path, folder: &Path) -> anyhow::Result<PathBuf> {
    let path = root.join(record::FOLDER);
    let blocked = fs::symlink_metadata(&path)
        .is_ok_and(|metadata| metadata.is_dir() || metadata.file_type().is_symlink());

    if blocked {
        return Err(RootError::FolderRecordBlocked { path }.into());
    }

    let staged = root.join(record::FOLDER_STAGED);
    write_staged(&staged, folder.as_os_str().as_bytes())?;

    Ok(staged)
}

// A FIFO or another non-regular entry at the staged path must never be opened: the write would
// wait for a reader. A fresh exclusive create keeps a file that appears after the check from
// being followed either.
fn write_staged(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let blocked = fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file());

    if blocked {
        return Err(RootError::FolderRecordBlocked {
            path: path.to_path_buf(),
        }
        .into());
    }

    remove_record(path).with_context(|| format!("Could not remove {}", path.display()))?;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("Could not write {}", path.display()))?;

    file.write_all(contents)
        .with_context(|| format!("Could not write {}", path.display()))
}

// A failed publish drops the previous record, so `status` never names the previous project for
// the new app.
pub(crate) fn publish_folder(root: &Path, staged: &Path) -> anyhow::Result<()> {
    let path = root.join(record::FOLDER);

    if let Err(error) = fs::rename(staged, &path) {
        let removal = remove_record(&path).and(remove_record(staged));
        let error = anyhow::Error::new(error).context(format!(
            "Could not record the project folder in {}",
            path.display()
        ));

        return match removal {
            Ok(()) => Err(error),
            Err(removal_error) => Err(error.context(format!(
                "Could not remove the failed folder record: {removal_error}"
            ))),
        };
    }

    Ok(())
}

fn remove_record(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn recorded_folder(root: &Path) -> anyhow::Result<Option<PathBuf>> {
    let path = root.join(record::FOLDER);

    match fs::read(&path) {
        Ok(folder) => Ok(Some(PathBuf::from(OsString::from_vec(folder)))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Could not read {}", path.display())),
    }
}

pub(crate) fn install_openers(root: &Path) -> anyhow::Result<()> {
    let opened = root.join(record::OPENED);

    for opener in ["xdg-open", "open"] {
        let path = root.join("bin").join(opener);

        let log = shell_quote(&opened.to_string_lossy());
        let script = format!(
            "#!/bin/sh\nif [ -L {log} ]; then\n  echo 'refusing the symlinked opened log' >&2\n  exit 1\nfi\nprintf '%s\\n' \"$*\" >>{log}\n"
        );

        write_unlinked_with(
            &path,
            &script,
            RootError::LinkedOpener { path: path.clone() },
        )?;
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
