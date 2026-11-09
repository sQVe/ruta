use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tempfile::TempDir;

// The first statement of every end-to-end test: `ruta down` sweeps `/proc`, so each test runs in
// a test namespace. See `src/test_support/namespace.rs`.
macro_rules! in_test_namespace {
    () => {
        if crate::namespace::rerun_outside_test_namespace() {
            return;
        }
    };
}

pub(crate) use in_test_namespace;

pub(crate) const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
pub(crate) const WINDOW_SIZE: (u32, u32) = (640, 480);
pub(crate) const TITLE: &str = "ruta test app";

pub(crate) const JSON_READY: &str = r#"line = "json""#;
pub(crate) const COMMAND_READY: &str = r#"command = ["true"]"#;
pub(crate) const RECORDS: &[&str] = &[
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
    pub(crate) fn with_config(text: &str) -> Self {
        let folder = tempfile::Builder::new()
            .prefix("ruta-end-to-end-")
            .tempdir()
            .expect("create the project folder");
        let project = Self { folder };

        project.write_config(text);

        project
    }

    pub(crate) fn new(title_to_find: &str) -> Self {
        Self::with_ready(title_to_find, JSON_READY)
    }

    pub(crate) fn with_ready(title_to_find: &str, ready: &str) -> Self {
        Self::with_config(&config_text(title_to_find, ready))
    }

    pub(crate) fn write_config(&self, text: &str) {
        fs::write(self.folder.path().join("ruta.toml"), text).expect("write ruta.toml");
    }

    pub(crate) fn sandbox(&self, label: &str) -> Sandbox {
        Sandbox {
            name: format!("e2e-{label}-{}", std::process::id()),
            folder: self.folder.path().to_path_buf(),
        }
    }
}

pub(crate) struct Sandbox {
    name: String,
    folder: PathBuf,
}

impl Sandbox {
    pub(crate) fn root(&self) -> PathBuf {
        PathBuf::from(format!("/var/tmp/ruta-{}", self.name))
    }

    pub(crate) fn marker(&self) -> String {
        format!("RUTA_ROOT={}", self.root().display())
    }

    pub(crate) fn run(&self, arguments: &[&str]) -> Output {
        self.command(arguments)
            .output()
            .expect("run the ruta binary")
    }

    pub(crate) fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ruta"));
        command
            .args(arguments)
            .current_dir(&self.folder)
            .env("RUTA_NAME", &self.name);

        command
    }

    pub(crate) fn succeed(&self, arguments: &[&str]) -> String {
        let output = self.run(arguments);

        assert!(
            output.status.success(),
            "ruta {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout).expect("the output is UTF-8")
    }

    pub(crate) fn shot_size(&self) -> (u32, u32) {
        let shot = self.root().join("end-to-end.png");
        let shot_argument = shot.to_str().expect("the shot path is UTF-8");

        self.succeed(&["shot", shot_argument]);

        png_size(&shot)
    }

    pub(crate) fn assert_gone(&self) {
        assert!(!self.root().exists(), "{}: the root is removed", self.name);
        assert_eq!(
            processes_with_marker(&self.marker()),
            Vec::<PathBuf>::new(),
            "{}: every process is stopped",
            self.name
        );
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

impl Drop for Sandbox {
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

pub(crate) fn config_text(title_to_find: &str, ready: &str) -> String {
    config_text_at_size(title_to_find, ready, WINDOW_SIZE)
}

pub(crate) fn config_text_at_size(title_to_find: &str, ready: &str, size: (u32, u32)) -> String {
    let app = test_app().to_str().expect("the test app path is UTF-8");
    let (width, height) = size;

    config_for_app_at_size(
        &format!(r#"["{app}", "{TITLE}", "{width}", "{height}"]"#),
        title_to_find,
        ready,
        size,
    )
}

// `command` is the TOML list that launches the app.
pub(crate) fn config_for_app(command: &str, title_to_find: &str, ready: &str) -> String {
    config_for_app_at_size(command, title_to_find, ready, WINDOW_SIZE)
}

fn config_for_app_at_size(
    command: &str,
    title_to_find: &str,
    ready: &str,
    (width, height): (u32, u32),
) -> String {
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

pub(crate) fn records(root: &Path) -> Vec<(&'static str, Option<String>)> {
    RECORDS
        .iter()
        .map(|name| (*name, fs::read_to_string(root.join(name)).ok()))
        .collect()
}

pub(crate) fn test_app() -> &'static Path {
    static APP: OnceLock<PathBuf> = OnceLock::new();

    APP.get_or_init(build_test_app)
}

fn build_test_app() -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "build",
            "--locked",
            "-p",
            "ruta-test-app",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo build");

    assert!(
        output.status.success(),
        "cargo build -p ruta-test-app failed: {}",
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

pub(crate) fn processes_with_marker(marker: &str) -> Vec<PathBuf> {
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

pub(crate) fn wait_for_log(sandbox: &Sandbox, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let log = sandbox.succeed(&["log"]);

        if log.contains(expected) || Instant::now() >= deadline {
            return log;
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}
