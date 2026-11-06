use super::*;

const ROOT: &str = "/var/tmp/ruta-smoke";

fn caller() -> Vec<(String, String)> {
    vec![
        ("DISPLAY".to_string(), ":0".to_string()),
        ("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string()),
        ("WAYLAND_SOCKET".to_string(), "3".to_string()),
        ("LIBEI_SOCKET".to_string(), "/run/user/1000/ei".to_string()),
        (
            "XAUTHORITY".to_string(),
            "/home/user/.Xauthority".to_string(),
        ),
        ("SWAYSOCK".to_string(), "/run/user/1000/sway".to_string()),
        (
            "DBUS_SESSION_BUS_ADDRESS".to_string(),
            "unix:path=/run/dbus".to_string(),
        ),
        ("HOME".to_string(), "/home/user".to_string()),
        ("XDG_RUNTIME_DIR".to_string(), "/run/user/1000".to_string()),
        ("PATH".to_string(), "/usr/bin".to_string()),
        ("LANG".to_string(), "en_US.UTF-8".to_string()),
        ("LC_ALL".to_string(), "C".to_string()),
        ("TRAILING".to_string(), "drop-me".to_string()),
    ]
}

fn hostile_caller() -> Vec<(String, String)> {
    [
        ("DISPLAY", ":0"),
        ("WAYLAND_DISPLAY", "wayland-0"),
        ("LIBEI_SOCKET", "/run/user/1000/eis-0"),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
        ("RUTA_ROOT", "/home/user"),
        ("HOME", "/home/user"),
        ("XDG_CONFIG_HOME", "/home/user/.config"),
        ("XDG_STATE_HOME", "/home/user/.local/state"),
        ("XDG_DATA_HOME", "/home/user/.local/share"),
        ("XDG_CACHE_HOME", "/home/user/.cache"),
        ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ("PATH", "/usr/bin:/bin"),
        ("RUST_LOG", "warn"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

fn value<'a>(environment: &'a [(String, String)], key: &str) -> Option<&'a str> {
    environment
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn assert_no_caller_session(environment: &[(String, String)]) {
    for key in [
        "WAYLAND_DISPLAY",
        "LIBEI_SOCKET",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        assert_eq!(value(environment, key), None, "{key}");
    }
}

#[test]
fn drops_caller_desktop_and_keeps_sandbox_paths() {
    let dirs = PrivateFolders::under(Path::new(ROOT));
    let environment = child_environment(&caller(), &dirs, &[("DISPLAY", ":7")]);

    assert_eq!(value(&environment, "DISPLAY"), Some(":7"));
    assert_eq!(value(&environment, "WAYLAND_DISPLAY"), None);
    assert_eq!(value(&environment, "WAYLAND_SOCKET"), None);
    assert_eq!(value(&environment, "LIBEI_SOCKET"), None);
    assert_eq!(value(&environment, "XAUTHORITY"), None);
    assert_eq!(value(&environment, "SWAYSOCK"), None);
    assert_eq!(value(&environment, "DBUS_SESSION_BUS_ADDRESS"), None);
    assert_eq!(value(&environment, "TRAILING"), None);
    assert_eq!(
        value(&environment, "HOME"),
        Some("/var/tmp/ruta-smoke/home")
    );
    assert_eq!(
        value(&environment, "XDG_RUNTIME_DIR"),
        Some("/var/tmp/ruta-smoke/run")
    );
    assert_eq!(
        value(&environment, "PATH"),
        Some("/var/tmp/ruta-smoke/bin:/usr/bin")
    );
    assert_eq!(value(&environment, "LANG"), Some("en_US.UTF-8"));
    assert_eq!(value(&environment, "LC_ALL"), Some("C"));
}

#[test]
fn extras_replace_the_caller_value() {
    let environment = child_environment(
        &caller(),
        &PrivateFolders::under(Path::new(ROOT)),
        &[("WAYLAND_DISPLAY", "wayland-1"), ("DISPLAY", ":7")],
    );

    assert_eq!(value(&environment, "WAYLAND_DISPLAY"), Some("wayland-1"));
    assert_eq!(
        environment
            .iter()
            .filter(|(key, _)| key == "WAYLAND_DISPLAY")
            .count(),
        1
    );
}

#[test]
fn sandbox_environment_uses_the_private_display_and_folders() {
    let environment = sandbox_environment(&hostile_caller(), Path::new(ROOT), ":7");

    assert_no_caller_session(&environment);

    for (key, expected) in [
        ("DISPLAY", ":7"),
        ("RUTA_ROOT", ROOT),
        ("HOME", "/var/tmp/ruta-smoke/home"),
        ("XDG_CONFIG_HOME", "/var/tmp/ruta-smoke/config"),
        ("XDG_STATE_HOME", "/var/tmp/ruta-smoke/state"),
        ("XDG_DATA_HOME", "/var/tmp/ruta-smoke/data"),
        ("XDG_CACHE_HOME", "/var/tmp/ruta-smoke/cache"),
        ("XDG_RUNTIME_DIR", "/var/tmp/ruta-smoke/run"),
        ("PATH", "/var/tmp/ruta-smoke/bin:/usr/bin:/bin"),
    ] {
        assert_eq!(value(&environment, key), Some(expected), "{key}");
    }
}

#[test]
fn app_environment_adds_the_configured_keys_over_the_caller() {
    let configured = vec![
        ("RUST_LOG".to_string(), "info".to_string()),
        ("APP_MODE".to_string(), "test".to_string()),
    ];

    let environment = app_environment(&hostile_caller(), Path::new(ROOT), ":7", &configured);

    assert_no_caller_session(&environment);
    assert_eq!(value(&environment, "RUST_LOG"), Some("info"));
    assert_eq!(value(&environment, "APP_MODE"), Some("test"));
    assert_eq!(value(&environment, "DISPLAY"), Some(":7"));
    assert_eq!(value(&environment, "RUTA_ROOT"), Some(ROOT));
    assert_eq!(
        environment
            .iter()
            .filter(|(key, _)| key == "RUST_LOG")
            .count(),
        1
    );
}

#[test]
fn weston_environment_has_no_caller_display_or_session() {
    let environment = weston_environment(&hostile_caller(), Path::new(ROOT));

    assert_no_caller_session(&environment);

    for (key, expected) in [
        ("DISPLAY", None),
        ("RUTA_ROOT", None),
        ("HOME", Some("/var/tmp/ruta-smoke/home")),
        ("XDG_STATE_HOME", Some("/var/tmp/ruta-smoke/state")),
        ("XDG_DATA_HOME", Some("/var/tmp/ruta-smoke/data")),
        ("XDG_CACHE_HOME", Some("/var/tmp/ruta-smoke/cache")),
        ("XDG_RUNTIME_DIR", Some("/var/tmp/ruta-smoke/run")),
        ("PATH", Some("/var/tmp/ruta-smoke/bin:/usr/bin:/bin")),
    ] {
        assert_eq!(value(&environment, key), expected, "{key}");
    }
}

#[test]
fn nested_weston_environment_gets_only_the_given_runtime_folder() {
    let environment = nested_weston_environment(
        &hostile_caller(),
        Path::new(ROOT),
        Path::new("/run/user/1000"),
    );

    assert_no_caller_session(&environment);

    for (key, expected) in [
        ("DISPLAY", None),
        ("XDG_RUNTIME_DIR", Some("/run/user/1000")),
        ("HOME", Some("/var/tmp/ruta-smoke/home")),
        ("XDG_CONFIG_HOME", Some("/var/tmp/ruta-smoke/config")),
        ("XDG_STATE_HOME", Some("/var/tmp/ruta-smoke/state")),
        ("XDG_DATA_HOME", Some("/var/tmp/ruta-smoke/data")),
        ("XDG_CACHE_HOME", Some("/var/tmp/ruta-smoke/cache")),
    ] {
        assert_eq!(value(&environment, key), expected, "{key}");
    }
}
