use std::process::Command;

#[test]
fn version_prints_without_config() {
    let folder = tempfile::tempdir().expect("create an empty folder");
    let output = Command::new(env!("CARGO_BIN_EXE_ruta"))
        .arg("--version")
        .current_dir(folder.path())
        .output()
        .expect("run ruta");

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        output.stdout,
        concat!("ruta ", env!("CARGO_PKG_VERSION"), "\n").as_bytes()
    );
}

#[test]
fn unknown_options_and_version_arguments_print_usage() {
    let folder = tempfile::tempdir().expect("create an empty folder");
    for arguments in [
        ["--version", "extra"].as_slice(),
        &["--help"],
        &["-V"],
        &["version"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_ruta"))
            .args(arguments)
            .current_dir(folder.path())
            .output()
            .expect("run ruta");

        assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).expect("UTF-8 usage");
        assert!(stderr.starts_with("Usage:\n"), "{stderr}");
        assert!(stderr.contains("\n  ruta --version\n"), "{stderr}");
    }
}
