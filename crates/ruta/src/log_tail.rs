use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use rustix::fs::{Mode, OFlags};

const TAIL_BYTES: usize = 16 * 1024;

pub(crate) fn last_lines(path: &Path, count: usize) -> Vec<String> {
    let Ok(tail) = read_tail(path) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&tail);
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.len().saturating_sub(count);

    lines[first..].iter().map(|line| line.to_string()).collect()
}

// Opens without following a symlink and without blocking on a FIFO, and reads only a regular file.
fn read_tail(path: &Path) -> std::io::Result<Vec<u8>> {
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let mut file = File::from(rustix::fs::open(path, flags, Mode::empty())?);
    let metadata = file.metadata()?;

    if !metadata.is_file() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }

    let start = metadata.len().saturating_sub(TAIL_BYTES as u64);
    let mut bytes = Vec::new();

    file.seek(SeekFrom::Start(start.saturating_sub(3)))?;
    file.read_to_end(&mut bytes)?;

    let window_offset = usize::try_from(start.min(3)).expect("the window offset is at most 3");
    let window_offset = window_offset.min(bytes.len());
    let mut window = bytes.split_off(window_offset);
    let preceding = bytes.as_slice();

    if preceding.last().is_some_and(|byte| *byte != b'\n') {
        drop_partial_first_line(&mut window, preceding);
    }

    Ok(window)
}

fn drop_partial_first_line(window: &mut Vec<u8>, preceding: &[u8]) {
    if let Some(line_break) = window.iter().position(|byte| *byte == b'\n') {
        window.drain(..=line_break);
        return;
    }

    drop_completed_character(window, preceding);
}

fn drop_completed_character(window: &mut Vec<u8>, preceding: &[u8]) {
    let Some(lead) = preceding
        .iter()
        .rposition(|byte| !is_continuation_byte(*byte))
    else {
        return;
    };
    let begun = &preceding[lead..];

    for length in 1..=3 {
        let Some(head) = window.get(..length) else {
            return;
        };
        let joined = [begun, head].concat();

        if is_one_character(&joined) {
            if length < window.len() {
                window.drain(..length);
            }
            return;
        }
    }
}

fn is_continuation_byte(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn is_one_character(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|text| text.chars().count() == 1)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn last_lines_keeps_only_the_last_lines() {
        let folder = crate::test_support::temporary_directory();
        let log = folder.path().join("app.log");
        let text: String = (1..=30).map(|number| format!("line {number}\n")).collect();
        fs::write(&log, text).expect("write the log");

        let expected: Vec<String> = (28..=30).map(|number| format!("line {number}")).collect();

        assert_eq!(last_lines(&log, 3), expected);
    }

    #[test]
    fn last_lines_returns_the_lines_inside_the_tail_window() {
        let folder = crate::test_support::temporary_directory();
        let log = folder.path().join("app.log");

        let first_in_window = "first in window\n";
        let line_start_content = format!(
            "{}\n{first_in_window}{}",
            "x".repeat(TAIL_BYTES),
            "y".repeat(TAIL_BYTES - first_in_window.len())
        )
        .into_bytes();
        let line_start_lines = vec![
            "first in window".to_string(),
            "y".repeat(TAIL_BYTES - first_in_window.len()),
        ];

        let partial_line = "\nfirst complete\n";
        let mid_line_content = format!("{}{partial_line}", "x".repeat(TAIL_BYTES * 2)).into_bytes();
        let mid_line_lines = vec!["first complete".to_string()];

        let no_newline_content = "x".repeat(TAIL_BYTES * 2).into_bytes();
        let no_newline_lines = vec!["x".repeat(TAIL_BYTES)];

        let split_character_content =
            format!("{}é{}", "x".repeat(TAIL_BYTES), "y".repeat(TAIL_BYTES - 1)).into_bytes();
        let split_character_lines = vec!["y".repeat(TAIL_BYTES - 1)];

        let three_byte_character_content =
            format!("{}€{}", "x".repeat(TAIL_BYTES), "y".repeat(TAIL_BYTES - 1)).into_bytes();
        let three_byte_character_lines = vec!["y".repeat(TAIL_BYTES - 1)];

        let four_byte_character_content =
            format!("{}😀{}", "x".repeat(TAIL_BYTES), "y".repeat(TAIL_BYTES - 3)).into_bytes();
        let four_byte_character_lines = vec!["y".repeat(TAIL_BYTES - 3)];

        let orphan_continuation_content = [
            "x".repeat(TAIL_BYTES).into_bytes(),
            b"y\x80\x80".to_vec(),
            "y".repeat(TAIL_BYTES - 2).into_bytes(),
        ]
        .concat();
        let orphan_continuation_lines = vec![format!(
            "{}{}",
            "\u{FFFD}".repeat(2),
            "y".repeat(TAIL_BYTES - 2)
        )];

        let continuation_window_content = [
            "x".repeat(TAIL_BYTES).into_bytes(),
            b"y".to_vec(),
            vec![0x80; TAIL_BYTES],
        ]
        .concat();
        let continuation_window_lines = vec!["\u{FFFD}".repeat(TAIL_BYTES)];

        let no_lead_content = [
            "x".repeat(TAIL_BYTES - 3).into_bytes(),
            vec![0x80; TAIL_BYTES + 3],
        ]
        .concat();
        let no_lead_lines = vec!["\u{FFFD}".repeat(TAIL_BYTES)];

        let cases = [
            (
                "window starts on a line start",
                line_start_content,
                line_start_lines,
            ),
            ("window starts mid-line", mid_line_content, mid_line_lines),
            (
                "window starts inside a multi-byte character",
                split_character_content,
                split_character_lines,
            ),
            (
                "window starts at the last byte of a three-byte character",
                three_byte_character_content,
                three_byte_character_lines,
            ),
            (
                "window starts after the first byte of a four-byte character",
                four_byte_character_content,
                four_byte_character_lines,
            ),
            (
                "window holds no newline",
                no_newline_content,
                no_newline_lines,
            ),
            (
                "window starts with continuation bytes after an ASCII byte",
                orphan_continuation_content,
                orphan_continuation_lines,
            ),
            (
                "window holds only continuation bytes",
                continuation_window_content,
                continuation_window_lines,
            ),
            (
                "window starts after three continuation bytes with no lead byte",
                no_lead_content,
                no_lead_lines,
            ),
        ];

        for (case, content, expected) in cases {
            fs::write(&log, content).expect("write the log");

            assert_eq!(last_lines(&log, 5), expected, "{case}");
        }
    }

    #[test]
    fn drop_completed_character_keeps_a_window_that_is_only_the_completed_character() {
        let mut window = vec![0xA9];

        drop_completed_character(&mut window, &[0xC3]);

        assert_eq!(window, [0xA9], "the excerpt is never empty");
    }

    #[test]
    fn last_lines_keeps_the_first_line_of_a_short_log() {
        let folder = crate::test_support::temporary_directory();
        let log = folder.path().join("app.log");
        fs::write(&log, "first\nsecond\n").expect("write the log");

        assert_eq!(last_lines(&log, 5), ["first", "second"]);
    }

    #[test]
    fn last_lines_does_not_open_a_fifo_or_follow_a_symlink() {
        let folder = crate::test_support::temporary_directory();
        let fifo = folder.path().join("fifo.log");
        let target = folder.path().join("target.log");
        let link = folder.path().join("app.log");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .expect("make the fifo");
        fs::write(&target, "secret\n").expect("write the target");
        std::os::unix::fs::symlink(&target, &link).expect("link the log");

        assert_eq!(last_lines(&fifo, 5), Vec::<String>::new(), "fifo");
        assert_eq!(last_lines(&link, 5), Vec::<String>::new(), "symlink");
    }

    #[test]
    fn last_lines_is_empty_for_a_missing_or_empty_log() {
        let folder = crate::test_support::temporary_directory();
        let log = folder.path().join("app.log");

        assert_eq!(last_lines(&log, 5), Vec::<String>::new(), "missing");

        fs::write(&log, "").expect("write the log");
        assert_eq!(last_lines(&log, 5), Vec::<String>::new(), "empty");
    }
}
