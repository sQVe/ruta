use std::fs;
use std::io::{Seek as _, SeekFrom};
use std::os::unix::net::UnixStream;

use super::*;
use crate::identity::ProcessIdentity;
use crate::identity::process_is_zombie;
use crate::identity::stop::stop_window;
use crate::process_stat::ProcessStat;
use crate::test_support::ChildGuard;

fn start_script(script: &str) -> (ChildGuard, ChildStdout) {
    let mut command = Command::new("sh");
    command
        .args(["-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    let mut started = ChildGuard::spawn_group(&mut command);
    let stdout = started.take_stdout();

    (started, stdout)
}

fn log_file(folder: &Path) -> (File, PathBuf) {
    let path = folder.join("app.log");
    let file = create_log(&path).expect("create the log");

    (file, path)
}

struct ReadReady {
    found: anyhow::Result<String>,
    logged: String,
    log_path: PathBuf,
}

fn read_ready(script: &str, timeout: Duration) -> ReadReady {
    read_ready_after(script, timeout, |_| {})
}

fn read_ready_after(script: &str, timeout: Duration, before_read: impl FnOnce(u32)) -> ReadReady {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let (started, stdout) = start_script(script);
    let identity = identity_of(started.id());
    before_read(started.id());
    let app = PathBuf::from("sh");

    let found = read_ready_line(
        stdout,
        log,
        &app,
        &log_path,
        identity,
        folder.path(),
        timeout,
    );
    let logged = fs::read_to_string(&log_path).expect("read the log");

    ReadReady {
        found: found.map(|(line, _)| line),
        logged,
        log_path,
    }
}

fn identity_of(pid: u32) -> ProcessIdentity {
    let stat = ProcessStat::read(pid).expect("the unreaped app process has a stat");

    ProcessIdentity {
        pid,
        started: stat.start,
    }
}

fn wait_for_zombie(pid: u32) {
    let zombie = poll_until(Duration::from_secs(3), || {
        Ok(process_is_zombie(pid).then_some(()))
    })
    .expect("the zombie check cannot fail");

    assert!(zombie.is_some(), "the app became a zombie");
}

#[test]
fn a_ready_line_app_whose_record_fails_keeps_its_output_in_the_log() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    fs::create_dir(folder.path().join("pid")).expect("block the app record");
    let (log, log_path) = log_file(folder.path());
    let mut command = Command::new("sh");
    command
        .args(["-c", "echo 'ruta-test: the setup is broken'; exit 3"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().expect("spawn the app");
    wait_for_zombie(child.id());

    let error =
        record_spawned_child(&mut child, folder.path(), &log).expect_err("the app record fails");

    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind),
        Some(std::io::ErrorKind::IsADirectory),
        "unexpected error: {error:#}"
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "ruta-test: the setup is broken\n"
    );
}

#[test]
fn a_writer_behind_the_end_of_the_app_log_keeps_the_earlier_lines() {
    let folder = crate::test_support::temporary_directory();
    let log_path = folder.path().join("app.log");
    let mut log = create_log(&log_path).expect("create the log");
    log.write_all(b"first\n").expect("write the first line");
    let mut earlier_position = log.try_clone().expect("clone the log");
    earlier_position
        .seek(SeekFrom::Start(0))
        .expect("seek to the start");

    earlier_position
        .write_all(b"second\n")
        .expect("write the second line");

    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "first\nsecond\n"
    );
}

#[test]
fn create_log_starts_the_app_log_empty() {
    let folder = crate::test_support::temporary_directory();
    let log_path = folder.path().join("app.log");
    fs::write(&log_path, "old run\n").expect("write the old log");

    let mut log = create_log(&log_path).expect("create the log");
    assert_eq!(fs::read_to_string(&log_path).expect("read the log"), "");

    log.write_all(b"new run\n").expect("write the new line");
    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "new run\n"
    );
}

#[test]
fn draining_the_pipe_stops_at_would_block_while_a_writer_stays_open() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let (reader, mut writer) = UnixStream::pair().expect("create a socket pair");
    writer.write_all(b"kept\n").expect("write to the pipe");

    let started = Instant::now();
    drain_stdout(reader, &log, DRAIN_LIMIT).expect("the pipe drains");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(1),
        "the drain took {elapsed:?} while the writer stayed open"
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "kept\n"
    );
}

#[test]
fn draining_the_pipe_stops_at_the_limit() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let (reader, mut writer) = UnixStream::pair().expect("create a socket pair");
    writer
        .write_all(b"kept\nnot kept\n")
        .expect("write to the pipe");

    drain_stdout(reader, &log, 5).expect("the pipe drains");

    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "kept\n"
    );
}

#[test]
fn copy_until_ready_writes_an_unfinished_line_to_the_log() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let (reader, mut writer) = UnixStream::pair().expect("create a socket pair");
    let app = PathBuf::from("test-app");

    let started = std::thread::spawn(move || {
        copy_until_ready(BufReader::new(reader), log, &app, READY_LINE_LIMIT)
    });
    writer
        .write_all(b"partial")
        .expect("write the start of the line");

    let partial = poll_until(Duration::from_secs(5), || {
        let logged = fs::read_to_string(&log_path).expect("read the log");

        Ok(logged.contains("partial").then_some(logged))
    })
    .expect("the log check cannot fail");

    assert_eq!(
        partial.as_deref(),
        Some("partial"),
        "the log holds the line start"
    );

    writer
        .write_all(b"\n{\"ready\":true}\n")
        .expect("write the rest of the output");

    let (line, _) = started
        .join()
        .expect("the reader finishes")
        .expect("the copy succeeds")
        .expect("a ready line");

    assert_eq!(line, "{\"ready\":true}");
    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "partial\n{\"ready\":true}\n"
    );
}

#[test]
fn copy_until_ready_fails_a_line_over_the_bound() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let app = PathBuf::from("/apps/test-app");
    let limit = 8;
    let line = vec![b'a'; usize::try_from(limit).expect("the limit fits") + 1];

    let error = copy_until_ready(BufReader::new(&line[..]), log, &app, limit)
        .expect_err("the line passes the bound");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::ReadyLineTooLong {
                app: named_app,
                limit: recorded_limit,
            }) if named_app == &app && *recorded_limit == limit
        ),
        "unexpected error: {error:#}"
    );
    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "a".repeat(line.len())
    );
}

#[test]
fn copy_until_ready_accepts_a_line_at_the_bound() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let app = PathBuf::from("/apps/test-app");
    let limit = 8;
    let input = b"12345678\n{}\n";

    let (line, _) = copy_until_ready(BufReader::new(&input[..]), log, &app, limit)
        .expect("the copy succeeds")
        .expect("a ready line");

    assert_eq!(line, "{}");
    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "12345678\n{}\n"
    );
}

#[test]
fn copy_until_ready_accepts_a_line_at_the_ready_line_bound() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let app = PathBuf::from("/apps/test-app");
    let input = format!("{}\n{{}}\n", "a".repeat(65_536));

    let (line, _) = copy_until_ready(
        BufReader::new(input.as_bytes()),
        log,
        &app,
        READY_LINE_LIMIT,
    )
    .expect("the copy succeeds")
    .expect("a ready line");

    assert_eq!(line, "{}");
    assert_eq!(fs::read_to_string(&log_path).expect("read the log"), input);
}

#[test]
fn copy_until_ready_returns_a_ready_line_at_the_end_of_the_output() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let app = PathBuf::from("/apps/test-app");

    let (line, _) = copy_until_ready(BufReader::new(&b"{}"[..]), log, &app, READY_LINE_LIMIT)
        .expect("the copy succeeds")
        .expect("a ready line");

    assert_eq!(line, "{}");
    assert_eq!(fs::read_to_string(&log_path).expect("read the log"), "{}");
}

#[test]
fn copy_until_ready_returns_no_ready_line_for_a_final_non_json_segment() {
    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let app = PathBuf::from("/apps/test-app");

    let found = copy_until_ready(BufReader::new(&b"noise"[..]), log, &app, READY_LINE_LIMIT)
        .expect("the copy succeeds");

    assert!(found.is_none(), "the final segment is not a ready line");
    assert_eq!(
        fs::read_to_string(&log_path).expect("read the log"),
        "noise"
    );
}

#[test]
fn read_ready_line_returns_the_first_json_object_and_logs_the_lines_before_it() {
    crate::test_support::in_test_namespace!();

    let ReadReady { found, logged, .. } = read_ready(
        r#"echo noise; echo '[1]'; echo '{"title":"t"}'; echo '{"title":"later"}'; exec sleep 30"#,
        Duration::from_secs(5),
    );

    assert_eq!(found.expect("a ready line"), r#"{"title":"t"}"#);
    assert!(
        logged.starts_with("noise\n[1]\n{\"title\":\"t\"}\n"),
        "logged: {logged:?}"
    );
}

#[test]
fn read_ready_line_accepts_a_ready_line_at_the_end_of_the_output() {
    crate::test_support::in_test_namespace!();

    let ReadReady { found, logged, .. } = read_ready(
        r#"printf '{"ready":true}'; exec sleep 30 >/dev/null"#,
        Duration::from_secs(5),
    );

    assert_eq!(found.expect("a ready line"), r#"{"ready":true}"#);
    assert_eq!(logged, r#"{"ready":true}"#);
}

#[test]
fn read_ready_line_reports_an_app_that_exited() {
    crate::test_support::in_test_namespace!();

    let ReadReady {
        found,
        logged,
        log_path,
    } = read_ready("echo noise", Duration::from_secs(5));

    let error = found.expect_err("the app exited");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::AppExited { log }) if log == &log_path
        ),
        "unexpected error: {error:#}"
    );
    assert_eq!(logged, "noise\n");
}

#[test]
fn read_ready_line_prefers_a_printed_ready_line_over_the_exit() {
    crate::test_support::in_test_namespace!();

    for attempt in 0..10 {
        let ReadReady { found, .. } = read_ready_after(
            r#"echo '{"ready":true}'"#,
            Duration::from_secs(5),
            wait_for_zombie,
        );

        assert_eq!(
            found.expect("the ready line wins over the exit check"),
            r#"{"ready":true}"#,
            "attempt {attempt}"
        );
    }
}

#[test]
fn read_ready_line_reports_an_exited_app_whose_output_stays_open_before_the_timeout() {
    crate::test_support::in_test_namespace!();

    let started = Instant::now();

    let ReadReady {
        found, log_path, ..
    } = read_ready_after(
        "sleep 30 & echo noise",
        Duration::from_secs(30),
        wait_for_zombie,
    );
    let elapsed = started.elapsed();

    let error = found.expect_err("the app exited");
    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::AppExited { log }) if log == &log_path
        ),
        "unexpected error: {error:#}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the exit was reported after {elapsed:?} instead of before the timeout"
    );
}

#[test]
fn read_ready_line_reports_a_timeout() {
    crate::test_support::in_test_namespace!();

    let timeout = Duration::from_millis(300);

    let ReadReady {
        found, log_path, ..
    } = read_ready("echo noise; exec sleep 30", timeout);

    let error = found.expect_err("no ready line");

    assert!(
        matches!(
            error.downcast_ref::<AppStartError>(),
            Some(AppStartError::ReadyTimeout { timeout: recorded, log })
                if *recorded == timeout && log == &log_path
        ),
        "unexpected error: {error:#}"
    );
}

#[test]
fn copy_rest_keeps_copying_output_after_the_ready_line() {
    crate::test_support::in_test_namespace!();

    let folder = crate::test_support::temporary_directory();
    let (log, log_path) = log_file(folder.path());
    let (started, stdout) =
        start_script(r#"echo '{}'; echo buffered; sleep 0.3; echo late; exec sleep 30"#);
    let pid = started.id();
    let reader_log = log.try_clone().expect("clone the log");
    let environment = vec![(
        "PATH".to_string(),
        std::env::var("PATH").unwrap_or_default(),
    )];

    let app = PathBuf::from("sh");
    let (line, rest) = read_ready_line(
        stdout,
        reader_log,
        &app,
        &log_path,
        identity_of(pid),
        folder.path(),
        Duration::from_secs(5),
    )
    .expect("a ready line");
    copy_rest(rest, log, &environment, pid).expect("the copy starts");

    let late = poll_until(Duration::from_secs(5), || {
        let logged = fs::read_to_string(&log_path).expect("read the log");

        Ok(logged.contains("late\n").then_some(logged))
    })
    .expect("the log check cannot fail");

    assert_eq!(line, "{}");
    assert_eq!(late.as_deref(), Some("{}\nbuffered\nlate\n"));
}

#[test]
fn the_app_start_errors_render_their_messages() {
    let cases = [
        (
            AppStartError::AppExited {
                log: PathBuf::from("/tmp/app.log"),
            },
            "The app exited before it was ready; see /tmp/app.log",
        ),
        (
            AppStartError::ReadyTimeout {
                timeout: Duration::from_secs(2),
                log: PathBuf::from("/tmp/app.log"),
            },
            "The app was not ready within 2 seconds; see /tmp/app.log",
        ),
        (
            AppStartError::ReadyLineTooLong {
                app: PathBuf::from("/apps/app"),
                limit: 65536,
            },
            "The app /apps/app printed a line longer than 65536 bytes before it was ready",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

mod command;
mod start;
