use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Changed,
    Audit,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) target: Target,
    pub(super) max_mutants: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Target {
    Changed { base: String },
    Audit { files: Vec<String> },
}

impl Target {
    pub(super) fn mode(&self) -> Mode {
        match self {
            Self::Changed { .. } => Mode::Changed,
            Self::Audit { .. } => Mode::Audit,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(super) enum RequestError {
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

pub(super) fn parse_request(arguments: &[String]) -> Result<Request, RequestError> {
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

    let target = match (mode_name.as_str(), targets.as_slice()) {
        ("changed", [base]) => Target::Changed { base: base.clone() },
        ("changed", _) => return Err(RequestError::ChangedTargets),
        ("audit", []) => return Err(RequestError::AuditTargets),
        ("audit", _) => Target::Audit { files: targets },
        _ => {
            return Err(RequestError::UnknownMode {
                mode: mode_name.clone(),
            });
        }
    };

    Ok(Request {
        target,
        max_mutants,
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::strings;

    #[test]
    fn parse_request_reads_modes_and_cap() {
        let changed = parse_request(&strings(&["changed", "main", "--max-mutants", "5"]))
            .expect("changed parses");
        let audit = parse_request(&strings(&["audit", "a.rs", "b.rs"])).expect("audit parses");

        assert_eq!(
            changed,
            Request {
                target: Target::Changed {
                    base: "main".to_string()
                },
                max_mutants: Some(5),
            }
        );
        assert_eq!(
            audit.target,
            Target::Audit {
                files: strings(&["a.rs", "b.rs"])
            }
        );
    }

    #[test]
    fn parse_request_rejects_bad_input() {
        let cases = [
            (vec![], RequestError::MissingMode),
            (strings(&["changed"]), RequestError::ChangedTargets),
            (
                strings(&["changed", "a", "b"]),
                RequestError::ChangedTargets,
            ),
            (strings(&["audit"]), RequestError::AuditTargets),
            (
                strings(&["audit", "a.rs", "--max-mutants"]),
                RequestError::MissingCap,
            ),
            (
                strings(&["audit", "a.rs", "--max-mutants", "many"]),
                RequestError::InvalidCap {
                    value: "many".to_string(),
                },
            ),
            (
                strings(&["audit", "a.rs", "--max-mutants", "0"]),
                RequestError::ZeroCap,
            ),
            (
                strings(&["audit", "a.rs", "--jobs"]),
                RequestError::UnknownOption {
                    option: "--jobs".to_string(),
                },
            ),
            (
                strings(&["other", "a.rs"]),
                RequestError::UnknownMode {
                    mode: "other".to_string(),
                },
            ),
        ];

        for (arguments, expected) in cases {
            assert_eq!(parse_request(&arguments), Err(expected), "{arguments:?}");
        }
    }

    #[test]
    fn options_are_checked_before_the_mode_and_target_count() {
        let cases = [
            (
                strings(&["other", "--jobs"]),
                RequestError::UnknownOption {
                    option: "--jobs".to_string(),
                },
            ),
            (
                strings(&["changed", "--max-mutants"]),
                RequestError::MissingCap,
            ),
            (
                strings(&["other", "--max-mutants", "0"]),
                RequestError::ZeroCap,
            ),
        ];

        for (arguments, expected) in cases {
            assert_eq!(parse_request(&arguments), Err(expected), "{arguments:?}");
        }
    }
}
