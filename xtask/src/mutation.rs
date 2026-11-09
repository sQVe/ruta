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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Changed,
    Audit,
}

#[derive(Debug, PartialEq, Eq)]
struct Request {
    mode: Mode,
    targets: Vec<String>,
    max_mutants: Option<usize>,
}

struct Scope {
    diff: Option<PathBuf>,
    files: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Survivor {
    file: String,
    line: usize,
    mutator: String,
    original: String,
    replacement: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Position {
    line: usize,
    column: usize,
}

enum Outcome {
    Nothing,
    Ran {
        survivors: Vec<Survivor>,
        unmutated: Vec<String>,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
enum RequestError {
    #[error("Name a mode: changed or audit.")]
    MissingMode,
    #[error("--max-mutants needs a number.")]
    MissingCap,
    #[error("--max-mutants needs a positive integer, not {value}.")]
    InvalidCap { value: String },
    #[error("--max-mutants must be at least 1.")]
    ZeroCap,
    #[error("Unknown option {option}.")]
    UnknownOption { option: String },
    #[error("Changed mode needs exactly one base commit.")]
    ChangedTargets,
    #[error("Audit mode needs at least one file.")]
    AuditTargets,
    #[error("Unknown mode {mode}. Use changed or audit.")]
    UnknownMode { mode: String },
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

fn parse_max_mutants(value: Option<&str>) -> Result<usize, RequestError> {
    let value = value.ok_or(RequestError::MissingCap)?;

    let parsed: usize = value.parse().map_err(|_| RequestError::InvalidCap {
        value: value.to_string(),
    })?;

    if parsed == 0 {
        return Err(RequestError::ZeroCap);
    }

    Ok(parsed)
}

fn parse_request(arguments: &[String]) -> Result<Request, RequestError> {
    let (mode_name, rest) = arguments.split_first().ok_or(RequestError::MissingMode)?;

    let mut targets = Vec::new();
    let mut max_mutants = None;
    let mut remaining = rest.iter();

    while let Some(argument) = remaining.next() {
        if argument == "--max-mutants" {
            max_mutants = Some(parse_max_mutants(remaining.next().map(String::as_str))?);
        } else if argument.starts_with('-') {
            return Err(RequestError::UnknownOption {
                option: argument.clone(),
            });
        } else {
            targets.push(argument.clone());
        }
    }

    let mode = match mode_name.as_str() {
        "changed" if targets.len() == 1 => Mode::Changed,
        "changed" => return Err(RequestError::ChangedTargets),
        "audit" if !targets.is_empty() => Mode::Audit,
        "audit" => return Err(RequestError::AuditTargets),
        _ => {
            return Err(RequestError::UnknownMode {
                mode: mode_name.clone(),
            });
        }
    };

    Ok(Request {
        mode,
        targets,
        max_mutants,
    })
}

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

fn read_position(value: &Value) -> Option<Position> {
    let line = usize::try_from(value["line"].as_u64()?).ok()?;
    let column = usize::try_from(value["column"].as_u64()?).ok()?;

    Some(Position { line, column })
}

fn span_text(source: &str, start: Position, end: Position) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut text = String::new();

    for line_number in start.line..=end.line {
        let Some(line) = lines.get(line_number.saturating_sub(1)) else {
            break;
        };

        let characters: Vec<char> = line.chars().collect();
        let last = characters.len();
        let from = if line_number == start.line {
            start.column.saturating_sub(1)
        } else {
            0
        };
        let to = if line_number == end.line {
            end.column.saturating_sub(1)
        } else {
            last
        };
        let to = to.min(last);

        if line_number > start.line {
            text.push('\n');
        }

        text.extend(&characters[from.min(to)..to]);
    }

    text
}

fn read_survivor(root: &Path, mutant: &Value) -> anyhow::Result<Survivor> {
    let file = mutant["file"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("A missed mutant has no file."))?;
    let start = read_position(&mutant["span"]["start"])
        .ok_or_else(|| anyhow::anyhow!("A missed mutant has no start."))?;
    let end = read_position(&mutant["span"]["end"])
        .ok_or_else(|| anyhow::anyhow!("A missed mutant has no end."))?;

    let source =
        fs::read_to_string(root.join(file)).with_context(|| format!("Could not read {file}"))?;

    Ok(Survivor {
        file: file.to_string(),
        line: start.line,
        mutator: mutant["genre"].as_str().unwrap_or("Unknown").to_string(),
        original: span_text(&source, start, end),
        replacement: mutant["replacement"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    })
}

fn read_survivors(root: &Path, output: &Path) -> anyhow::Result<Vec<Survivor>> {
    let path = output.join("mutants.out").join("outcomes.json");

    let text =
        fs::read_to_string(&path).with_context(|| format!("Could not read {}", path.display()))?;

    let outcomes: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;

    let outcomes = outcomes["outcomes"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("outcomes.json has no outcomes list."))?;

    outcomes
        .iter()
        .filter(|outcome| outcome["summary"] == "MissedMutant")
        .map(|outcome| read_survivor(root, &outcome["scenario"]["Mutant"]))
        .collect()
}

fn mutate(root: &Path, request: &Request, cargo: &str, git: &str) -> anyhow::Result<Outcome> {
    let output = root.join(OUTPUT_DIRECTORY);

    fs::create_dir_all(&output)
        .with_context(|| format!("Could not create {}", output.display()))?;

    let scope = match request.mode {
        Mode::Changed => match write_diff(git, root, &output, &request.targets[0])? {
            Some(diff) => Scope {
                diff: Some(diff),
                files: Vec::new(),
            },
            None => return Ok(Outcome::Nothing),
        },
        Mode::Audit => Scope {
            diff: None,
            files: request
                .targets
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
        diff: scope.diff,
        files: selected,
    };

    run_mutants(cargo, root, &output, &selected_scope)?;

    let survivors = read_survivors(root, &output)?;

    Ok(Outcome::Ran {
        survivors,
        unmutated,
    })
}

fn error_report(message: &str) -> Value {
    json!({
        "version": 1,
        "status": "error",
        "error": message,
        "survivors": [],
        "unmutated": [],
    })
}

fn ran_report(mode: Mode, survivors: &[Survivor], unmutated: &[String]) -> (Value, u8) {
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
        "status": "pass",
        "survivors": survivor_values,
        "unmutated": unmutated,
    });

    if !survivors.is_empty() {
        report["status"] = json!("survivors");
    }

    if !unmutated.is_empty() {
        if survivors.is_empty() {
            report["status"] = json!("error");
            report["error"] = json!("Files over the mutant cap were not mutated.");
        }

        return (report, 1);
    }

    let exit = match (survivors.is_empty(), mode) {
        (false, Mode::Changed) => 2,
        _ => 0,
    };

    (report, exit)
}

fn report(root: &Path, arguments: &[String], cargo: &str, git: &str) -> (Value, u8) {
    let request = match parse_request(arguments) {
        Ok(request) => request,
        Err(error) => return (error_report(&format!("{error:#}")), 1),
    };

    match mutate(root, &request, cargo, git) {
        Ok(Outcome::Nothing) => (
            json!({ "version": 1, "status": "nothing", "survivors": [], "unmutated": [] }),
            0,
        ),
        Ok(Outcome::Ran {
            survivors,
            unmutated,
        }) => ran_report(request.mode, &survivors, &unmutated),
        Err(error) => (error_report(&format!("{error:#}")), 1),
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
