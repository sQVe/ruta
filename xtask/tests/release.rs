use std::process::Command;

#[test]
fn release_check_dispatches_only_its_task() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is below the workspace root");
    let text =
        std::fs::read_to_string(root.join("crates/ruta/Cargo.toml")).expect("read ruta manifest");
    let manifest: toml::Table = toml::from_str(&text).expect("parse ruta manifest");
    let version = manifest["package"]["version"]
        .as_str()
        .expect("crate version");
    let tag = format!("v{version}");

    for (arguments, code, message) in [
        (
            vec!["release-check", tag.as_str()],
            0,
            format!("Release tag {tag} matches the crate version.\n"),
        ),
        (
            vec!["release-check", "vnot-the-crate-version"],
            1,
            format!("Release tag vnot-the-crate-version differs from crate version {version}\n"),
        ),
        (
            vec!["release-check", version],
            1,
            format!("Release tag {version} must start with v\n"),
        ),
        (
            vec!["unknown-task", tag.as_str()],
            1,
            "Usage:\n".to_string(),
        ),
        (
            vec!["release-check"],
            1,
            "Usage: cargo xtask release-check <tag>\n".to_string(),
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(&arguments)
            .output()
            .expect("run xtask");

        assert_eq!(
            output.status.code(),
            Some(code),
            "{arguments:?}: {output:?}"
        );
        assert!(output.stdout.is_empty(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
        if arguments[0] == "unknown-task" {
            assert!(stderr.starts_with(&message), "{stderr}");
        } else {
            assert_eq!(stderr, message, "{arguments:?}");
        }
    }
}
