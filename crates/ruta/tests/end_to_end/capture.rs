use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Stdio;

use super::support::{
    COMMAND_READY, Config, Project, WINDOW_SIZE, assert_gone, in_test_namespace, png_size,
};

#[test]
#[ignore = "starts Weston and Xwayland"]
fn capture_with_geometry_writes_crop_and_tears_down() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("capture-crop");
    let output = tempfile::tempdir().expect("output folder");
    let folder = output.path().to_str().expect("UTF-8 path");

    sandbox.succeed(&["up"]);
    let running_records = sandbox.records();

    sandbox.succeed(&[
        "capture",
        "--out",
        folder,
        "--",
        "--geometry",
        "10,20,120,80,2",
    ]);

    assert_eq!(png_size(&output.path().join("shot.png")), WINDOW_SIZE);
    assert_eq!(png_size(&output.path().join("crop.png")), (240, 160));
    assert!(output.path().join("capture.json").is_file());
    assert_gone(&sandbox.capture_root());
    assert_eq!(sandbox.records(), running_records);
    assert_eq!(sandbox.shot_size(), WINDOW_SIZE);
}

#[test]
#[ignore = "starts Weston and Xwayland"]
fn capture_with_command_readiness_has_no_ready_record_or_crop() {
    in_test_namespace!();

    let project = Project::new(&Config {
        ready: COMMAND_READY,
        ..Config::default()
    });
    let sandbox = project.sandbox("capture-ready");
    let output = tempfile::tempdir().expect("output folder");
    let folder = output.path().join("nested");
    let argument = folder.to_str().expect("UTF-8 path");

    sandbox.succeed(&[
        "capture",
        "--out",
        argument,
        "--",
        "--geometry",
        "10,20,120,80,2",
    ]);

    assert_eq!(png_size(&folder.join("shot.png")), WINDOW_SIZE);
    assert!(!folder.join("crop.png").exists());
    let record = fs::read_to_string(folder.join("capture.json")).expect("capture record");
    let record: serde_json::Value = serde_json::from_str(&record).expect("JSON record");
    assert!(record.get("ready").is_none());
    assert_gone(&sandbox.capture_root());
}

#[test]
#[ignore = "starts Weston and Xwayland"]
fn capture_without_geometry_writes_no_crop_and_tears_down() {
    in_test_namespace!();

    let project = Project::new(&Config::default());
    let sandbox = project.sandbox("capture-plain");
    let output = tempfile::tempdir().expect("output folder");
    let folder = output.path().to_str().expect("UTF-8 path");
    for name in ["shot.png", "crop.png", "capture.json", "keep.txt"] {
        fs::write(output.path().join(name), "old").expect("old output");
    }

    sandbox.succeed(&["capture", "--out", folder]);

    assert_eq!(png_size(&output.path().join("shot.png")), WINDOW_SIZE);
    assert!(!output.path().join("crop.png").exists());
    let record = fs::read_to_string(output.path().join("capture.json")).expect("capture record");
    let _: serde_json::Value = serde_json::from_str(&record).expect("JSON record");
    assert_eq!(
        fs::read_to_string(output.path().join("keep.txt")).expect("unrelated output"),
        "old"
    );
    assert_gone(&sandbox.capture_root());
}

#[test]
fn a_failed_capture_output_still_stops_the_sandbox_and_removes_its_root() {
    in_test_namespace!();

    let tools = tempfile::tempdir().expect("fake tools");
    write_fake_tools(tools.path());
    let project = Project::new(&Config {
        command: Some(r#"["/bin/sh", "-c", "sleep 30"]"#),
        ready: COMMAND_READY,
        ..Config::default()
    });
    let sandbox = project.sandbox("broken-stdout");
    let out = tempfile::tempdir().expect("output folder");
    let path = format!(
        "{}:{}",
        tools.path().display(),
        std::env::var("PATH").expect("PATH is set")
    );
    let full = fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");

    let output = sandbox
        .command(&["capture", "--out", out.path().to_str().expect("UTF-8 path")])
        .env("PATH", path)
        .stdout(Stdio::from(full))
        .stderr(Stdio::piped())
        .spawn()
        .expect("start capture")
        .wait_with_output()
        .expect("finish capture");

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Could not write the output"),
        "{output:?}"
    );
    assert_gone(&sandbox.capture_root());
}

fn write_fake_tools(folder: &std::path::Path) {
    write_tool(
        folder,
        "weston",
        r#"#!/bin/bash
for argument in "$@"; do
  case "$argument" in
    --log=*) log="${argument#--log=}" ;;
  esac
done
root="$(dirname "$log")"
export RUTA_ROOT="$root"
printf 'xserver listening on display :9\n' > "$log"
bash -c 'exec -a "Xwayland :9" sleep 30' &
exec -a ruta-fake-weston sleep 30
"#,
    );
    write_tool(folder, "Xwayland", "#!/bin/sh\nexit 0\n");
    write_tool(
        folder,
        "xdotool",
        r#"#!/bin/sh
case "$1" in
  search) printf '12345\n' ;;
  getwindowgeometry) printf 'WINDOW=12345\nX=0\nY=0\nWIDTH=640\nHEIGHT=480\nSCREEN=0\n' ;;
esac
exit 0
"#,
    );
    write_tool(
        folder,
        "import",
        "#!/bin/sh\necho 'the screenshot tool must not run' >&2\nexit 7\n",
    );
}

fn write_tool(folder: &std::path::Path, name: &str, script: &str) {
    let path = folder.join(name);
    fs::write(&path, script).expect("write the fake tool");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
        .expect("make the fake tool runnable");
}
