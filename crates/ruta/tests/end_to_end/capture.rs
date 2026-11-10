use std::fs;

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
