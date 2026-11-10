use std::os::unix::fs::PermissionsExt as _;

use tempfile::TempDir;

use super::*;
use crate::test_support::strings;

const LISTING: &str =
    r#"[{"file":"a.rs"},{"file":"a.rs"},{"file":"b.rs"},{"file":"b.rs"},{"file":"c.rs"}]"#;
const DIFF: &str = "diff --git a/a.rs b/a.rs";

struct Fixture {
    folder: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let folder = crate::test_support::temporary_directory();
        let outcomes = folder.path().join(OUTPUT_DIRECTORY).join("mutants.out");

        for file in ["a.rs", "b.rs", "c.rs"] {
            fs::write(folder.path().join(file), "").expect("write a source file");
        }

        fs::create_dir_all(&outcomes).expect("create the output folder");
        fs::write(outcomes.join("outcomes.json"), r#"{"outcomes":[]}"#).expect("write outcomes");

        Self { folder }
    }

    fn tool(&self, name: &str, script: &str) -> String {
        let path = self.folder.path().join(name);

        fs::write(&path, format!("#!/bin/sh\n{script}\n")).expect("write a fake tool");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("make the fake tool runnable");

        path.to_str()
            .expect("the fake tool path is UTF-8")
            .to_string()
    }

    fn cargo(&self, listing: &str, list_exit: u8, run: &str) -> String {
        let script = format!(
            "printf '%s\\n' \"$*\" >> \"$(dirname \"$0\")/cargo.log\"\n\
             case \"$*\" in *--list*) printf '%s' '{listing}'; exit {list_exit};; esac\n\
             {run}"
        );

        self.tool("cargo", &script)
    }

    fn git(&self, diff: &str, exit: u8) -> String {
        self.tool("git", &format!("printf '%s' '{diff}'\nexit {exit}"))
    }

    fn cargo_calls(&self) -> Vec<String> {
        let log = fs::read_to_string(self.folder.path().join("cargo.log")).expect("read cargo.log");

        log.lines().map(str::to_string).collect()
    }

    fn report(&self, arguments: &[&str], cargo: &str, git: &str) -> (Value, u8) {
        report(self.folder.path(), &strings(arguments), cargo, git)
    }

    fn audit(&self, cargo: &str) -> (Value, u8) {
        let git = self.git("", 0);

        self.report(&["audit", "a.rs"], cargo, &git)
    }
}

fn status(report: &(Value, u8)) -> (Option<&str>, u8) {
    (report.0["status"].as_str(), report.1)
}

#[test]
fn run_fails_on_bad_arguments() {
    let fixture = Fixture::new();

    assert_eq!(
        run(fixture.folder.path(), &strings(&["other"])),
        ExitCode::FAILURE
    );
}

#[test]
fn report_keeps_request_errors_in_json_without_starting_work() {
    let root = crate::test_support::temporary_directory();
    let actual = report(
        root.path(),
        &strings(&["other", "a.rs"]),
        "/unused-cargo",
        "/unused-git",
    );
    let expected = json!({
        "version": 1, "status": "error",
        "error": "Unknown mode other. Use changed or audit.",
        "survivors": [], "unmutated": [],
    });

    assert_eq!(actual.1, 1);
    assert_eq!(actual.0, expected);
    assert_eq!(actual.0.to_string(), expected.to_string());
    assert_eq!(
        fs::read_dir(root.path())
            .expect("list the untouched root")
            .count(),
        0
    );
}

#[test]
fn report_fails_when_the_listing_fails() {
    let fixture = Fixture::new();
    let cargo = fixture.cargo("[]", 1, "exit 0");

    assert_eq!(status(&fixture.audit(&cargo)), (Some("error"), 1));
}

#[test]
fn report_fails_when_the_listing_is_not_json() {
    let fixture = Fixture::new();
    let cargo = fixture.cargo("not json", 0, "exit 0");

    assert_eq!(status(&fixture.audit(&cargo)), (Some("error"), 1));
}

#[test]
fn report_reads_the_exit_code_of_the_mutants_run() {
    let cases = [
        ("exit 0", (Some("pass"), 0)),
        ("exit 2", (Some("pass"), 0)),
        ("exit 3", (Some("pass"), 0)),
        ("exit 4", (Some("error"), 1)),
        ("exit 1", (Some("error"), 1)),
        ("kill -9 $$", (Some("error"), 1)),
    ];

    for (run, expected) in cases {
        let fixture = Fixture::new();
        let cargo = fixture.cargo(LISTING, 0, run);

        assert_eq!(status(&fixture.audit(&cargo)), expected, "{run}");
    }
}

#[test]
fn report_fails_when_git_diff_fails() {
    let fixture = Fixture::new();
    let cargo = fixture.cargo(LISTING, 0, "exit 0");
    let git = fixture.git("", 128);

    let report = fixture.report(&["changed", "main"], &cargo, &git);

    assert_eq!(status(&report), (Some("error"), 1));
}

#[test]
fn report_finds_nothing_without_work() {
    let fixture = Fixture::new();
    let cargo = fixture.cargo("[]", 0, "exit 0");
    let git = fixture.git("", 0);

    let empty_diff = fixture.report(&["changed", "main"], &cargo, &git);
    let empty_listing = fixture.audit(&cargo);

    let expected = (
        json!({ "version": 1, "status": "nothing", "survivors": [], "unmutated": [] }),
        0,
    );

    assert_eq!(empty_diff, expected);
    assert_eq!(empty_listing, expected);
}

#[test]
fn report_finds_nothing_when_the_listing_prints_nothing() {
    let fixture = Fixture::new();
    let cargo = fixture.cargo("", 0, "exit 0");
    let git = fixture.git(DIFF, 0);

    let report = fixture.report(&["changed", "main"], &cargo, &git);

    assert_eq!(status(&report), (Some("nothing"), 0));
}

#[test]
fn report_runs_the_selected_files_within_the_diff() {
    let fixture = Fixture::new();
    let cargo = fixture.cargo(LISTING, 0, "exit 0");
    let git = fixture.git(DIFF, 0);
    let diff = fixture
        .folder
        .path()
        .join(OUTPUT_DIRECTORY)
        .join("changes.diff");

    let report = fixture.report(&["changed", "main", "--max-mutants", "4"], &cargo, &git);
    let calls = fixture.cargo_calls();

    assert_eq!(report.0["unmutated"], json!(["c.rs"]));
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0].ends_with(&format!("--list --json --in-diff {}", diff.display())),
        "{}",
        calls[0]
    );
    assert!(
        calls[1].ends_with(&format!(
            "--in-diff {} --file a.rs --file b.rs",
            diff.display()
        )),
        "{}",
        calls[1]
    );
}

#[test]
fn report_runs_the_requested_audit_files() {
    let fixture = Fixture::new();
    let cargo = fixture.cargo(r#"[{"file":"a.rs"},{"file":"b.rs"}]"#, 0, "exit 0");
    let git = fixture.git("", 0);

    let report = fixture.report(&["audit", "a.rs", "./b.rs"], &cargo, &git);
    let calls = fixture.cargo_calls();

    assert_eq!(status(&report), (Some("pass"), 0));
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0].ends_with("--list --json --file a.rs --file b.rs"),
        "{}",
        calls[0]
    );
    assert!(
        calls[1].ends_with(" --file a.rs --file b.rs"),
        "{}",
        calls[1]
    );
    assert!(
        calls.iter().all(|call| !call.contains("--in-diff")),
        "{calls:?}"
    );
}

#[test]
fn report_keeps_survivor_exit_codes_for_each_requested_mode() {
    let fixture = Fixture::new();
    let root = fixture.folder.path();
    let outcomes = json!({ "outcomes": [{
        "summary": "MissedMutant",
        "scenario": { "Mutant": {
            "file": "a.rs", "genre": "BinaryOperator", "replacement": ">",
            "span": { "start": { "line": 1, "column": 6 }, "end": { "line": 1, "column": 8 } },
        } },
    }] });
    fs::write(root.join("a.rs"), "left <= right\n").expect("write source");
    fs::write(
        root.join(OUTPUT_DIRECTORY)
            .join("mutants.out/outcomes.json"),
        outcomes.to_string(),
    )
    .expect("write outcomes");
    let cargo = fixture.cargo(r#"[{"file":"a.rs"}]"#, 0, "exit 0");
    let git = fixture.git(DIFF, 0);

    for (arguments, exit) in [(["changed", "main"], 2), (["audit", "a.rs"], 0)] {
        let actual = fixture.report(&arguments, &cargo, &git);

        assert_eq!(status(&actual), (Some("survivors"), exit), "{arguments:?}");
        assert_eq!(actual.0["survivors"][0]["original"], "<=", "{arguments:?}");
    }
}

#[test]
fn report_fails_when_a_tool_cannot_start() {
    let missing = "/nonexistent/ruta-missing-program";
    let fixture = Fixture::new();
    let cargo = fixture.cargo(LISTING, 0, "exit 0");
    let git = fixture.git(DIFF, 0);

    let without_git = fixture.report(&["changed", "main"], &cargo, missing);
    let without_cargo = fixture.report(&["audit", "a.rs"], missing, &git);

    assert_eq!(status(&without_git), (Some("error"), 1));
    assert_eq!(status(&without_cargo), (Some("error"), 1));
}

#[test]
fn report_fails_when_cargo_cannot_start_the_run() {
    let fixture = Fixture::new();
    let cargo = fixture.tool(
        "cargo",
        &format!("case \"$*\" in *--list*) printf '%s' '{LISTING}'; rm \"$0\";; esac"),
    );

    assert_eq!(status(&fixture.audit(&cargo)), (Some("error"), 1));
    assert!(!Path::new(&cargo).exists());
}

#[test]
fn scope_arguments_keep_flag_and_value_order() {
    let diff = PathBuf::from("out/changes.diff");
    let cases = [
        (None, vec![], vec![]),
        (
            Some(diff.clone()),
            vec![],
            strings(&["--in-diff", "out/changes.diff"]),
        ),
        (
            None,
            strings(&["a.rs", "b.rs"]),
            strings(&["--file", "a.rs", "--file", "b.rs"]),
        ),
        (
            Some(diff),
            strings(&["a.rs", "b.rs"]),
            strings(&[
                "--in-diff",
                "out/changes.diff",
                "--file",
                "a.rs",
                "--file",
                "b.rs",
            ]),
        ),
    ];

    for (diff, files, expected) in cases {
        let scope = Scope { diff, files };

        assert_eq!(scope.arguments(), expected, "{expected:?}");
    }
}

#[test]
fn report_value_carries_the_error_message() {
    assert_eq!(
        report_value("error", Some("broken"), &[], &[]),
        json!({
            "version": 1,
            "status": "error",
            "error": "broken",
            "survivors": [],
            "unmutated": [],
        })
    );
}

#[test]
fn job_count_stays_within_the_limit() {
    let jobs = job_count();

    assert!((1..=MAX_JOBS).contains(&jobs), "{jobs}");
}

#[test]
fn apply_cap_selects_whole_files_within_the_limit() {
    let cases = [
        (
            "exact fit",
            [2, 2, 1],
            Some(4),
            strings(&["a.rs", "b.rs"]),
            strings(&["c.rs"]),
        ),
        (
            "skip a large file",
            [2, 5, 1],
            Some(3),
            strings(&["a.rs", "c.rs"]),
            strings(&["b.rs"]),
        ),
        (
            "no cap",
            [2, 5, 1],
            None,
            strings(&["a.rs", "b.rs", "c.rs"]),
            vec![],
        ),
    ];

    for (name, counts, cap, selected, unmutated) in cases {
        let counts = BTreeMap::from([
            ("a.rs".to_string(), counts[0]),
            ("b.rs".to_string(), counts[1]),
            ("c.rs".to_string(), counts[2]),
        ]);

        assert_eq!(apply_cap(&counts, cap), (selected, unmutated), "{name}");
    }
}

#[test]
fn workspace_relative_path_normalizes_existing_files() {
    let fixture = Fixture::new();
    let root = fixture.folder.path();
    let expected = "src/a.rs";
    let absolute = root.join(expected);
    fs::create_dir(root.join("src")).expect("create the source folder");
    fs::write(&absolute, "").expect("write the nested source file");

    assert_eq!(
        workspace_relative_path(root, "./src/a.rs").expect("relative"),
        expected
    );
    assert_eq!(
        workspace_relative_path(root, &absolute.display().to_string()).expect("absolute"),
        expected
    );
    assert!(matches!(
        workspace_relative_path(root, "crates/missing.rs"),
        Err(PathError::Missing { path, .. }) if path == "crates/missing.rs"
    ));
    assert!(matches!(
        workspace_relative_path(root, "/"),
        Err(PathError::Outside { path }) if path == "/"
    ));
}

#[test]
fn ran_report_sets_status_and_exit_code() {
    let found = [Survivor {
        file: "src/lib.rs".to_string(),
        line: 3,
        mutator: "BinaryOperator".to_string(),
        original: "<=".to_string(),
        replacement: ">".to_string(),
    }];
    let skipped = strings(&["b.rs"]);

    let cases = [
        (Mode::Changed, &[][..], &[][..], "pass", 0, None),
        (Mode::Changed, &found[..], &[][..], "survivors", 2, None),
        (Mode::Audit, &found[..], &[][..], "survivors", 0, None),
        (
            Mode::Audit,
            &[][..],
            &skipped[..],
            "error",
            1,
            Some("Files over the mutant cap were not mutated."),
        ),
        (
            Mode::Changed,
            &found[..],
            &skipped[..],
            "survivors",
            1,
            None,
        ),
        (Mode::Audit, &found[..], &skipped[..], "survivors", 1, None),
    ];

    for (mode, survivors, unmutated, expected_status, exit, error) in cases {
        let actual = ran_report(mode, survivors, unmutated);
        let case = format!("{mode:?} {expected_status} {unmutated:?}");
        let expected_survivors = if survivors.is_empty() {
            json!([])
        } else {
            json!([{
                "file": "src/lib.rs", "line": 3, "mutator": "BinaryOperator",
                "original": "<=", "replacement": ">", "covered": true,
            }])
        };
        let mut expected = json!({
            "version": 1, "status": expected_status,
            "survivors": expected_survivors, "unmutated": unmutated,
        });
        if let Some(error) = error {
            expected["error"] = json!(error);
        }

        assert_eq!(status(&actual), (Some(expected_status), exit), "{case}");
        assert_eq!(actual.0, expected, "{case}");
        assert_eq!(actual.0.to_string(), expected.to_string(), "{case}");
    }
}
