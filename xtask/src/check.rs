use std::path::Path;
use std::process::{Command, ExitCode};

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Tidy,
    Build,
}

struct Step {
    name: &'static str,
    group: Group,
    program: &'static str,
    arguments: &'static [&'static str],
    environment: &'static [(&'static str, &'static str)],
}

const DENY_WARNINGS: (&str, &str) = ("CARGO_BUILD_WARNINGS", "deny");

const STEPS: &[Step] = &[
    Step {
        name: "rustfmt",
        group: Group::Tidy,
        program: "cargo",
        arguments: &["fmt", "--all", "--check"],
        environment: &[],
    },
    Step {
        name: "typos",
        group: Group::Tidy,
        program: "typos",
        arguments: &[],
        environment: &[],
    },
    Step {
        name: "taplo",
        group: Group::Tidy,
        program: "taplo",
        arguments: &["fmt", "--check"],
        environment: &[("RUST_LOG", "warn")],
    },
    Step {
        name: "cargo-deny",
        group: Group::Tidy,
        program: "cargo",
        arguments: &["deny", "--locked", "check"],
        environment: &[],
    },
    Step {
        name: "cargo-shear",
        group: Group::Tidy,
        program: "cargo",
        arguments: &["shear", "--locked", "--deny-warnings"],
        environment: &[],
    },
    Step {
        name: "clippy",
        group: Group::Build,
        program: "cargo",
        arguments: &["clippy", "--workspace", "--all-targets", "--locked"],
        environment: &[DENY_WARNINGS],
    },
    Step {
        name: "nextest",
        group: Group::Build,
        program: "cargo",
        arguments: &["nextest", "run", "--workspace", "--locked"],
        environment: &[DENY_WARNINGS],
    },
    Step {
        name: "rustdoc",
        group: Group::Build,
        program: "cargo",
        arguments: &["doc", "--workspace", "--no-deps", "--locked"],
        environment: &[DENY_WARNINGS, ("RUSTDOCFLAGS", "-D warnings")],
    },
];

#[derive(Debug, Error, PartialEq, Eq)]
#[error("Unknown check arguments: {}. Use tidy, build, or none.", arguments.join(" "))]
struct UnknownArguments {
    arguments: Vec<String>,
}

fn parse_groups(arguments: &[String]) -> Result<Vec<Group>, UnknownArguments> {
    match arguments {
        [] => Ok(vec![Group::Tidy, Group::Build]),
        [group] if group == "tidy" => Ok(vec![Group::Tidy]),
        [group] if group == "build" => Ok(vec![Group::Build]),
        _ => Err(UnknownArguments {
            arguments: arguments.to_vec(),
        }),
    }
}

#[expect(
    clippy::print_stderr,
    reason = "check progress and failures are the command's output"
)]
fn run_step(root: &Path, step: &Step) -> bool {
    eprintln!("==> {}", step.name);

    let status = Command::new(step.program)
        .args(step.arguments)
        .envs(step.environment.iter().copied())
        .current_dir(root)
        .status();

    match status {
        Ok(status) => status.success(),
        Err(error) => {
            eprintln!("Could not start {}: {error}", step.program);
            false
        }
    }
}

fn failed_steps<'a>(root: &Path, steps: &'a [Step], groups: &[Group]) -> Vec<&'a str> {
    let mut failed = Vec::new();

    for step in steps {
        if !groups.contains(&step.group) {
            continue;
        }

        if !run_step(root, step) {
            failed.push(step.name);
        }
    }

    failed
}

#[expect(
    clippy::print_stderr,
    reason = "check progress and failures are the command's output"
)]
fn check_steps(root: &Path, steps: &[Step], groups: &[Group]) -> ExitCode {
    let failed = failed_steps(root, steps, groups);

    if failed.is_empty() {
        eprintln!("All checks passed.");
        return ExitCode::SUCCESS;
    }

    eprintln!("Failed checks: {}", failed.join(", "));
    ExitCode::FAILURE
}

#[expect(
    clippy::print_stderr,
    reason = "check progress and failures are the command's output"
)]
pub(crate) fn run(root: &Path, arguments: &[String]) -> ExitCode {
    let groups = match parse_groups(arguments) {
        Ok(groups) => groups,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    check_steps(root, STEPS, &groups)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::test_support::strings;

    fn shell_step(name: &'static str, arguments: &'static [&'static str]) -> Step {
        Step {
            name,
            group: Group::Tidy,
            program: "/bin/sh",
            arguments,
            environment: &[],
        }
    }

    fn names(group: Group) -> Vec<&'static str> {
        STEPS
            .iter()
            .filter(|step| step.group == group)
            .map(|step| step.name)
            .collect()
    }

    #[test]
    fn steps_group_every_check() {
        assert_eq!(
            names(Group::Tidy),
            ["rustfmt", "typos", "taplo", "cargo-deny", "cargo-shear"]
        );
        assert_eq!(names(Group::Build), ["clippy", "nextest", "rustdoc"]);
    }

    #[test]
    fn steps_lock_cargo_and_deny_build_warnings() {
        for step in STEPS {
            if step.program == "cargo" && step.name != "rustfmt" {
                assert!(
                    step.arguments.contains(&"--locked"),
                    "{} is locked",
                    step.name
                );
            }

            if step.group == Group::Build {
                assert!(
                    step.environment.contains(&DENY_WARNINGS),
                    "{} denies warnings",
                    step.name
                );
            }

            if step.name == "rustdoc" {
                assert!(step.environment.contains(&("RUSTDOCFLAGS", "-D warnings")));
            }
        }
    }

    #[test]
    fn parse_groups_selects_groups() {
        let cases = [
            (vec![], vec![Group::Tidy, Group::Build]),
            (strings(&["tidy"]), vec![Group::Tidy]),
            (strings(&["build"]), vec![Group::Build]),
        ];

        for (arguments, expected) in cases {
            assert_eq!(parse_groups(&arguments), Ok(expected), "{arguments:?}");
        }
    }

    #[test]
    fn parse_groups_rejects_unknown_arguments() {
        for arguments in [strings(&["lint"]), strings(&["tidy", "build"])] {
            assert_eq!(
                parse_groups(&arguments),
                Err(UnknownArguments {
                    arguments: arguments.clone()
                }),
                "{arguments:?}"
            );
        }
    }

    #[test]
    fn run_fails_on_unknown_arguments() {
        let root = crate::test_support::temporary_directory();

        assert_eq!(run(root.path(), &strings(&["lint"])), ExitCode::FAILURE);
    }

    #[test]
    fn check_steps_fails_when_a_step_fails_and_runs_later_steps() {
        let root = crate::test_support::temporary_directory();
        let steps = [
            shell_step("fails", &["-c", "exit 7"]),
            shell_step("later", &["-c", "touch later"]),
        ];

        assert_eq!(
            check_steps(root.path(), &steps, &[Group::Tidy]),
            ExitCode::FAILURE
        );
        assert!(root.path().join("later").exists());
    }

    #[test]
    fn run_step_fails_when_its_program_exits_non_zero() {
        let root = crate::test_support::temporary_directory();

        assert!(run_step(
            root.path(),
            &shell_step("passes", &["-c", "exit 0"])
        ));
        assert!(!run_step(
            root.path(),
            &shell_step("fails", &["-c", "exit 7"])
        ));
    }

    #[test]
    fn run_step_fails_when_its_program_does_not_exist() {
        let root = crate::test_support::temporary_directory();
        let step = Step {
            program: "/nonexistent/ruta-missing-program",
            ..shell_step("missing", &[])
        };

        assert!(!run_step(root.path(), &step));
    }

    #[test]
    fn run_step_runs_in_the_root_with_its_environment() {
        let root = crate::test_support::temporary_directory();
        let step = Step {
            environment: &[("RUTA_CHECK_VALUE", "expected")],
            ..shell_step(
                "environment",
                &[
                    "-c",
                    r#"test -f marker && test "$RUTA_CHECK_VALUE" = expected"#,
                ],
            )
        };

        fs::write(root.path().join("marker"), "").expect("write the marker");

        assert!(run_step(root.path(), &step));
    }

    #[test]
    fn failed_steps_runs_every_selected_step_after_a_failure() {
        let root = crate::test_support::temporary_directory();
        let steps = [
            shell_step("fails", &["-c", "exit 1"]),
            shell_step("later", &["-c", "touch later"]),
            Step {
                group: Group::Build,
                ..shell_step("build", &["-c", "touch build"])
            },
        ];

        let failed = failed_steps(root.path(), &steps, &[Group::Tidy]);

        assert_eq!(failed, ["fails"]);
        assert!(root.path().join("later").exists());
        assert!(!root.path().join("build").exists());
    }
}
