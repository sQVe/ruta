use super::*;

#[test]
fn find_program_uses_the_given_path() {
    let directory = crate::test_support::temporary_directory();
    let program = directory.path().join("fake-tool");
    std::fs::write(&program, "#!/bin/sh\n").expect("write the fake tool");
    std::fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("make the tool runnable");

    let found = find_program("fake-tool", &directory.path().to_string_lossy());
    assert_eq!(found, Some(program));
    assert_eq!(find_program("fake-tool", "/nonexistent"), None);
}

#[test]
fn find_program_skips_a_file_that_is_not_executable() {
    use std::os::unix::fs::PermissionsExt as _;

    let plain = crate::test_support::temporary_directory();
    let runnable = crate::test_support::temporary_directory();
    let skipped = plain.path().join("fake-tool");
    let program = runnable.path().join("fake-tool");
    for path in [&skipped, &program] {
        std::fs::write(path, "#!/bin/sh\n").expect("write the fake tool");
    }
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
        .expect("make the tool runnable");
    let plain_path = plain.path().to_string_lossy().into_owned();
    let both = format!("{plain_path}:{}", runnable.path().display());

    // 0o641 lets only others run a file that the current user owns. Root may run any file with
    // an execute bit, so that case applies only to other users.
    let modes: &[u32] = if rustix::process::geteuid().is_root() {
        &[0o644]
    } else {
        &[0o644, 0o641]
    };

    for &mode in modes {
        std::fs::set_permissions(&skipped, std::fs::Permissions::from_mode(mode))
            .expect("set the tool mode");

        assert_eq!(find_program("fake-tool", &plain_path), None, "{mode:o}");
        assert_eq!(
            find_program("fake-tool", &both),
            Some(program.clone()),
            "{mode:o}"
        );
    }
}

#[test]
fn missing_tools_keep_their_names_and_package_messages() {
    for (tool, name, package) in [
        (Tool::Weston, "weston", "weston"),
        (Tool::Xwayland, "Xwayland", "xorg-xwayland"),
        (Tool::Xdotool, "xdotool", "xdotool"),
        (Tool::Import, "import", "imagemagick"),
        (Tool::Magick, "magick", "imagemagick"),
        (Tool::Cat, "cat", "coreutils"),
    ] {
        let error = require_program_in(tool, "/nonexistent").expect_err("missing tool");

        assert_eq!(error, MissingTool { tool });
        assert_eq!(tool.name(), name);
        assert_eq!(
            error.to_string(),
            format!("The tool \"{name}\" is missing. Install the {package} package and try again.")
        );
    }
}

#[test]
fn require_program_accepts_an_executable_tool() {
    let directory = crate::test_support::temporary_directory();
    let program = directory.path().join("weston");
    fs::write(&program, "#!/bin/sh\n").expect("write the tool");
    fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("make the tool runnable");

    assert_eq!(
        require_program_in(Tool::Weston, &directory.path().to_string_lossy()),
        Ok(())
    );
}
