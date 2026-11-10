use std::fs;
use std::path::Path;

use anyhow::Context as _;
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Survivor {
    pub(super) file: String,
    pub(super) line: usize,
    pub(super) mutator: String,
    pub(super) original: String,
    pub(super) replacement: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Position {
    line: usize,
    column: usize,
}

pub(super) fn read_survivors(root: &Path, output: &Path) -> anyhow::Result<Vec<Survivor>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_survivors_keeps_only_missed_mutants() {
        let root = crate::test_support::temporary_directory();
        let output = root.path().join("out");
        let mutant = |summary: &str, replacement: &str| {
            json!({
                "summary": summary,
                "scenario": { "Mutant": {
                    "file": "lib.rs",
                    "genre": "BinaryOperator",
                    "replacement": replacement,
                    "span": { "start": { "line": 1, "column": 3 }, "end": { "line": 1, "column": 5 } },
                } },
            })
        };
        let outcomes = json!({ "outcomes": [
            { "scenario": "Baseline", "summary": "Success" },
            mutant("CaughtMutant", "||"),
            mutant("MissedMutant", ">"),
            mutant("Timeout", "<"),
        ] });

        fs::create_dir_all(output.join("mutants.out")).expect("create output");
        fs::write(root.path().join("lib.rs"), "a <= b\n").expect("write source");
        fs::write(
            output.join("mutants.out/outcomes.json"),
            outcomes.to_string(),
        )
        .expect("write outcomes");

        let survivors = read_survivors(root.path(), &output).expect("survivors parse");

        assert_eq!(
            survivors,
            vec![Survivor {
                file: "lib.rs".to_string(),
                line: 1,
                mutator: "BinaryOperator".to_string(),
                original: "<=".to_string(),
                replacement: ">".to_string(),
            }]
        );
    }

    #[test]
    fn span_text_reads_single_and_multiple_lines() {
        let source = "fn a() {\n    x <= y\n}\n";
        let start = Position { line: 2, column: 7 };

        assert_eq!(
            span_text(source, start, Position { line: 2, column: 9 }),
            "<="
        );
        assert_eq!(
            span_text(
                source,
                Position { line: 1, column: 8 },
                Position { line: 3, column: 2 }
            ),
            "{\n    x <= y\n}"
        );
    }
}
