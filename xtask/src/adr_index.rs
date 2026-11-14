use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context as _, bail};
use thiserror::Error;

const ADR_DIRECTORY: &str = "docs/adr";
const INDEX_FILE: &str = "README.md";
const INDEX_HEADING: &str = "## Index";
const RETIRED_HEADING: &str = "## Retired";

#[derive(Debug, Error, PartialEq, Eq)]
enum Problem {
    #[error("{}: these ADRs share number {number}", adr_paths(.files))]
    SharedNumber { number: String, files: Vec<String> },
    #[error("docs/adr/{file}: the first heading must be `# ADR {number}: Title`")]
    WrongHeading { file: String, number: String },
    #[error(
        "docs/adr/{file}: number {number} is in the Retired table of docs/adr/README.md, and numbers are never reused"
    )]
    RetiredNumber { file: String, number: String },
    #[error("docs/adr/README.md: the Index does not list {file}")]
    NotListed { file: String },
    #[error("docs/adr/README.md: the Index lists {file} {count} times")]
    ListedMoreThanOnce { file: String, count: usize },
    #[error(
        "docs/adr/README.md: the Index line `{line}` is not a `###` area heading or an entry like `- [NNNN: Title](./NNNN-file.md)`"
    )]
    UnreadableEntry { line: String },
    #[error("docs/adr/README.md: the Index lists {file}, which is not a current ADR")]
    UnknownFile { file: String },
    #[error("docs/adr/README.md: the Index entry for {file} reads `{label}`, not `{expected}`")]
    WrongLabel {
        file: String,
        label: String,
        expected: String,
    },
}

struct Adr {
    file: String,
    number: String,
    title: Option<String>,
}

struct Entry {
    label: String,
    file: String,
}

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let problems = find_problems(root)?;

    if problems.is_empty() {
        return Ok(());
    }

    let lines: Vec<String> = problems.iter().map(ToString::to_string).collect();
    bail!("{}", lines.join("\n"))
}

fn find_problems(root: &Path) -> anyhow::Result<Vec<Problem>> {
    let directory = root.join(ADR_DIRECTORY);
    let index_path = directory.join(INDEX_FILE);
    let index = fs::read_to_string(&index_path)
        .with_context(|| format!("read {}", index_path.display()))?;
    let adrs = read_adrs(&directory)?;

    Ok(problems(&index, &adrs))
}

fn problems(index: &str, adrs: &[Adr]) -> Vec<Problem> {
    let mut entries = Vec::new();
    let mut problems = shared_numbers(adrs);
    let retired = retired_numbers(index);

    for line in section_lines(index, INDEX_HEADING) {
        if let Some(entry) = index_entry(line) {
            entries.push(entry);
        } else if !is_index_layout(line) {
            problems.push(Problem::UnreadableEntry {
                line: line.to_owned(),
            });
        }
    }

    for adr in adrs {
        problems.extend(adr_problems(adr, &entries, &retired));
    }

    for entry in &entries {
        if !adrs.iter().any(|adr| adr.file == entry.file) {
            problems.push(Problem::UnknownFile {
                file: entry.file.clone(),
            });
        }
    }

    problems
}

fn adr_problems(adr: &Adr, entries: &[Entry], retired: &BTreeSet<&str>) -> Vec<Problem> {
    let mut problems = Vec::new();
    let listed: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.file == adr.file)
        .collect();

    if adr.title.is_none() {
        problems.push(Problem::WrongHeading {
            file: adr.file.clone(),
            number: adr.number.clone(),
        });
    }

    if retired.contains(adr.number.as_str()) {
        problems.push(Problem::RetiredNumber {
            file: adr.file.clone(),
            number: adr.number.clone(),
        });
    }

    if listed.is_empty() {
        problems.push(Problem::NotListed {
            file: adr.file.clone(),
        });
    }

    if listed.len() > 1 {
        problems.push(Problem::ListedMoreThanOnce {
            file: adr.file.clone(),
            count: listed.len(),
        });
    }

    if let Some(title) = &adr.title {
        let expected = format!("{}: {title}", adr.number);

        for entry in listed.iter().filter(|entry| entry.label != expected) {
            problems.push(Problem::WrongLabel {
                file: adr.file.clone(),
                label: entry.label.clone(),
                expected: expected.clone(),
            });
        }
    }

    problems
}

fn shared_numbers(adrs: &[Adr]) -> Vec<Problem> {
    let mut files_by_number: BTreeMap<&str, Vec<String>> = BTreeMap::new();

    for adr in adrs {
        files_by_number
            .entry(&adr.number)
            .or_default()
            .push(adr.file.clone());
    }

    files_by_number
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(number, files)| Problem::SharedNumber {
            number: number.to_owned(),
            files,
        })
        .collect()
}

fn read_adrs(directory: &Path) -> anyhow::Result<Vec<Adr>> {
    let read_error = || format!("read {}", directory.display());
    let mut adrs = Vec::new();

    for directory_entry in fs::read_dir(directory).with_context(read_error)? {
        let path = directory_entry.with_context(read_error)?.path();
        let Some(file) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(number) = adr_number(file) else {
            continue;
        };
        let contents =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;

        adrs.push(Adr {
            file: file.to_owned(),
            number: number.to_owned(),
            title: adr_title(&contents, number),
        });
    }

    adrs.sort_by(|left, right| left.file.cmp(&right.file));
    Ok(adrs)
}

fn adr_number(file: &str) -> Option<&str> {
    let (number, rest) = file.split_once('-')?;

    (is_adr_number(number) && rest.ends_with(".md")).then_some(number)
}

fn is_adr_number(text: &str) -> bool {
    text.len() == 4 && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn adr_title(contents: &str, number: &str) -> Option<String> {
    let heading = contents
        .lines()
        .find(|line| line.trim_start().starts_with('#'))?;
    let (heading_number, title) = heading.strip_prefix("# ADR ")?.split_once(": ")?;
    let title = title.trim();

    (heading_number == number && !title.is_empty()).then(|| title.to_owned())
}

// A section runs from its level-two heading to the next one, so `###` areas stay inside it.
fn section_lines<'a>(markdown: &'a str, heading: &'a str) -> impl Iterator<Item = &'a str> {
    markdown
        .lines()
        .skip_while(move |line| line.trim_end() != heading)
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
}

fn is_index_layout(line: &str) -> bool {
    line.trim().is_empty() || line.starts_with("### ")
}

fn index_entry(line: &str) -> Option<Entry> {
    let link = line.strip_prefix("- [")?.trim_end().strip_suffix(')')?;
    let (label, target) = link.split_once("](")?;
    let file = target.strip_prefix("./").unwrap_or(target);

    Some(Entry {
        label: label.to_owned(),
        file: file.to_owned(),
    })
}

fn retired_numbers(index: &str) -> BTreeSet<&str> {
    section_lines(index, RETIRED_HEADING)
        .filter_map(|line| line.strip_prefix('|')?.split('|').next())
        .map(str::trim)
        .filter(|cell| is_adr_number(cell))
        .collect()
}

fn adr_paths(files: &[String]) -> String {
    let paths: Vec<String> = files
        .iter()
        .map(|file| format!("{ADR_DIRECTORY}/{file}"))
        .collect();

    paths.join(", ")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    const INDEX: &str = "\
# Architecture decision records

## Before writing

Follow [ADR 0001](./0001-first.md). Open the [template](./TEMPLATE.md).

## Index

### Area

- [0001: First choice](./0001-first.md)
- [0002: Second choice](./0002-second.md)

## Retired

| ADR | Title | Replaced by | Last commit |
| --- | ----- | ----------- | ----------- |
| 0003 | Old choice | 0002 | abc1234 |
";

    fn adr_tree(index: &str, adrs: &[(&str, &str)]) -> TempDir {
        let root = crate::test_support::temporary_directory();
        let directory = root.path().join("docs/adr");

        fs::create_dir_all(&directory).expect("create the ADR folder");
        fs::write(directory.join("README.md"), index).expect("write the index");
        fs::write(directory.join("TEMPLATE.md"), "# ADR NNNN: Title\n")
            .expect("write the template");

        for (file, contents) in adrs {
            fs::write(directory.join(file), contents).expect("write an ADR");
        }

        root
    }

    fn valid_adrs() -> Vec<(&'static str, &'static str)> {
        vec![
            ("0001-first.md", "# ADR 0001: First choice\n\nText.\n"),
            ("0002-second.md", "# ADR 0002: Second choice\n"),
        ]
    }

    fn problems_of(index: &str, adrs: &[(&str, &str)]) -> Vec<Problem> {
        let root = adr_tree(index, adrs);

        find_problems(root.path()).expect("read the ADR folder")
    }

    #[test]
    fn find_problems_passes_a_valid_tree() {
        assert_eq!(problems_of(INDEX, &valid_adrs()), []);
    }

    #[test]
    fn check_passes_a_valid_tree() {
        let root = adr_tree(INDEX, &valid_adrs());

        assert!(check(root.path()).is_ok());
    }

    #[test]
    fn find_problems_reports_files_that_share_a_number() {
        let index = INDEX.replace(
            "- [0002: Second choice](./0002-second.md)",
            "- [0002: Second choice](./0002-second.md)\n- [0002: Other choice](./0002-other.md)",
        );
        let mut adrs = valid_adrs();
        adrs.push(("0002-other.md", "# ADR 0002: Other choice\n"));

        assert_eq!(
            problems_of(&index, &adrs),
            [Problem::SharedNumber {
                number: "0002".to_owned(),
                files: vec!["0002-other.md".to_owned(), "0002-second.md".to_owned()],
            }]
        );
    }

    #[test]
    fn find_problems_reports_a_wrong_first_heading() {
        let cases = [
            ("number differs", "# ADR 0003: Second choice\n"),
            ("no ADR prefix", "# Second choice\n"),
            ("empty title", "# ADR 0002: \n"),
            ("second level first", "## ADR 0002: Second choice\n"),
            ("no heading", "Text only.\n"),
            (
                "indented wrong heading first",
                "   # Wrong\n\n# ADR 0002: Second choice\n",
            ),
            (
                "wrong heading first",
                "## Wrong\n\n# ADR 0002: Second choice\n",
            ),
        ];

        for (case, contents) in cases {
            let adrs = [valid_adrs()[0], ("0002-second.md", contents)];

            assert_eq!(
                problems_of(INDEX, &adrs),
                [Problem::WrongHeading {
                    file: "0002-second.md".to_owned(),
                    number: "0002".to_owned(),
                }],
                "{case}"
            );
        }
    }

    #[test]
    fn find_problems_reports_a_retired_number_in_use() {
        let index = INDEX.replace(
            "- [0002: Second choice](./0002-second.md)",
            "- [0002: Second choice](./0002-second.md)\n- [0003: Revived choice](./0003-revived.md)",
        );
        let mut adrs = valid_adrs();
        adrs.push(("0003-revived.md", "# ADR 0003: Revived choice\n"));

        assert_eq!(
            problems_of(&index, &adrs),
            [Problem::RetiredNumber {
                file: "0003-revived.md".to_owned(),
                number: "0003".to_owned(),
            }]
        );
    }

    #[test]
    fn find_problems_reports_an_adr_the_index_does_not_list() {
        let index = INDEX.replace("- [0002: Second choice](./0002-second.md)\n", "");

        assert_eq!(
            problems_of(&index, &valid_adrs()),
            [Problem::NotListed {
                file: "0002-second.md".to_owned(),
            }]
        );
    }

    #[test]
    fn find_problems_ignores_links_outside_the_index() {
        let index = INDEX.replace("- [0001: First choice](./0001-first.md)\n", "");

        assert_eq!(
            problems_of(&index, &valid_adrs()),
            [Problem::NotListed {
                file: "0001-first.md".to_owned(),
            }]
        );
    }

    #[test]
    fn find_problems_reports_an_adr_the_index_lists_twice() {
        let index = INDEX.replace(
            "## Retired",
            "### Other area\n\n- [0002: Second choice](./0002-second.md)\n\n## Retired",
        );

        assert_eq!(
            problems_of(&index, &valid_adrs()),
            [Problem::ListedMoreThanOnce {
                file: "0002-second.md".to_owned(),
                count: 2,
            }]
        );
    }

    #[test]
    fn find_problems_reports_an_entry_for_a_missing_file() {
        let index = INDEX.replace(
            "- [0002: Second choice](./0002-second.md)",
            "- [0002: Second choice](./0002-second.md)\n- [0004: Missing choice](./0004-missing.md)",
        );

        assert_eq!(
            problems_of(&index, &valid_adrs()),
            [Problem::UnknownFile {
                file: "0004-missing.md".to_owned(),
            }]
        );
    }

    #[test]
    fn find_problems_reports_a_label_that_differs_from_the_heading() {
        let cases = [
            ("other title", "- [0002: Another choice](./0002-second.md)"),
            ("other number", "- [0004: Second choice](./0002-second.md)"),
            (
                "ADR prefix",
                "- [ADR 0002: Second choice](./0002-second.md)",
            ),
        ];

        for (case, entry) in cases {
            let index = INDEX.replace("- [0002: Second choice](./0002-second.md)", entry);
            let label = entry
                .strip_prefix("- [")
                .and_then(|rest| rest.split_once(']'))
                .map(|(label, _)| label)
                .expect("the case is an index entry");

            assert_eq!(
                problems_of(&index, &valid_adrs()),
                [Problem::WrongLabel {
                    file: "0002-second.md".to_owned(),
                    label: label.to_owned(),
                    expected: "0002: Second choice".to_owned(),
                }],
                "{case}"
            );
        }
    }

    #[test]
    fn check_reports_each_problem_with_its_file() {
        let index = INDEX.replace("- [0002: Second choice](./0002-second.md)\n", "");
        let adrs = [valid_adrs()[0], ("0002-second.md", "# Second choice\n")];
        let root = adr_tree(&index, &adrs);

        let error = check(root.path()).expect_err("the tree has problems");

        assert_eq!(
            error.to_string(),
            "\
docs/adr/0002-second.md: the first heading must be `# ADR 0002: Title`
docs/adr/README.md: the Index does not list 0002-second.md"
        );
    }

    #[test]
    fn find_problems_reports_an_index_link_it_cannot_read() {
        let cases = [
            "* [0002: Second choice](./0002-second.md)",
            "+ [0004: Missing choice](./0004-missing.md)",
            "  - [0004: Missing choice](./0004-missing.md)",
            "- [0004: Missing choice](./0004-missing.md",
            "- [0004: Missing choice] (./0004-missing.md)",
            "See [0004: Missing choice](./0004-missing.md).",
        ];

        for line in cases {
            let index = INDEX.replace(
                "- [0002: Second choice](./0002-second.md)",
                &format!("- [0002: Second choice](./0002-second.md)\n{line}"),
            );

            assert_eq!(
                problems_of(&index, &valid_adrs()),
                [Problem::UnreadableEntry {
                    line: line.to_owned()
                }],
                "{line}"
            );
        }
    }

    #[test]
    fn find_problems_accepts_a_target_without_a_dot_prefix() {
        let index = INDEX.replace("](./0002-second.md)", "](0002-second.md)");

        assert_eq!(problems_of(&index, &valid_adrs()), []);
    }

    #[test]
    fn find_problems_ignores_links_after_the_index() {
        let index = format!("{INDEX}\n## Notes\n\n- [0004: Missing choice](./0004-missing.md)\n");

        assert_eq!(problems_of(&index, &valid_adrs()), []);
    }

    #[test]
    fn find_problems_ignores_files_that_are_not_adrs() {
        let mut adrs = valid_adrs();
        adrs.extend([
            ("0006-notes.txt", "Notes.\n"),
            ("00a6-letters.md", "Letters.\n"),
            ("00006-long.md", "Long.\n"),
            ("0006.md", "No title.\n"),
        ]);

        assert_eq!(problems_of(INDEX, &adrs), []);
    }

    #[test]
    fn find_problems_fails_without_an_index() {
        let root = adr_tree(INDEX, &valid_adrs());
        fs::remove_file(root.path().join("docs/adr/README.md")).expect("remove the index");

        let error = find_problems(root.path()).expect_err("the index is missing");
        let cause = error
            .downcast_ref::<std::io::Error>()
            .expect("the cause is an IO error");

        assert_eq!(cause.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            error.to_string(),
            format!("read {}", root.path().join("docs/adr/README.md").display())
        );
    }
}
