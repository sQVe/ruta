use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use rustix::fs::{Mode, OFlags};

const TAIL_BYTES: usize = 16 * 1024;

// The last `count` lines of the log at `path`, or none when it cannot be read. Only the last
// `TAIL_BYTES` are read, so a huge log costs no more than a small one.
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

    let length = metadata.len();
    let start = length.saturating_sub(TAIL_BYTES as u64);
    let mut tail = Vec::new();

    file.seek(SeekFrom::Start(start))?;
    file.read_to_end(&mut tail)?;

    // A tail that starts inside a line drops that partial line.
    if start > 0 {
        let after_break = tail
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(tail.len(), |index| index + 1);
        tail.drain(..after_break);
    }

    Ok(tail)
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
    fn last_lines_reads_only_the_end_of_a_long_log() {
        let folder = crate::test_support::temporary_directory();
        let log = folder.path().join("app.log");
        let long_line = "x".repeat(TAIL_BYTES * 2);
        fs::write(&log, format!("{long_line}\nlast\n")).expect("write the log");

        assert_eq!(
            last_lines(&log, 5),
            ["last"],
            "the partial line at the start of the tail is dropped"
        );
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
