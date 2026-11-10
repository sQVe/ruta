use std::fs;
use std::path::Path;

use crate::identity::ProcessIdentity;
use crate::identity::tests::record_changed_start;

pub(crate) fn record_live_display(root: &Path, display: &str) {
    let identity =
        ProcessIdentity::current(std::process::id()).expect("the test process has a start time");

    identity
        .write(&root.join("weston.pid"))
        .expect("write weston.pid");
    fs::write(root.join("display"), display).expect("write the display");
}

pub(crate) fn record_display_settings(root: &Path, mode: &str, (width, height): (u32, u32)) {
    fs::write(root.join("mode"), format!("{mode}\n")).expect("write the mode");
    fs::write(root.join("size"), format!("{width}x{height}\n")).expect("write the size");
}

pub(crate) type StaleDisplayCase = (&'static str, fn(&Path));

pub(crate) fn stale_display_cases() -> [StaleDisplayCase; 3] {
    [
        ("changed start", |root| {
            record_changed_start(&root.join("weston.pid"), std::process::id());
        }),
        ("missing pid file", |root| {
            fs::remove_file(root.join("weston.pid")).expect("remove weston.pid");
        }),
        ("no colon", |root| {
            fs::write(root.join("display"), "7\n").expect("rewrite the display");
        }),
    ]
}
