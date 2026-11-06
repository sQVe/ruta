use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tempfile::TempDir;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const WINDOW_SIZE: (u32, u32) = (640, 480);
const TITLE: &str = "ruta test app";

const JSON_READY: &str = r#"line = "json""#;
const COMMAND_READY: &str = r#"command = ["true"]"#;
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

struct Project {
    folder: TempDir,
}

impl Project {
    fn new(title_to_find: &str) -> Self {
        Self::with_ready(title_to_find, JSON_READY)
    }

    fn with_ready(title_to_find: &str, ready: &str) -> Self {
        let folder = tempfile::Builder::new()
            .prefix("ruta-end-to-end-")
            .tempdir()
            .expect("create the project folder");
        let project = Self { folder };

        project.write_config(&config_text(title_to_find, ready));

        project
    }

    fn write_config(&self, text: &str) {
        fs::write(self.folder.path().join("ruta.toml"), text).expect("write ruta.toml");
    }

    fn sandbox(&self, label: &str) -> Sandbox {
        Sandbox {
            name: format!("e2e-{label}-{}", std::process::id()),
            folder: self.folder.path().to_path_buf(),
        }
    }
}

struct Sandbox {
    name: String,
    folder: PathBuf,
}

impl Sandbox {
    fn root(&self) -> PathBuf {
        PathBuf::from(format!("/var/tmp/ruta-{}", self.name))
    }

    fn marker(&self) -> String {
        format!("RUTA_ROOT={}", self.root().display())
    }

    fn run(&self, arguments: &[&str]) -> Output {
        self.command(arguments)
            .output()
            .expect("run the ruta binary")
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ruta"));
        command
            .args(arguments)
            .current_dir(&self.folder)
            .env("RUTA_NAME", &self.name);

        command
    }

    fn succeed(&self, arguments: &[&str]) -> String {
        let output = self.run(arguments);

        assert!(
            output.status.success(),
            "ruta {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        String::from_utf8(output.stdout).expect("the output is UTF-8")
    }

    fn shot_size(&self) -> (u32, u32) {
        let shot = self.root().join("end-to-end.png");
        let shot_argument = shot.to_str().expect("the shot path is UTF-8");

        self.succeed(&["shot", shot_argument]);

        png_size(&shot)
    }

    fn assert_gone(&self) {
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

fn config_text(title_to_find: &str, ready: &str) -> String {
    let app = test_app().to_str().expect("the test app path is UTF-8");
    let (width, height) = WINDOW_SIZE;

    format!(
        r#"name = "e2e"

[app]
command = ["{app}", "{TITLE}", "{width}", "{height}"]
title = "{title_to_find}"
width = {width}
height = {height}

[ready]
{ready}

[environment]
RUST_BACKTRACE = "1"
"#
    )
}

fn records(root: &Path) -> Vec<(&'static str, Option<String>)> {
    RECORDS
        .iter()
        .map(|name| (*name, fs::read_to_string(root.join(name)).ok()))
        .collect()
}

fn test_app() -> &'static Path {
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

fn png_size(path: &Path) -> (u32, u32) {
    let bytes = fs::read(path).expect("read the shot");

    assert!(bytes.len() >= 24, "the shot is too short for a PNG header");
    assert_eq!(&bytes[..8], PNG_SIGNATURE, "the shot is a PNG");
    assert_eq!(&bytes[12..16], b"IHDR", "the PNG starts with IHDR");

    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("four width bytes"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("four height bytes"));

    (width, height)
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

fn wait_for_log(sandbox: &Sandbox, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let log = sandbox.succeed(&["log"]);

        if log.contains(expected) || Instant::now() >= deadline {
            return log;
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn up_shoots_logs_and_goes_down() {
    let project = Project::new(TITLE);
    let sandbox = project.sandbox("up");

    sandbox.succeed(&["up", "--", "--headed", "extra"]);

    let ready = fs::read_to_string(sandbox.root().join("ready.json")).expect("read ready.json");
    let ready: serde_json::Value = serde_json::from_str(&ready).expect("ready.json is JSON");
    assert_eq!(
        ready,
        serde_json::json!({
            "title": TITLE,
            "width": WINDOW_SIZE.0,
            "height": WINDOW_SIZE.1,
            "arguments": ["--headed", "extra"],
        }),
        "ready.json holds the app's ready line"
    );

    assert_eq!(
        sandbox.shot_size(),
        WINDOW_SIZE,
        "the shot has the window size"
    );

    let log = wait_for_log(&sandbox, "ruta-test-app: running");
    for expected in [
        "ruta-test-app: starting\n",
        "\"title\":\"ruta test app\"",
        "ruta-test-app: running\n",
    ] {
        assert!(log.contains(expected), "the log holds {expected:?}: {log}");
    }

    assert_eq!(sandbox.succeed(&["opened"]), "", "no link was opened");

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn failed_up_leaves_no_root_and_no_process() {
    let project = Project::new("no window has this title");
    let sandbox = project.sandbox("bad");

    let output = sandbox.run(&["up"]);

    assert!(
        !output.status.success(),
        "up without a matching window fails"
    );
    sandbox.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn two_sandboxes_run_side_by_side() {
    let project = Project::new(TITLE);
    let first = project.sandbox("one");
    let second = project.sandbox("two");

    first.succeed(&["up"]);
    second.succeed(&["up"]);

    assert_ne!(first.root(), second.root());
    assert_eq!(first.shot_size(), WINDOW_SIZE, "the first shot");
    assert_eq!(second.shot_size(), WINDOW_SIZE, "the second shot");

    first.succeed(&["down"]);
    first.assert_gone();
    assert_eq!(second.shot_size(), WINDOW_SIZE, "the second stays up");

    second.succeed(&["down"]);
    second.assert_gone();
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_bad_config_or_bad_arguments_leave_a_running_sandbox_unchanged() {
    let project = Project::new(TITLE);
    let sandbox = project.sandbox("preflight");
    sandbox.succeed(&["up"]);
    let before = records(&sandbox.root());

    project.write_config(&format!(
        "colour = \"red\"\n{}",
        config_text(TITLE, JSON_READY)
    ));
    for arguments in [&["up"][..], &["restart"], &["down"]] {
        let output = sandbox.run(arguments);

        assert!(
            !output.status.success(),
            "{arguments:?} with a bad ruta.toml fails"
        );
    }

    project.write_config(&config_text(TITLE, JSON_READY));
    for arguments in [
        &["shot", "a.png", "b.png"][..],
        &["nonsense"],
        &["log", "x"],
    ] {
        let output = sandbox.run(arguments);

        assert!(!output.status.success(), "{arguments:?} fails");
    }

    assert_eq!(
        records(&sandbox.root()),
        before,
        "the sandbox records are unchanged"
    );
    assert_eq!(sandbox.shot_size(), WINDOW_SIZE, "the sandbox is still up");
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn command_ready_up_writes_no_ready_json() {
    let project = Project::with_ready(TITLE, COMMAND_READY);
    let sandbox = project.sandbox("command");

    sandbox.succeed(&["up"]);

    assert!(
        !sandbox.root().join("ready.json").exists(),
        "command-ready mode writes no ready.json"
    );
    assert_eq!(
        sandbox.shot_size(),
        WINDOW_SIZE,
        "the shot has the window size"
    );

    sandbox.succeed(&["down"]);
    sandbox.assert_gone();
}

fn path_with_xdotool_mode(mode: u32) -> TempDir {
    let folder = tempfile::Builder::new()
        .prefix("ruta-end-to-end-path-")
        .tempdir()
        .expect("create the PATH folder");

    for tool in ["weston", "Xwayland", "import"] {
        let real = find_on_path(tool).unwrap_or_else(|| panic!("{tool} is installed"));
        std::os::unix::fs::symlink(real, folder.path().join(tool)).expect("link the tool");
    }

    let plain = folder.path().join("xdotool");
    fs::write(&plain, "#!/bin/sh\nexit 0\n").expect("write the plain xdotool");
    fs::set_permissions(&plain, std::os::unix::fs::PermissionsExt::from_mode(mode))
        .expect("set the xdotool mode");

    folder
}

fn find_on_path(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;

    std::env::split_paths(&path)
        .map(|folder| folder.join(tool))
        .find(|candidate| candidate.is_file())
}

#[test]
#[ignore = "needs Weston, Xwayland, xdotool, and ImageMagick"]
fn a_tool_the_user_cannot_run_is_refused_and_leaves_a_running_sandbox_up() {
    let project = Project::new(TITLE);
    let sandbox = project.sandbox("plain-tool");
    sandbox.succeed(&["up"]);
    let before = records(&sandbox.root());

    // 0o641 lets only others run a file that the current user owns. Root may run any file with an
    // execute bit, so that case applies only to other users.
    let modes: &[u32] = if rustix::process::geteuid().is_root() {
        &[0o644]
    } else {
        &[0o644, 0o641]
    };

    for &mode in modes {
        let path = path_with_xdotool_mode(mode);

        for arguments in [&["up"][..], &["restart"]] {
            let output = sandbox
                .command(arguments)
                .env("PATH", path.path())
                .output()
                .expect("run the ruta binary");
            let stderr = String::from_utf8_lossy(&output.stderr);

            assert!(!output.status.success(), "{mode:o} {arguments:?} fails");
            assert!(
                stderr.contains("The tool \"xdotool\" is missing."),
                "{mode:o} {arguments:?}: {stderr}"
            );
        }
    }

    assert_eq!(
        records(&sandbox.root()),
        before,
        "the sandbox records are unchanged"
    );
    assert_eq!(sandbox.shot_size(), WINDOW_SIZE, "the sandbox is still up");
}
