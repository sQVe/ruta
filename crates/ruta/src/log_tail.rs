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
    let mut window = Vec::new();

    file.seek(SeekFrom::Start(start.saturating_sub(1)))?;
    file.read_to_end(&mut window)?;

    // The byte before the window tells a line start from a line middle, and is absent at byte 0.
    let boundary = match window.first() {
        Some(byte) if start > 0 => Some(*byte),
        _ => None,
    };

    if let Some(boundary) = boundary {
        window.drain(..1);

        if boundary != b'\n' {
            drop_partial_first_line(&mut window);
        }
    }

    Ok(window)
}

// Without a line break the window holds no complete line, so its bytes are kept as one line.
fn drop_partial_first_line(window: &mut Vec<u8>) {
    let Some(line_break) = window.iter().position(|byte| *byte == b'\n') else {
        return;
    };

    window.drain(..=line_break);
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
        );
        let line_start_lines = vec![
            "first in window".to_string(),
            "y".repeat(TAIL_BYTES - first_in_window.len()),
        ];

        let partial_line = "\nfirst complete\n";
        let mid_line_content = format!("{}{partial_line}", "x".repeat(TAIL_BYTES * 2));
        let mid_line_lines = vec!["first complete".to_string()];

        let no_newline_content = "x".repeat(TAIL_BYTES * 2);
        let no_newline_lines = vec!["x".repeat(TAIL_BYTES)];

        let cases = [
            (
                "window starts on a line start",
                line_start_content,
                line_start_lines,
            ),
            ("window starts mid-line", mid_line_content, mid_line_lines),
            (
                "window holds no newline",
                no_newline_content,
                no_newline_lines,
            ),
        ];

        for (case, content, expected) in cases {
            fs::write(&log, content).expect("write the log");

            assert_eq!(last_lines(&log, 5), expected, "{case}");
        }
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
