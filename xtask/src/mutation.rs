//! `cargo xtask mutation`: runs cargo-mutants and prints Tau's mutation report on stdout.
//!
//! Exit codes follow Tau's contract: 0 for `pass`, `nothing`, or survivors in audit mode, 2 for
//! survivors in changed mode, and 1 for an error or any unmutated file. Everything except the
//! report goes to stderr, because Tau parses stdout as JSON.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::Context as _;
use serde_json::{Value, json};
use thiserror::Error;

mod request;
mod survivors;

use request::{Mode, Request, Target, parse_request};
use survivors::{Survivor, read_survivors};

struct Scope {
    diff: Option<PathBuf>,
    files: Vec<String>,
}

enum Outcome {
    Nothing,
    Ran {
        survivors: Vec<Survivor>,
        unmutated: Vec<String>,
    },
}

#[derive(Debug, Error)]
enum PathError {
    #[error("Could not find {path}")]
    Missing {
        path: String,
        source: std::io::Error,
    },
    #[error("Could not resolve {root}")]
    UnresolvedRoot {
        root: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is outside the workspace.")]
    Outside { path: String },
}

const MUTANTS_TEMPORARY_FOLDER: &str = "/var/tmp";
const OUTPUT_DIRECTORY: &str = "target/mutants";
const MAX_JOBS: usize = 4;

impl Scope {
    fn arguments(&self) -> Vec<String> {
        let mut arguments = Vec::new();

        if let Some(diff) = &self.diff {
            arguments.push("--in-diff".to_string());
            arguments.push(diff.display().to_string());
        }

        for file in &self.files {
            arguments.push("--file".to_string());
            arguments.push(file.clone());
        }

        arguments
    }
}

fn workspace_relative_path(root: &Path, path: &str) -> Result<String, PathError> {
    let file = root
        .join(path)
        .canonicalize()
        .map_err(|source| PathError::Missing {
            path: path.to_string(),
            source,
        })?;

    let canonical_root = root
        .canonicalize()
        .map_err(|source| PathError::UnresolvedRoot {
            root: root.to_path_buf(),
            source,
        })?;

    let relative = file
        .strip_prefix(&canonical_root)
        .map_err(|_| PathError::Outside {
            path: path.to_string(),
        })?;

    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();

    Ok(parts.join("/"))
}

fn write_diff(
    git: &str,
    root: &Path,
    output: &Path,
    base: &str,
) -> anyhow::Result<Option<PathBuf>> {
    let diff = Command::new(git)
        .args(["diff", "--no-ext-diff", base, "--"])
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .context("Could not start git diff")?;

    if !diff.status.success() {
        anyhow::bail!("git diff {base} failed with {}.", diff.status);
    }

    if diff.stdout.is_empty() {
        return Ok(None);
    }

    let path = output.join("changes.diff");

    fs::write(&path, &diff.stdout)
        .with_context(|| format!("Could not write {}", path.display()))?;

    Ok(Some(path))
}

fn count_mutants(
    cargo: &str,
    root: &Path,
    scope: &Scope,
) -> anyhow::Result<BTreeMap<String, usize>> {
    let listing = Command::new(cargo)
        .args(["mutants", "--workspace", "--list", "--json"])
        .args(scope.arguments())
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .context("Could not start cargo mutants")?;

    if !listing.status.success() {
        anyhow::bail!("cargo mutants --list failed with {}.", listing.status);
    }

    if listing.stdout.trim_ascii().is_empty() {
        return Ok(BTreeMap::new());
    }

    let mutants: Value = serde_json::from_slice(&listing.stdout)
        .with_context(|| "cargo mutants --list printed invalid JSON")?;

    let mutants = mutants
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("cargo mutants --list did not print a JSON array."))?;

    let mut counts = BTreeMap::new();

    for mutant in mutants {
        let file = mutant["file"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("A listed mutant has no file."))?;

        *counts.entry(file.to_string()).or_insert(0) += 1;
    }

    Ok(counts)
}

fn apply_cap(
    counts: &BTreeMap<String, usize>,
    max_mutants: Option<usize>,
) -> (Vec<String>, Vec<String>) {
    let mut remaining = max_mutants.unwrap_or(usize::MAX);
    let mut selected = Vec::new();
    let mut unmutated = Vec::new();

    for (file, &count) in counts {
        if count <= remaining {
            remaining -= count;
            selected.push(file.clone());
        } else {
            unmutated.push(file.clone());
        }
    }

    (selected, unmutated)
}

fn job_count() -> usize {
    let available = std::thread::available_parallelism().map_or(1, |count| count.get());

    available.min(MAX_JOBS)
}

fn run_mutants(cargo: &str, root: &Path, output: &Path, scope: &Scope) -> anyhow::Result<()> {
    let status = Command::new(cargo)
        .args(["mutants", "--workspace", "--no-shuffle", "--jobs"])
        .arg(job_count().to_string())
        .arg("--output")
        .arg(output)
        .args(scope.arguments())
        .current_dir(root)
        // cargo-mutants copies the tree into the temporary folder. ruta's signal tests hide `/tmp`
        // behind a private mount and refuse to run from a binary there.
        .env("TMPDIR", MUTANTS_TEMPORARY_FOLDER)
        .stdout(std::io::stderr())
        .status()
        .context("Could not start cargo mutants")?;

    match status.code() {
        Some(0 | 2 | 3) => Ok(()),
        Some(4) => anyhow::bail!("The unmutated tree fails to build or pass its tests."),
        _ => anyhow::bail!("cargo mutants failed with {status}."),
    }
}

fn mutate(root: &Path, request: &Request, cargo: &str, git: &str) -> anyhow::Result<Outcome> {
    let output = root.join(OUTPUT_DIRECTORY);

    fs::create_dir_all(&output)
        .with_context(|| format!("Could not create {}", output.display()))?;

    let scope = match &request.target {
        Target::Changed { base } => {
            let Some(diff) = write_diff(git, root, &output, base)? else {
                return Ok(Outcome::Nothing);
            };

            Scope {
                diff: Some(diff),
                files: Vec::new(),
            }
        }
        Target::Audit { files } => Scope {
            diff: None,
            files: files
                .iter()
                .map(|path| workspace_relative_path(root, path))
                .collect::<Result<Vec<_>, PathError>>()?,
        },
    };

    let counts = count_mutants(cargo, root, &scope)?;

    if counts.is_empty() {
        return Ok(Outcome::Nothing);
    }

    let (selected, unmutated) = apply_cap(&counts, request.max_mutants);

    if selected.is_empty() {
        return Ok(Outcome::Ran {
            survivors: Vec::new(),
            unmutated,
        });
    }

    let selected_scope = Scope {
        files: selected,
        ..scope
    };

    run_mutants(cargo, root, &output, &selected_scope)?;

    let survivors = read_survivors(root, &output)?;

    Ok(Outcome::Ran {
        survivors,
        unmutated,
    })
}

fn report_value(
    status: &str,
    error: Option<&str>,
    survivors: &[Survivor],
    unmutated: &[String],
) -> Value {
    let survivor_values: Vec<Value> = survivors
        .iter()
        .map(|survivor| {
            json!({
                "file": survivor.file,
                "line": survivor.line,
                "mutator": survivor.mutator,
                "original": survivor.original,
                "replacement": survivor.replacement,
                // cargo-mutants measures no coverage, so every missed mutant counts as covered.
                "covered": true,
            })
        })
        .collect();

    let mut report = json!({
        "version": 1,
        "status": status,
        "survivors": survivor_values,
        "unmutated": unmutated,
    });

    if let Some(error) = error {
        report["error"] = json!(error);
    }

    report
}

fn ran_report(mode: Mode, survivors: &[Survivor], unmutated: &[String]) -> (Value, u8) {
    let (status, exit, error) = match (survivors.is_empty(), unmutated.is_empty(), mode) {
        (true, true, _) => ("pass", 0, None),
        (false, true, Mode::Changed) => ("survivors", 2, None),
        (false, true, Mode::Audit) => ("survivors", 0, None),
        (true, false, _) => (
            "error",
            1,
            Some("Files over the mutant cap were not mutated."),
        ),
        (false, false, _) => ("survivors", 1, None),
    };

    (report_value(status, error, survivors, unmutated), exit)
}

fn run_request(
    root: &Path,
    arguments: &[String],
    cargo: &str,
    git: &str,
) -> anyhow::Result<(Mode, Outcome)> {
    let request = parse_request(arguments)?;

    let outcome = mutate(root, &request, cargo, git)?;

    Ok((request.target.mode(), outcome))
}

fn report(root: &Path, arguments: &[String], cargo: &str, git: &str) -> (Value, u8) {
    match run_request(root, arguments, cargo, git) {
        Ok((_, Outcome::Nothing)) => (report_value("nothing", None, &[], &[]), 0),
        Ok((
            mode,
            Outcome::Ran {
                survivors,
                unmutated,
            },
        )) => ran_report(mode, &survivors, &unmutated),
        Err(error) => (
            report_value("error", Some(&format!("{error:#}")), &[], &[]),
            1,
        ),
    }
}

#[expect(
    clippy::print_stdout,
    reason = "the mutation report is the command's stdout output"
)]
pub(crate) fn run(root: &Path, arguments: &[String]) -> ExitCode {
    let (report, exit) = report(root, arguments, "cargo", "git");

    println!("{report}");
    ExitCode::from(exit)
}

#[cfg(test)]
mod tests;
