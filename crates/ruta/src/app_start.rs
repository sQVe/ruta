use std::fs::{File, OpenOptions};
use std::io::{BufRead as _, BufReader, Read, Write as _};
use std::os::fd::AsFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use thiserror::Error;

use crate::cleanup;
use crate::config::{CommandLine, Config, Ready};
use crate::environment::{app_environment, caller_environment};
use crate::identity::stop::{group_alive, stop_app_group};
use crate::identity::{ProcessIdentity, RECORD_TIMEOUT, record_marked_process};
use crate::process::{POLL, ProcessError, TOOL_TIMEOUT, poll_until, run_until};
use crate::root::write_unlinked;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const EXIT_GRACE: Duration = Duration::from_millis(500);
const DRAIN_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Error)]
enum AppStartError {
    #[error("The app exited before it was ready; see {}", log.display())]
    AppExited { log: PathBuf },
    #[error(
        "The app was not ready within {} seconds; see {}",
        timeout.as_secs(),
        log.display()
    )]
    ReadyTimeout { timeout: Duration, log: PathBuf },
}

pub(crate) fn start_app(
    root: &Path,
    config: &Config,
    arguments: &[String],
    display: &str,
) -> anyhow::Result<()> {
    stop_app_group(root)?;

    let log_path = root.join(crate::root::record::APP_LOG);
    let ready_path = root.join(crate::root::record::READY);
    cleanup::first_failure([
        cleanup::remove_file(&log_path),
        cleanup::remove_file(&ready_path),
    ])?;
    let log = create_log(&log_path)?;
    let environment = app_environment(&caller_environment(), root, display, &config.environment);
    let program = &config.app.command.program;

    let mut command = crate::process::sandboxed_command(program, &environment);
    command
        .args(&config.app.command.arguments)
        .args(arguments)
        .current_dir(&config.folder)
        .stdin(Stdio::null())
        .stderr(clone_log(&log, &log_path)?)
        .process_group(0);

    match &config.ready {
        Ready::Line => start_with_ready_line(command, root, log, &environment)?,
        Ready::Command(probe) => {
            command.stdout(clone_log(&log, &log_path)?);
            let (child, leader) = spawn_recorded(command, root, &log)?;
            drop(child);

            wait_for_ready_command(
                || probe_command(probe, &environment, &config.folder),
                leader,
                root,
                &log_path,
                READY_TIMEOUT,
            )?;
        }
    }

    let recorded = serde_json::to_string(arguments).context("Could not encode the arguments")?;

    write_unlinked(
        &root.join(crate::root::record::ARGUMENTS),
        format!("{recorded}\n"),
    )
}

// The app's stderr and the `cat` that copies the app's stdout write this file from two
// processes. A shared file position lets `splice` overwrite the app's bytes, so every write
// must append. The log is emptied after opening because `OpenOptions` rejects append with
// truncate, and it must start empty even when removing the old file failed.
fn create_log(path: &Path) -> anyhow::Result<File> {
    let context = || format!("Could not create {}", path.display());
    let file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .with_context(context)?;
    file.set_len(0).with_context(context)?;

    Ok(file)
}

fn start_with_ready_line(
    mut command: Command,
    root: &Path,
    log: File,
    environment: &[(String, String)],
) -> anyhow::Result<()> {
    command.stdout(Stdio::piped());
    let (mut child, leader) = spawn_recorded(command, root, &log)?;
    let stdout = child
        .stdout
        .take()
        .expect("JSON ready mode pipes the app's stdout");
    drop(child);

    let log_path = root.join(crate::root::record::APP_LOG);
    let reader_log = clone_log(&log, &log_path)?;
    let (line, rest) = read_ready_line(stdout, reader_log, &log_path, leader, root, READY_TIMEOUT)?;

    copy_rest(rest, log, environment, leader.pid)?;
    write_unlinked(&root.join(crate::root::record::READY), format!("{line}\n"))
}

fn spawn_recorded(
    mut command: Command,
    root: &Path,
    log: &File,
) -> anyhow::Result<(Child, ProcessIdentity)> {
    let program = PathBuf::from(command.get_program());
    let mut child = command
        .spawn()
        .with_context(|| format!("Could not start {}", program.display()))?;

    let identity = record_spawned_child(&mut child, root, log)?;

    Ok((child, identity))
}

fn record_spawned_child(
    child: &mut Child,
    root: &Path,
    log: &File,
) -> anyhow::Result<ProcessIdentity> {
    let identity = match record_marked_process(
        &root.join(crate::root::record::PID),
        child,
        root,
        RECORD_TIMEOUT,
    ) {
        Ok(identity) => identity,
        Err(error) => {
            // The app may have printed its last line before it exited, so keep the output that is
            // already in the pipe. A process outside the group can still hold the pipe open.
            let stdout = child.stdout.take();
            let stopped = cleanup::stop_child_group(child);
            let copied = match stdout {
                Some(stdout) => cleanup::warn(
                    "copy the app's output to its log",
                    drain_stdout(stdout, log, DRAIN_LIMIT),
                ),
                None => Ok(()),
            };

            return Err(cleanup::with_report_failure(error, stopped.and(copied)));
        }
    };

    Ok(identity)
}

fn drain_stdout(stdout: impl Read + AsFd, log: &File, limit: u64) -> std::io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(&stdout).map_err(std::io::Error::from)?;
    rustix::fs::fcntl_setfl(&stdout, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(std::io::Error::from)?;
    let mut stdout = stdout.take(limit);
    let mut log = log.try_clone()?;
    let mut buffer = [0_u8; 4096];

    loop {
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(written) => log.write_all(&buffer[..written])?,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error),
        }
    }
}

fn clone_log(log: &File, path: &Path) -> anyhow::Result<File> {
    log.try_clone()
        .with_context(|| format!("Could not open {} for writing", path.display()))
}

fn read_ready_line(
    stdout: ChildStdout,
    log: File,
    log_path: &Path,
    leader: ProcessIdentity,
    root: &Path,
    timeout: Duration,
) -> anyhow::Result<(String, BufReader<ChildStdout>)> {
    let reader = std::thread::spawn(move || copy_until_ready(BufReader::new(stdout), log));

    let mut exited_at = None;

    let finished = poll_until(timeout, || {
        if reader.is_finished() {
            return Ok(Some(()));
        }

        if app_gone(leader, root) {
            let exited: Instant = *exited_at.get_or_insert_with(Instant::now);

            if exited.elapsed() >= EXIT_GRACE {
                return Err(app_exited_error(log_path));
            }
        }

        Ok(None)
    })?;

    if finished.is_none() {
        return Err(ready_timeout_error(timeout, log_path));
    }

    let found = reader
        .join()
        .map_err(|_| anyhow::anyhow!("The reader of the app's output stopped"))?
        .context("Could not copy the app's output to its log")?;

    found.ok_or_else(|| app_exited_error(log_path))
}

fn copy_until_ready(
    mut reader: BufReader<ChildStdout>,
    mut log: File,
) -> std::io::Result<Option<(String, BufReader<ChildStdout>)>> {
    let mut line = Vec::new();

    loop {
        line.clear();

        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(None);
        }

        log.write_all(&line)?;

        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\n', '\r']);

        if is_json_object(text) {
            return Ok(Some((text.to_string(), reader)));
        }
    }
}

fn is_json_object(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line).is_ok_and(|value| value.is_object())
}

fn copy_rest(
    reader: BufReader<ChildStdout>,
    mut log: File,
    environment: &[(String, String)],
    group: u32,
) -> anyhow::Result<()> {
    log.write_all(reader.buffer())
        .context("Could not write the app log")?;

    let errors = log.try_clone().context("Could not open the app log")?;
    let group = i32::try_from(group).context("The app's process group is out of range")?;

    crate::process::sandboxed_command(crate::tool::Tool::Cat.name(), environment)
        .stdin(Stdio::from(reader.into_inner()))
        .stdout(log)
        .stderr(errors)
        .process_group(group)
        .spawn()
        .context("Could not start cat to copy the app's output")?;

    Ok(())
}

fn probe_command(probe: &CommandLine, environment: &[(String, String)], folder: &Path) -> Command {
    let mut command = crate::process::sandboxed_command(&probe.program, environment);
    command.args(&probe.arguments).current_dir(folder);

    command
}

fn wait_for_ready_command(
    probe: impl FnMut() -> Command,
    leader: ProcessIdentity,
    root: &Path,
    log_path: &Path,
    timeout: Duration,
) -> anyhow::Result<()> {
    wait_for_ready_command_with(
        probe,
        leader,
        root,
        log_path,
        timeout,
        |command, limit, stopped| run_until(command, limit, stopped),
    )
}

fn wait_for_ready_command_with<F>(
    mut probe: impl FnMut() -> Command,
    leader: ProcessIdentity,
    root: &Path,
    log_path: &Path,
    timeout: Duration,
    mut run: F,
) -> anyhow::Result<()>
where
    F: FnMut(Command, Duration, &mut dyn FnMut() -> bool) -> anyhow::Result<Output>,
{
    let deadline = Instant::now() + timeout;

    let ready = poll_until(timeout, || {
        let limit = deadline
            .saturating_duration_since(Instant::now())
            .clamp(POLL, TOOL_TIMEOUT);
        let mut stopped = || app_gone(leader, root);

        match run(probe(), limit, &mut stopped) {
            Ok(output) if output.status.success() => return Ok(Some(())),
            Ok(_) => {}
            Err(error) if is_retryable_probe_error(&error) => {}
            Err(error) => return Err(error.context("Could not run the ready command")),
        }

        if app_gone(leader, root) {
            return Err(app_exited_error(log_path));
        }

        Ok(None)
    })?;

    ready.ok_or_else(|| ready_timeout_error(timeout, log_path))
}

// A timeout or an early stop is retried, but a failed cleanup report is a real failure that the
// loop must return instead of hiding behind another attempt.
fn is_retryable_probe_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ProcessError>().is_some()
        && error.downcast_ref::<cleanup::ReportFailure>().is_none()
}

fn app_gone(leader: ProcessIdentity, root: &Path) -> bool {
    !group_alive(leader, root)
}

fn ready_timeout_error(timeout: Duration, log_path: &Path) -> anyhow::Error {
    AppStartError::ReadyTimeout {
        timeout,
        log: log_path.to_path_buf(),
    }
    .into()
}

fn app_exited_error(log_path: &Path) -> anyhow::Error {
    AppStartError::AppExited {
        log: log_path.to_path_buf(),
    }
    .into()
}

#[cfg(test)]
mod tests;
