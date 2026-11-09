use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::Context as _;
use thiserror::Error;

use crate::environment::{caller_environment, sandbox_environment};
use crate::process::{self, poll_until};

pub(crate) mod send;

const WINDOW_TIMEOUT: Duration = Duration::from_secs(10);
const INPUT_SETTLE: Duration = Duration::from_millis(150);
const FOCUS_X: i32 = 5;
const FOCUS_Y: i32 = 5;

#[derive(Debug, Error, PartialEq, Eq)]
enum WindowError {
    #[error("The recorded window id \"{window}\" is not numeric.")]
    NotNumeric { window: String },
    #[error("Found {count} windows titled \"{title}\" at {width}x{height}; expected one.")]
    Several {
        count: usize,
        title: String,
        width: u32,
        height: u32,
    },
    #[error(
        "No window titled \"{title}\" at {width}x{height} appeared within {} seconds.",
        timeout.as_secs()
    )]
    Missing {
        title: String,
        width: u32,
        height: u32,
        timeout: Duration,
    },
}

pub(crate) fn window_id(root: &Path) -> anyhow::Result<String> {
    let window =
        fs::read_to_string(root.join("window")).with_context(|| "Could not read the window id")?;
    let window = window.trim().to_string();

    if window.is_empty() || !window.chars().all(|character| character.is_ascii_digit()) {
        return Err(WindowError::NotNumeric { window }.into());
    }

    Ok(window)
}

#[mutants::skip]
pub(crate) fn focus_window(
    root: &Path,
    display: &str,
    title: &str,
    size: (u32, u32),
) -> anyhow::Result<()> {
    let window = find_window(root, display, title, size)?;

    fs::write(root.join("window"), format!("{window}\n"))
        .with_context(|| "Could not write the window id")?;

    move_pointer(root, display, &window, FOCUS_X, FOCUS_Y)?;
    std::thread::sleep(INPUT_SETTLE);
    xdotool(root, display, &["click".to_string(), "1".to_string()])
}

#[mutants::skip]
fn find_window(
    root: &Path,
    display: &str,
    title: &str,
    (width, height): (u32, u32),
) -> anyhow::Result<String> {
    let environment = sandbox_environment(&caller_environment(), root, display);

    let window = poll_until(WINDOW_TIMEOUT, || {
        let mut matching = Vec::new();

        for id in search_windows(&environment, title)? {
            if window_size(&environment, &id)? == Some((width, height)) {
                matching.push(id);
            }
        }

        Ok(pick_window(matching, title, (width, height))?)
    })?;

    window.ok_or_else(|| {
        WindowError::Missing {
            title: title.to_string(),
            width,
            height,
            timeout: WINDOW_TIMEOUT,
        }
        .into()
    })
}

fn pick_window(
    matching: Vec<String>,
    title: &str,
    (width, height): (u32, u32),
) -> Result<Option<String>, WindowError> {
    match <[String; 1]>::try_from(matching) {
        Ok([id]) => Ok(Some(id)),
        Err(matching) if matching.is_empty() => Ok(None),
        Err(matching) => Err(WindowError::Several {
            count: matching.len(),
            title: title.to_string(),
            width,
            height,
        }),
    }
}

#[mutants::skip]
fn search_windows(environment: &[(String, String)], title: &str) -> anyhow::Result<Vec<String>> {
    let output = xdotool_output(
        environment,
        &["search", "--onlyvisible", "--name", &title_pattern(title)],
    )?;

    // xdotool exits 1 when no window matches yet.
    if !output.status.success() {
        return Ok(Vec::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

#[mutants::skip]
fn window_size(
    environment: &[(String, String)],
    window: &str,
) -> anyhow::Result<Option<(u32, u32)>> {
    let output = xdotool_output(environment, &["getwindowgeometry", "--shell", window])?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(parse_geometry(&String::from_utf8_lossy(&output.stdout)))
}

fn xdotool_output(
    environment: &[(String, String)],
    arguments: &[&str],
) -> anyhow::Result<std::process::Output> {
    xdotool_output_within(environment, arguments, process::TOOL_TIMEOUT)
}

fn xdotool_output_within(
    environment: &[(String, String)],
    arguments: &[&str],
    timeout: Duration,
) -> anyhow::Result<std::process::Output> {
    let mut command = Command::new("xdotool");
    command
        .args(arguments)
        .env_clear()
        .envs(environment.iter().cloned());

    process::run_with_timeout(command, timeout)
}

fn title_pattern(title: &str) -> String {
    let mut pattern = String::from("^");

    for character in title.chars() {
        if r"\^$.|?*+()[]{}".contains(character) {
            pattern.push('\\');
        }

        pattern.push(character);
    }

    pattern.push('$');

    pattern
}

fn parse_geometry(output: &str) -> Option<(u32, u32)> {
    let value = |key: &str| {
        output
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .and_then(|value| value.trim().parse().ok())
    };

    Some((value("WIDTH")?, value("HEIGHT")?))
}

#[mutants::skip]
fn move_pointer(root: &Path, display: &str, window: &str, x: i32, y: i32) -> anyhow::Result<()> {
    xdotool(
        root,
        display,
        &[
            "mousemove".to_string(),
            "--window".to_string(),
            window.to_string(),
            x.to_string(),
            y.to_string(),
        ],
    )
}

#[mutants::skip]
fn xdotool(root: &Path, display: &str, arguments: &[String]) -> anyhow::Result<()> {
    xdotool_within(root, display, arguments, process::TOOL_TIMEOUT)
}

#[mutants::skip]
fn xdotool_within(
    root: &Path,
    display: &str,
    arguments: &[String],
    timeout: Duration,
) -> anyhow::Result<()> {
    let environment = sandbox_environment(&caller_environment(), root, display);
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let output = xdotool_output_within(&environment, &arguments, timeout)?;

    if !output.status.success() {
        anyhow::bail!(
            "xdotool {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_id_reads_the_trimmed_record() {
        let root = crate::test_support::temporary_directory();
        fs::write(root.path().join("window"), "123\n").expect("write the window");

        assert_eq!(window_id(root.path()).expect("a numeric id"), "123");
    }

    #[test]
    fn window_id_refuses_an_id_that_is_not_a_number() {
        for (case, recorded, window) in [
            ("empty", "", ""),
            ("whitespace", " \n", ""),
            ("letters", "abc\n", "abc"),
            ("trailing letter", "12a\n", "12a"),
        ] {
            let root = crate::test_support::temporary_directory();
            fs::write(root.path().join("window"), recorded).expect("write the window");

            let error = window_id(root.path()).expect_err("the id is refused");

            assert_eq!(
                error.downcast_ref::<WindowError>(),
                Some(&WindowError::NotNumeric {
                    window: window.to_string()
                }),
                "{case}"
            );
        }
    }

    #[test]
    fn window_id_reports_a_missing_record() {
        let root = crate::test_support::temporary_directory();

        let error = window_id(root.path()).expect_err("a missing record fails");

        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::NotFound)
        );
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn pick_window_waits_for_a_window_and_takes_the_only_one() {
        assert_eq!(pick_window(Vec::new(), "greppa", (640, 480)), Ok(None));
        assert_eq!(
            pick_window(ids(&["7"]), "greppa", (640, 480)),
            Ok(Some("7".to_string()))
        );
    }

    #[test]
    fn pick_window_refuses_several_matching_windows() {
        assert_eq!(
            pick_window(ids(&["7", "8", "9"]), "greppa", (640, 480)),
            Err(WindowError::Several {
                count: 3,
                title: "greppa".to_string(),
                width: 640,
                height: 480,
            })
        );
    }

    #[test]
    fn title_pattern_matches_the_whole_title_literally() {
        assert_eq!(title_pattern("greppa"), "^greppa$");
        assert_eq!(
            title_pattern("a.b (1) [x] $5 ^ | * + ? {2} \\"),
            r"^a\.b \(1\) \[x\] \$5 \^ \| \* \+ \? \{2\} \\$"
        );
    }

    #[test]
    fn parse_geometry_reads_the_width_and_height() {
        let output = "WINDOW=123\nX=10\nY=20\nWIDTH=640\nHEIGHT=480\nSCREEN=0\n";

        assert_eq!(parse_geometry(output), Some((640, 480)));
    }

    #[test]
    fn parse_geometry_needs_both_sizes() {
        for output in [
            "WIDTH=640\n",
            "HEIGHT=480\n",
            "WIDTH=wide\nHEIGHT=480\n",
            "",
        ] {
            assert_eq!(parse_geometry(output), None, "{output:?}");
        }
    }

    #[test]
    fn the_window_errors_name_the_title_and_size() {
        let cases = [
            (
                WindowError::Several {
                    count: 2,
                    title: "greppa".to_string(),
                    width: 640,
                    height: 480,
                },
                "Found 2 windows titled \"greppa\" at 640x480; expected one.",
            ),
            (
                WindowError::Missing {
                    title: "greppa".to_string(),
                    width: 640,
                    height: 480,
                    timeout: Duration::from_secs(10),
                },
                "No window titled \"greppa\" at 640x480 appeared within 10 seconds.",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected, "{error:?}");
        }
    }
}
