// Tests that send signals run in a user and PID namespace with its own `/proc`, so code that scans
// `/proc` and signals what it finds can only reach test processes, even when a bug or a mutant
// widens what it selects.
//
// A guarded test that starts on the host re-runs itself, and only itself, through `unshare`, and
// returns without running its body on the host. Inside, the test binary is the namespace's init.
// The check reads the kernel, not the environment: only the init of a PID namespace has PID 1, and
// `/proc/self` names PID 1 only when `/proc` belongs to that namespace.
//
// This file is shared by the unit tests and the end-to-end harness, so it uses no crate paths.

use std::path::Path;
use std::process::Command;

const INSIDE: &str = "RUTA_TEST_NAMESPACE";

pub(crate) fn inside_test_namespace() -> bool {
    let pid_is_init = std::process::id() == 1;
    let proc_is_ours =
        std::fs::read_link("/proc/self").is_ok_and(|target| target == Path::new("1"));

    pid_is_init && proc_is_ours
}

// Returns false inside the namespace, so the caller runs its body. Outside, it runs the current
// test inside a new namespace, panics when that fails, and returns true, so the caller returns
// without running its body on the host.
pub(crate) fn rerun_outside_test_namespace() -> bool {
    if inside_test_namespace() {
        return false;
    }

    assert!(
        std::env::var_os(INSIDE).is_none(),
        "refusing to run a signal test: it was started for a test namespace, but it is not \
         inside one (PID {}, /proc/self {:?})",
        std::process::id(),
        std::fs::read_link("/proc/self")
    );

    let name = std::thread::current()
        .name()
        .filter(|name| *name != "main")
        .expect("libtest names the test thread after the test")
        .to_string();
    let test_binary = std::env::current_exe().expect("the test binary has a path");
    let output = Command::new("unshare")
        .args([
            "--user",
            "--map-current-user",
            "--pid",
            "--fork",
            "--mount-proc",
            "--kill-child",
            "--",
        ])
        .arg(&test_binary)
        .args([
            "--exact",
            &name,
            "--include-ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(INSIDE, "1")
        .output()
        .unwrap_or_else(|error| {
            panic!("refusing to run a signal test: could not start unshare: {error}")
        });
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ran_once = stdout.contains("test result: ok. 1 passed");

    assert!(
        output.status.success() && ran_once,
        "the test {name} failed in its test namespace ({}):\n{stdout}\n{stderr}",
        output.status
    );

    true
}
