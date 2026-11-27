use std::path::{Path, PathBuf};

use anyhow::Context as _;
use thiserror::Error;

use crate::root::create_root;

use super::{StopTarget, cleanup, stop_target};

pub(super) fn run_one_shot<B>(root: &Path, name: &str, out: &Path, body: B) -> anyhow::Result<()>
where
    B: FnOnce() -> anyhow::Result<()>,
{
    refuse_capture_output_in_root(root, out)?;

    match create_root(root) {
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => anyhow::bail!(
            "The capture root already exists; run 'RUTA_NAME={name} ruta down' first."
        ),
        Ok(()) => {}
        Err(error) => return Err(error).context("Could not create the capture root"),
    }

    let result = body();

    finish_capture(root, result, stop_target, cleanup::report)
}

pub(super) fn refuse_capture_output_in_root(root: &Path, out: &Path) -> anyhow::Result<()> {
    let root = lexical_absolute_path(root)?;
    let out = lexical_absolute_path(out)?;

    if out.starts_with(&root) {
        return Err(CaptureOutputInsideRoot { out, root }.into());
    }

    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("The capture output folder {} must not be the capture root {} or inside it.", out.display(), root.display())]
pub(super) struct CaptureOutputInsideRoot {
    pub(super) out: PathBuf,
    pub(super) root: PathBuf,
}

fn lexical_absolute_path(path: &Path) -> anyhow::Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .with_context(|| format!("Could not make {} absolute", path.display()))?;
    let mut normalized = PathBuf::new();

    for component in absolute.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component),
        }
    }

    Ok(normalized)
}

pub(super) fn finish_capture<F, R>(
    root: &Path,
    captured: anyhow::Result<()>,
    stop: F,
    mut report: R,
) -> anyhow::Result<()>
where
    F: FnMut(&Path, StopTarget) -> anyhow::Result<()>,
    R: FnMut(&str) -> std::io::Result<()>,
{
    let cleaned = cleanup(root, stop, &mut report);

    if let Err(error) = captured {
        if let Err(cleanup_error) = cleaned {
            let warning = format!("Could not clean up after capture: {cleanup_error:#}");

            if let Err(report_error) = report(&warning) {
                return Err(cleanup::with_report_failure(error, Err(report_error)));
            }
        }

        return Err(error);
    }

    cleaned
}
