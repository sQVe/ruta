use std::path::Path;

use crate::root::PrivateFolders;

const ALLOWED_KEYS: &[&str] = &[
    "TERM",
    "USER",
    "LOGNAME",
    "SHELL",
    "TZ",
    "LANG",
    "LANGUAGE",
    "XDG_DATA_DIRS",
    "LD_LIBRARY_PATH",
    "RUST_BACKTRACE",
    "RUST_LOG",
];

const ALLOWED_PREFIXES: &[&str] = &[
    "LC_",
    "LIBGL_",
    "MESA_",
    "VK_",
    "VULKAN_",
    "WGPU_",
    "FONTCONFIG_",
    "FREETYPE_",
    "NIX_",
    "WESTON_",
    "__",
];

const RESERVED_KEYS: &[&str] = &[
    "DISPLAY",
    "HOME",
    "PATH",
    "XAUTHORITY",
    "DBUS_SESSION_BUS_ADDRESS",
    "LIBEI_SOCKET",
    "SWAYSOCK",
];
const RESERVED_PREFIXES: &[&str] = &["WAYLAND_", "XDG_", "RUTA_"];

pub(crate) const ROOT_MARKER: &str = "RUTA_ROOT";

pub(crate) fn is_reserved(key: &str) -> bool {
    RESERVED_KEYS.contains(&key)
        || RESERVED_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

pub(crate) fn caller_environment() -> Vec<(String, String)> {
    utf8_entries(std::env::vars_os())
}

// Only allowlisted names reach a child, and none of them needs a value that is not UTF-8, so such
// an entry is dropped instead of failing the command.
fn utf8_entries(
    variables: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(String, String)> {
    variables
        .into_iter()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

pub(crate) fn sandbox_environment(
    caller: &[(String, String)],
    root: &Path,
    display: &str,
) -> Vec<(String, String)> {
    let marker = root.to_string_lossy().into_owned();
    let dirs = PrivateFolders::under(root);

    child_environment(
        caller,
        &dirs,
        &[("DISPLAY", display), (ROOT_MARKER, &marker)],
    )
}

pub(crate) fn app_environment(
    caller: &[(String, String)],
    root: &Path,
    display: &str,
    configured: &[(String, String)],
) -> Vec<(String, String)> {
    let mut environment = sandbox_environment(caller, root, display);

    for (key, value) in configured {
        set(&mut environment, key, value);
    }

    environment
}

pub(crate) fn weston_environment(
    caller: &[(String, String)],
    root: &Path,
) -> Vec<(String, String)> {
    child_environment(caller, &PrivateFolders::under(root), &[])
}

pub(crate) fn nested_weston_environment(
    caller: &[(String, String)],
    root: &Path,
    runtime: &Path,
) -> Vec<(String, String)> {
    let mut dirs = PrivateFolders::under(root);
    dirs.runtime = runtime.to_path_buf();

    child_environment(caller, &dirs, &[])
}

fn caller_key_is_allowed(key: &str) -> bool {
    ALLOWED_KEYS.contains(&key)
        || ALLOWED_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

fn set(environment: &mut Vec<(String, String)>, key: &str, value: &str) {
    environment.retain(|(existing, _)| existing != key);
    environment.push((key.to_string(), value.to_string()));
}

fn child_environment(
    caller: &[(String, String)],
    dirs: &PrivateFolders,
    extras: &[(&str, &str)],
) -> Vec<(String, String)> {
    let host_path = caller
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();

    let mut environment: Vec<_> = [
        ("HOME", dirs.home.to_string_lossy().into_owned()),
        (
            "XDG_CONFIG_HOME",
            dirs.config.to_string_lossy().into_owned(),
        ),
        ("XDG_STATE_HOME", dirs.state.to_string_lossy().into_owned()),
        ("XDG_DATA_HOME", dirs.data.to_string_lossy().into_owned()),
        ("XDG_CACHE_HOME", dirs.cache.to_string_lossy().into_owned()),
        (
            "XDG_RUNTIME_DIR",
            dirs.runtime.to_string_lossy().into_owned(),
        ),
        ("PATH", format!("{}:{host_path}", dirs.bin.display())),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect();

    for (key, value) in extras {
        set(&mut environment, key, value);
    }

    for (key, value) in caller {
        if environment.iter().any(|(existing, _)| existing == key) {
            continue;
        }

        if caller_key_is_allowed(key) {
            environment.push((key.clone(), value.clone()));
        }
    }

    environment
}

#[cfg(test)]
mod tests;
