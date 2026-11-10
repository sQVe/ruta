use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tempfile::TempDir;

pub(crate) use crate::namespace::in_test_namespace;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
pub(crate) const WINDOW_SIZE: (u32, u32) = (640, 480);
pub(crate) const TITLE: &str = "ruta test app";

const JSON_READY: &str = r#"line = "json""#;
pub(crate) const COMMAND_READY: &str = r#"command = ["true"]"#;
const RECORDS: &[&str] = &[
    "pid",
    "window",
    "display",
    "mode",
    "arguments",
    "ready.json",
    "weston.pid",
    "xwayland.pid",
];

pub(crate) struct Project {
    folder: TempDir,
}

impl Project {
    pub(crate) fn new(config: &Config<'_>) -> Self {
        let folder = tempfile::Builder::new()
            .prefix("ruta-end-to-end-")
            .tempdir()
            .expect("create the project folder");
        let project = Self { folder };

        project.write_config(&config.text());

        project
    }

    pub(crate) fn write_config(&self, text: &str) {
        fs::write(self.folder.path().join("ruta.toml"), text).expect("write ruta.toml");
    }

    pub(crate) fn sandbox(&self, label: &str) -> Sandbox<'_> {
        Sandbox {
            name: format!("e2e-{label}-{}", std::process::id()),
            project: self,
        }
    }
}

pub(crate) struct Sandbox<'project> {
    name: String,
    project: &'project Project,
}

impl Sandbox<'_> {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn root(&self) -> PathBuf {
        PathBuf::from(format!("/var/tmp/ruta-{}", self.name))
    }

    pub(crate) fn capture_root(&self) -> PathBuf {
        PathBuf::from(format!("/var/tmp/ruta-{}-capture", self.name))
    }

    pub(crate) fn run(&self, arguments: &[&str]) -> Output {
        Self::run_command(&mut self.command(arguments))
    }

    pub(crate) fn run_command(command: &mut Command) -> Output {
        command.output().expect("run the ruta binary")
    }

    pub(crate) fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ruta"));
        command
            .args(arguments)
            .current_dir(self.project.folder.path())
            .env("RUTA_NAME", &self.name);

        command
    }

    pub(crate) fn succeed(&self, arguments: &[&str]) -> String {
        Self::succeed_command(&mut self.command(arguments))
    }

    pub(crate) fn succeed_command(command: &mut Command) -> String {
        let output = Self::run_command(command);

        assert!(
            output.status.success(),
            "{command:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout).expect("the output is UTF-8")
    }

    pub(crate) fn fail(&self, arguments: &[&str]) -> Output {
        Self::fail_command(&mut self.command(arguments))
    }

    pub(crate) fn fail_command(command: &mut Command) -> Output {
        let output = Self::run_command(command);

        assert!(
            !output.status.success(),
            "{command:?} unexpectedly succeeded: {output:?}"
        );

        output
    }

    pub(crate) fn run_within(&self, arguments: &[&str], timeout: Duration) -> Option<Output> {
        run_within(&mut self.command(arguments), timeout)
    }

    pub(crate) fn records(&self) -> Vec<(&'static str, Option<String>)> {
        let root = self.root();

        RECORDS
            .iter()
            .map(|name| (*name, fs::read_to_string(root.join(name)).ok()))
            .collect()
    }

    pub(crate) fn recorded_pid(&self, record: &str) -> u32 {
        fs::read_to_string(self.root().join(record))
            .expect("read the record")
            .split_whitespace()
            .next()
            .and_then(|pid| pid.parse().ok())
            .expect("a recorded PID")
    }

    pub(crate) fn wait_for_log(&self, expected: &str) -> String {
        let mut log = String::new();

        poll_until(Duration::from_secs(5), Duration::from_millis(100), || {
            log = self.succeed(&["log"]);
            log.contains(expected)
        });

        log
    }

    pub(crate) fn shot_size(&self) -> (u32, u32) {
        let shot = self.root().join("end-to-end.png");
        let shot_argument = shot.to_str().expect("the shot path is UTF-8");

        self.succeed(&["shot", shot_argument]);

        png_size(&shot)
    }

    pub(crate) fn assert_gone(&self) {
        assert_gone(&self.root());
    }

    // `down` deletes the root, so its logs and shots are copied out first for CI to upload.
    fn keep_diagnostics(&self, folder: &Path) -> std::io::Result<()> {
        let target = folder.join(&self.name);

        fs::create_dir_all(&target)?;

        for entry in fs::read_dir(self.root())? {
            let path = entry?.path();
            let is_diagnostic = path
                .extension()
                .is_some_and(|extension| extension == "log" || extension == "png");

            if is_diagnostic && let Some(file) = path.file_name() {
                fs::copy(&path, target.join(file))?;
            }
        }

        Ok(())
    }
}

impl Drop for Sandbox<'_> {
    #[expect(
        clippy::print_stderr,
        reason = "no caller can take a cleanup error in drop"
    )]
    fn drop(&mut self) {
        if let Some(folder) = std::env::var_os("RUTA_E2E_ARTIFACTS")
            && self.root().is_dir()
            && let Err(error) = self.keep_diagnostics(Path::new(&folder))
        {
            eprintln!("could not keep the diagnostics of {}: {error}", self.name);
        }

        let output = self.run(&["down"]);

        if !output.status.success() {
            eprintln!(
                "ruta {} did not go down: {}",
                self.name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

pub(crate) struct Config<'a> {
    // Overrides are TOML lists, not shell command lines.
    pub(crate) command: Option<&'a str>,
    pub(crate) title: &'a str,
    pub(crate) ready: &'a str,
    pub(crate) size: (u32, u32),
}

impl Default for Config<'_> {
    fn default() -> Self {
        Self {
            command: None,
            title: TITLE,
            ready: JSON_READY,
            size: WINDOW_SIZE,
        }
    }
}

impl Config<'_> {
    pub(crate) fn text(&self) -> String {
        let (width, height) = self.size;
        let title_to_find = self.title;
        let ready = self.ready;

        let generated_command;
        let command = if let Some(command) = self.command {
            command
        } else {
            let app = test_app().to_str().expect("the test app path is UTF-8");
            generated_command = format!(r#"["{app}", "{TITLE}", "{width}", "{height}"]"#);
            &generated_command
        };

        format!(
            r#"name = "e2e"

[app]
command = {command}
title = "{title_to_find}"
width = {width}
height = {height}

[ready]
{ready}
"#
        )
    }
}

pub(crate) fn test_app() -> &'static Path {
    static APP: OnceLock<PathBuf> = OnceLock::new();

    APP.get_or_init(build_test_app)
}

// The test namespace has no network, so the test app's dependencies must already be downloaded.
fn build_test_app() -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "build",
            "--locked",
            "--offline",
            "-p",
            "ruta-test-app",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo build");

    assert!(
        output.status.success(),
        "cargo build --offline -p ruta-test-app failed; run `cargo build -p ruta-test-app` first: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["target"]["name"] == "ruta-test-app")
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
        .expect("cargo reports the test app executable")
}

pub(crate) fn png_size(path: &Path) -> (u32, u32) {
    let bytes = fs::read(path).expect("read the shot");

    assert!(bytes.len() >= 24, "the shot is too short for a PNG header");
    assert_eq!(&bytes[..8], PNG_SIGNATURE, "the shot is a PNG");
    assert_eq!(&bytes[12..16], b"IHDR", "the PNG starts with IHDR");

    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("four width bytes"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("four height bytes"));

    (width, height)
}

pub(crate) fn assert_gone(root: &Path) {
    assert!(!root.exists(), "{}: the root is removed", root.display());
    let marker = format!("RUTA_ROOT={}", root.display());
    assert_eq!(
        processes_with_marker(&marker),
        Vec::<PathBuf>::new(),
        "{}: every process is stopped",
        root.display()
    );
}

fn processes_with_marker(marker: &str) -> Vec<PathBuf> {
    let processes = fs::read_dir("/proc").expect("list /proc");

    processes
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("environ"))
        .filter(|environ| {
            fs::read(environ).is_ok_and(|bytes| {
                bytes
                    .split(|byte| *byte == 0)
                    .any(|variable| variable == marker.as_bytes())
            })
        })
        .collect()
}

fn run_within(command: &mut Command, timeout: Duration) -> Option<Output> {
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the bounded command");
    let exited = poll_until(timeout, Duration::from_millis(50), || {
        child
            .try_wait()
            .expect("check the bounded command")
            .is_some()
    });

    if !exited {
        child.kill().expect("stop the bounded command");
    }

    let output = child
        .wait_with_output()
        .expect("collect the bounded command");

    exited.then_some(output)
}

pub(crate) fn poll_until(
    timeout: Duration,
    interval: Duration,
    mut attempt: impl FnMut() -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;

    loop {
        if attempt() {
            return true;
        }

        if Instant::now() >= deadline {
            return false;
        }

        std::thread::sleep(interval);
    }
}

#[cfg(test)]
mod tests;
