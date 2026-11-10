use std::fs::File;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use thiserror::Error;

use crate::cleanup;
use crate::config::{CommandLine, Config, Ready};
use crate::environment::{app_environment, caller_environment};
use crate::identity::{
    STOP_TIMEOUT, process_is_zombie, process_start, record_marked_process, stop_window,
};
use crate::process::{POLL, ProcessError, TOOL_TIMEOUT, poll_until, run_until};
use crate::root::write_unlinked;

const READY_TIMEOUT: Duration = Duration::from_secs(60);
const EXIT_GRACE: Duration = Duration::from_millis(500);

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
    stop_window(root)?;

    let log_path = root.join(crate::root::record::APP_LOG);
    let ready_path = root.join(crate::root::record::READY);
    cleanup::remove_file(&log_path);
    cleanup::remove_file(&ready_path);
    let log = File::create(&log_path)
        .with_context(|| format!("Could not create {}", log_path.display()))?;
    let environment = app_environment(&caller_environment(), root, display, &config.environment);
    let program = &config.app.command.program;

    let mut command = Command::new(program);
    command
        .args(&config.app.command.arguments)
        .args(arguments)
        .current_dir(&config.folder)
        .env_clear()
        .envs(environment.iter().cloned())
        .stdin(Stdio::null())
        .stderr(clone_log(&log, &log_path)?)
        .process_group(0);

    match &config.ready {
        Ready::Line => command.stdout(Stdio::piped()),
        Ready::Command(_) => command.stdout(clone_log(&log, &log_path)?),
    };

    let mut child = command
        .spawn()
        .with_context(|| format!("Could not start {}", program.display()))?;
    let pid = child.id();

    if let Err(error) = record_marked_process(
        &root.join(crate::root::record::PID),
        pid,
        root,
        STOP_TIMEOUT,
    ) {
        cleanup::stop_child_group(&mut child);
        return Err(error);
    }

    let stdout = child.stdout.take();
    drop(child);

    match &config.ready {
        Ready::Line => {
            let stdout = stdout.expect("JSON ready mode pipes the app's stdout");
            let reader_log = clone_log(&log, &log_path)?;
            let (line, rest) = read_ready_line(stdout, reader_log, &log_path, pid, READY_TIMEOUT)?;

            copy_rest(rest, log, &environment, pid)?;
            write_unlinked(&ready_path, &format!("{line}\n"))?;
        }
        Ready::Command(probe) => wait_for_ready_command(
            || probe_command(probe, &environment, &config.folder),
            pid,
            &log_path,
            READY_TIMEOUT,
        )?,
    }

    let recorded = serde_json::to_string(arguments).context("Could not encode the arguments")?;

    write_unlinked(
        &root.join(crate::root::record::ARGUMENTS),
        &format!("{recorded}\n"),
    )
}

fn clone_log(log: &File, path: &Path) -> anyhow::Result<File> {
    log.try_clone()
        .with_context(|| format!("Could not open {} for writing", path.display()))
}

fn read_ready_line(
    stdout: ChildStdout,
    log: File,
    log_path: &Path,
    pid: u32,
    timeout: Duration,
) -> anyhow::Result<(String, BufReader<ChildStdout>)> {
    let reader = std::thread::spawn(move || copy_until_ready(BufReader::new(stdout), log));

    let mut exited_at = None;

    let finished = poll_until(timeout, || {
        if reader.is_finished() {
            return Ok(Some(()));
        }

        if app_exited(pid) {
            let exited: Instant = *exited_at.get_or_insert_with(Instant::now);

            if exited.elapsed() >= EXIT_GRACE {
                return Err(app_exited_error(log_path));
            }
        }

        Ok(None)
    })?;

    if finished.is_none() {
        return Err(AppStartError::ReadyTimeout {
            timeout,
            log: log_path.to_path_buf(),
        }
        .into());
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

    Command::new(crate::tool::Tool::Cat.name())
        .env_clear()
        .envs(environment.iter().cloned())
        .stdin(Stdio::from(reader.into_inner()))
        .stdout(log)
        .stderr(errors)
        .process_group(group)
        .spawn()
        .context("Could not start cat to copy the app's output")?;

    Ok(())
}

fn probe_command(probe: &CommandLine, environment: &[(String, String)], folder: &Path) -> Command {
    let mut command = Command::new(&probe.program);
    command
        .args(&probe.arguments)
        .current_dir(folder)
        .env_clear()
        .envs(environment.iter().cloned());

    command
}

fn wait_for_ready_command(
    mut probe: impl FnMut() -> Command,
    pid: u32,
    log_path: &Path,
    timeout: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;

    let ready = poll_until(timeout, || {
        let limit = deadline
            .saturating_duration_since(Instant::now())
            .clamp(POLL, TOOL_TIMEOUT);

        match run_until(probe(), limit, || app_exited(pid)) {
            Ok(output) if output.status.success() => return Ok(Some(())),
            Ok(_) => {}
            Err(error) if error.downcast_ref::<ProcessError>().is_some() => {}
            Err(error) => return Err(error.context("Could not run the ready command")),
        }

        if app_exited(pid) {
            return Err(app_exited_error(log_path));
        }

        Ok(None)
    })?;

    ready.ok_or_else(|| {
        AppStartError::ReadyTimeout {
            timeout,
            log: log_path.to_path_buf(),
        }
        .into()
    })
}

fn app_exited(pid: u32) -> bool {
    process_start(pid).is_none() || process_is_zombie(pid)
}

fn app_exited_error(log_path: &Path) -> anyhow::Error {
    AppStartError::AppExited {
        log: log_path.to_path_buf(),
    }
    .into()
}

#[cfg(test)]
mod tests;
