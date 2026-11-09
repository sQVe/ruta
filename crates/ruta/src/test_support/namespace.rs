// Tests that send signals run in a user and PID namespace with its own `/proc`, so code that scans
// `/proc` and signals what it finds can only reach test processes, even when a bug or a mutant
// widens what it selects.
//
// A guarded test that starts on the host re-runs itself, and only itself, through `unshare`, and
// returns without running its body on the host. Inside, the test binary is the namespace's init.
// The check reads the kernel, not the environment: only the init of a PID namespace has PID 1, and
// `/proc/self` names PID 1 only when `/proc` belongs to that namespace.
//
// Inside, `/tmp` is a private tmpfs before the body runs. X lock files and sockets, which X servers
// judge by a PID that another namespace cannot see, then never cross tests or reach the host. The
// network namespace is new too, because X clients connect first to an abstract socket, which
// belongs to the network namespace and not to `/tmp`.
//
// This file is shared by the unit tests and the end-to-end harness, so it uses no crate paths.

use std::path::Path;
use std::process::Command;

const INSIDE: &str = "RUTA_TEST_NAMESPACE";
const PRIVATE_TMP: &str = "/tmp";

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
    rerun_outside_test_namespace_with(&[])
}

// As `rerun_outside_test_namespace`, and passes `variables` to the run inside.
pub(crate) fn rerun_outside_test_namespace_with(variables: &[(&str, &str)]) -> bool {
    if inside_test_namespace() {
        isolate_inside();
        return false;
    }

    assert!(
        std::env::var_os(INSIDE).is_none(),
        "refusing to run a signal test: it was started for a test namespace, but it is not \
         inside one (PID {}, /proc/self {:?})",
        std::process::id(),
        std::fs::read_link("/proc/self")
    );

    let test_binary = std::env::current_exe().expect("the test binary has a path");

    assert!(
        !test_binary.starts_with(PRIVATE_TMP),
        "refusing to run a signal test: the test binary {} lives under {PRIVATE_TMP}, which the \
         test namespace replaces; build the tests outside it, for example with TMPDIR=/var/tmp",
        test_binary.display()
    );

    let name = std::thread::current()
        .name()
        .filter(|name| *name != "main")
        .expect("libtest names the test thread after the test")
        .to_string();
    let output = Command::new("unshare")
        .args([
            "--user",
            "--map-current-user",
            "--keep-caps",
            "--pid",
            "--net",
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
        // Temporary folders inside go to the private `/tmp`, not a caller's `TMPDIR`.
        .env("TMPDIR", PRIVATE_TMP)
        .envs(variables.iter().copied())
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

// Mounts a private tmpfs on `/tmp`, then drops the capabilities that `unshare --keep-caps` kept,
// so the body runs with the same rights as on the host. Any failure stops the test before its body.
fn isolate_inside() {
    use rustix::mount::{MountFlags, MountPropagationFlags, mount, mount_change};

    // `unshare` makes every mount private already. This keeps the tmpfs from reaching the host
    // even if that default changes.
    mount_change(
        "/",
        MountPropagationFlags::PRIVATE | MountPropagationFlags::REC,
    )
    .unwrap_or_else(|error| refuse("could not make the mounts private", &error));
    mount(
        "tmpfs",
        PRIVATE_TMP,
        "tmpfs",
        MountFlags::NOSUID | MountFlags::NODEV,
        c"mode=1777",
    )
    .unwrap_or_else(|error| refuse("could not mount a private /tmp", &error));

    let sockets = Path::new(PRIVATE_TMP).join(".X11-unix");
    std::fs::create_dir(&sockets)
        .and_then(|()| {
            std::fs::set_permissions(
                &sockets,
                std::os::unix::fs::PermissionsExt::from_mode(0o1777),
            )
        })
        .unwrap_or_else(|error| refuse("could not create the private X socket folder", &error));

    rustix::thread::set_capabilities(
        None,
        rustix::thread::CapabilitySets {
            effective: rustix::thread::CapabilitySet::empty(),
            permitted: rustix::thread::CapabilitySet::empty(),
            inheritable: rustix::thread::CapabilitySet::empty(),
        },
    )
    .unwrap_or_else(|error| refuse("could not drop the namespace capabilities", &error));
}

fn refuse(action: &str, error: &dyn std::fmt::Display) -> ! {
    panic!("refusing to run a signal test: {action}: {error}")
}
