use std::path::Path;
use std::time::Duration;

use super::{INPUT_SETTLE, xdotool};
use crate::environment::{caller_environment, sandbox_environment};
use crate::process::TOOL_TIMEOUT;

const CLICK_SETTLE: Duration = Duration::from_millis(200);
const STEP_DELAY: Duration = Duration::from_millis(16);
const DRAG_STEPS: u32 = 24;
const TYPE_DELAY: Duration = Duration::from_millis(30);
const NOTCH_DELAY: Duration = Duration::from_millis(50);
const WHEEL_UP: u8 = 4;
const WHEEL_DOWN: u8 = 5;

#[mutants::skip]
fn warm_up(environment: &[(String, String)], window: &str) -> anyhow::Result<()> {
    super::set_focus(environment, window)?;
    move_pointer(environment, window, (0, 0))?;
    std::thread::sleep(INPUT_SETTLE);

    Ok(())
}

#[mutants::skip]
pub(crate) fn glide(
    root: &Path,
    display: &str,
    window: &str,
    from: (i32, i32),
    to: (i32, i32),
) -> anyhow::Result<()> {
    let environment = sandbox_environment(&caller_environment(), root, display);
    warm_up(&environment, window)?;
    glide_steps(&environment, window, from, to)
}

#[mutants::skip]
fn glide_steps(
    environment: &[(String, String)],
    window: &str,
    from: (i32, i32),
    to: (i32, i32),
) -> anyhow::Result<()> {
    for step in 0..=DRAG_STEPS {
        let x = interpolate(from.0, to.0, step);
        let y = interpolate(from.1, to.1, step);

        move_pointer(environment, window, (x, y))?;
        std::thread::sleep(STEP_DELAY);
    }

    Ok(())
}

#[mutants::skip]
pub(crate) fn click(
    root: &Path,
    display: &str,
    window: &str,
    (x, y): (i32, i32),
    button: u8,
) -> anyhow::Result<()> {
    let environment = sandbox_environment(&caller_environment(), root, display);
    point_at(&environment, window, (x, y))?;
    xdotool(&environment, &["click", &button.to_string()], TOOL_TIMEOUT)
}

// Presses button 1, glides, and always releases after a successful press. When both the motion and
// the release fail, the motion error is returned, so the caller sees the first cause.
#[mutants::skip]
pub(crate) fn drag(
    root: &Path,
    display: &str,
    window: &str,
    from: (i32, i32),
    to: (i32, i32),
) -> anyhow::Result<()> {
    let environment = sandbox_environment(&caller_environment(), root, display);
    point_at(&environment, window, from)?;
    xdotool(&environment, &["mousedown", "1"], TOOL_TIMEOUT)?;

    let motion = (|| {
        std::thread::sleep(CLICK_SETTLE);
        glide_steps(&environment, window, from, to)?;
        std::thread::sleep(INPUT_SETTLE);

        Ok(())
    })();
    let released = xdotool(&environment, &["mouseup", "1"], TOOL_TIMEOUT);

    motion.and(released)
}

#[mutants::skip]
pub(crate) fn scroll(
    root: &Path,
    display: &str,
    window: &str,
    (x, y): (i32, i32),
    notches: i32,
) -> anyhow::Result<()> {
    let steps = usize::try_from(notches.unsigned_abs()).unwrap_or(usize::MAX);
    let environment = sandbox_environment(&caller_environment(), root, display);

    point_at(&environment, window, (x, y))?;
    xdotool(
        &environment,
        &scroll_arguments(notches),
        paced_timeout(steps, NOTCH_DELAY),
    )
}

#[mutants::skip]
pub(crate) fn type_text(
    root: &Path,
    display: &str,
    window: &str,
    text: &str,
) -> anyhow::Result<()> {
    let environment = sandbox_environment(&caller_environment(), root, display);
    warm_up(&environment, window)?;
    xdotool(
        &environment,
        &[
            "type",
            "--delay",
            &TYPE_DELAY.as_millis().to_string(),
            "--",
            text,
        ],
        paced_timeout(text.chars().count(), TYPE_DELAY),
    )
}

// No `--clearmodifiers`: it would release the keys that `hold` keeps down.
#[mutants::skip]
pub(crate) fn key(
    root: &Path,
    display: &str,
    window: &str,
    key: &str,
    modifiers: &[String],
) -> anyhow::Result<()> {
    let environment = sandbox_environment(&caller_environment(), root, display);
    warm_up(&environment, window)?;
    xdotool(
        &environment,
        &["key", &key_combo(key, modifiers)],
        TOOL_TIMEOUT,
    )
}

pub(crate) fn hold(
    root: &Path,
    display: &str,
    window: &str,
    keys: &[String],
) -> anyhow::Result<()> {
    let environment = sandbox_environment(&caller_environment(), root, display);
    super::set_focus(&environment, window)?;

    xdotool(&environment, &key_arguments("keydown", keys), TOOL_TIMEOUT)
}

pub(crate) fn release(
    root: &Path,
    display: &str,
    window: &str,
    keys: &[String],
) -> anyhow::Result<()> {
    let environment = sandbox_environment(&caller_environment(), root, display);
    let released = xdotool(&environment, &key_arguments("keyup", keys), TOOL_TIMEOUT);
    let focused = super::set_focus(&environment, window);

    released.and(focused)
}

#[mutants::skip]
fn point_at(
    environment: &[(String, String)],
    window: &str,
    point: (i32, i32),
) -> anyhow::Result<()> {
    warm_up(environment, window)?;
    move_pointer(environment, window, point)?;
    std::thread::sleep(CLICK_SETTLE);

    Ok(())
}

#[mutants::skip]
fn move_pointer(
    environment: &[(String, String)],
    window: &str,
    (x, y): (i32, i32),
) -> anyhow::Result<()> {
    xdotool(
        environment,
        &[
            "mousemove",
            "--window",
            window,
            &x.to_string(),
            &y.to_string(),
        ],
        TOOL_TIMEOUT,
    )
}

fn interpolate(from: i32, to: i32, step: u32) -> i32 {
    let distance = i64::from(to) - i64::from(from);
    let moved = distance * i64::from(step) / i64::from(DRAG_STEPS);

    i32::try_from(i64::from(from) + moved).expect("a point between two i32 values fits in i32")
}

// A fixed timeout would kill xdotool partway through long paced input, after it sent only a prefix.
fn paced_timeout(steps: usize, delay: Duration) -> Duration {
    let steps = u32::try_from(steps).unwrap_or(u32::MAX);

    TOOL_TIMEOUT.saturating_add(delay.saturating_mul(steps))
}

fn scroll_arguments(notches: i32) -> Vec<String> {
    let button = if notches > 0 { WHEEL_DOWN } else { WHEEL_UP };

    vec![
        "click".to_string(),
        "--repeat".to_string(),
        notches.unsigned_abs().to_string(),
        "--delay".to_string(),
        NOTCH_DELAY.as_millis().to_string(),
        button.to_string(),
    ]
}

fn key_combo(key: &str, modifiers: &[String]) -> String {
    let mut combo = modifiers.to_vec();
    combo.push(key.to_string());

    combo.join("+")
}

fn key_arguments(action: &str, keys: &[String]) -> Vec<String> {
    let mut arguments = vec![action.to_string()];
    arguments.extend_from_slice(keys);

    arguments
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;

    use super::*;

    use crate::input::WindowError;
    use crate::test_support::strings;

    // A fake xdotool that records every call, answers the focus request with `windowfocus` and the
    // focus query with `getwindowfocus`.
    fn fake_xdotool(
        windowfocus: &str,
        getwindowfocus: &str,
    ) -> (tempfile::TempDir, Vec<(String, String)>, PathBuf) {
        let folder = crate::test_support::temporary_directory();
        let calls = folder.path().join("calls");
        let script = folder.path().join("xdotool");
        let script_text = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$1\" in\n  windowfocus) {windowfocus} ;;\n  getwindowfocus) {getwindowfocus}\nesac\nexit 0\n",
            calls.display()
        );
        fs::write(&script, script_text).expect("write the fake xdotool");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("make the fake xdotool runnable");
        let environment = vec![(
            "PATH".to_string(),
            folder.path().to_string_lossy().into_owned(),
        )];

        (folder, environment, calls)
    }

    #[test]
    fn warm_up_refuses_a_focus_that_stayed_on_another_window() {
        crate::test_support::in_test_namespace!();

        let (_folder, environment, calls) = fake_xdotool("exit 0", "printf '999\\n'");

        let error = warm_up(&environment, "123").expect_err("the focus stayed elsewhere");

        assert_eq!(
            error.downcast_ref::<WindowError>(),
            Some(&WindowError::FocusRefused {
                window: "123".to_string(),
                holder: "999".to_string(),
            })
        );
        assert_eq!(
            fs::read_to_string(&calls).expect("read the calls"),
            "windowfocus 123\ngetwindowfocus -f\n",
            "a refused focus sends no pointer, key or button input"
        );
    }

    #[test]
    fn warm_up_names_a_failed_focus_query() {
        crate::test_support::in_test_namespace!();

        let (_folder, environment, calls) = fake_xdotool(
            "exit 0",
            "printf 'Failed creating new xdo instance.\\n' >&2; exit 1",
        );

        let error = warm_up(&environment, "123").expect_err("the focus query fails");

        assert_eq!(
            format!("{error:#}"),
            "xdotool getwindowfocus failed: Failed creating new xdo instance."
        );
        assert_eq!(
            fs::read_to_string(&calls).expect("read the calls"),
            "windowfocus 123\ngetwindowfocus -f\n",
            "a failed focus query sends no pointer, key or button input"
        );
    }

    #[test]
    fn warm_up_names_a_failed_focus_request() {
        crate::test_support::in_test_namespace!();

        let (_folder, environment, calls) = fake_xdotool(
            "printf 'Failed to focus the window.\\n' >&2; exit 1",
            "exit 0",
        );

        let error = warm_up(&environment, "123").expect_err("the focus request fails");

        assert_eq!(
            format!("{error:#}"),
            "xdotool windowfocus 123 failed: Failed to focus the window."
        );
        assert_eq!(
            fs::read_to_string(&calls).expect("read the calls"),
            "windowfocus 123\n",
            "a failed focus request makes no focus query and sends no input"
        );
    }

    // hold and release build the sandbox environment themselves, which puts the root's bin folder
    // first on PATH, so the fake xdotool must live there.
    fn fake_root_xdotool(windowfocus: &str, getwindowfocus: &str) -> (tempfile::TempDir, PathBuf) {
        let root = crate::test_support::temporary_sandbox_root("send-keys");
        let bin = root.path().join("bin");
        fs::create_dir(&bin).expect("create the bin folder");
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o700))
            .expect("make the bin folder private");
        let calls = root.path().join("calls");
        let script = bin.join("xdotool");
        let script_text = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$1\" in\n  windowfocus) {windowfocus} ;;\n  getwindowfocus) {getwindowfocus}\nesac\nexit 0\n",
            calls.display()
        );
        fs::write(&script, script_text).expect("write the fake xdotool");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("make the fake xdotool runnable");

        (root, calls)
    }

    #[test]
    fn hold_sends_the_keydown_only_after_a_taken_focus() {
        crate::test_support::in_test_namespace!();

        let (root, calls) = fake_root_xdotool("exit 0", "printf '123\\n'");

        hold(root.path(), ":7", "123", &strings(&["Shift_L"])).expect("the focus is taken");

        assert_eq!(
            fs::read_to_string(&calls).expect("read the calls"),
            "windowfocus 123\ngetwindowfocus -f\nkeydown Shift_L\n"
        );
    }

    #[test]
    fn hold_refuses_a_focus_that_stayed_elsewhere_without_pressing_a_key() {
        crate::test_support::in_test_namespace!();

        let (root, calls) = fake_root_xdotool("exit 0", "printf '999\\n'");

        let error = hold(root.path(), ":7", "123", &strings(&["Shift_L"]))
            .expect_err("the focus stayed elsewhere");

        assert_eq!(
            error.downcast_ref::<WindowError>(),
            Some(&WindowError::FocusRefused {
                window: "123".to_string(),
                holder: "999".to_string(),
            })
        );
        assert_eq!(
            fs::read_to_string(&calls).expect("read the calls"),
            "windowfocus 123\ngetwindowfocus -f\n",
            "a refused focus presses no key"
        );
    }

    #[test]
    fn release_sends_the_keyup_after_a_taken_focus() {
        crate::test_support::in_test_namespace!();

        let (root, calls) = fake_root_xdotool("exit 0", "printf '123\\n'");

        release(root.path(), ":7", "123", &strings(&["Shift_L"])).expect("the focus is taken");

        assert_eq!(
            fs::read_to_string(&calls).expect("read the calls"),
            "keyup Shift_L\nwindowfocus 123\ngetwindowfocus -f\n"
        );
    }

    #[test]
    fn release_sends_the_keyup_when_the_focus_is_refused() {
        crate::test_support::in_test_namespace!();

        let (root, calls) = fake_root_xdotool("exit 0", "printf '999\\n'");

        let error = release(root.path(), ":7", "123", &strings(&["Shift_L"]))
            .expect_err("the focus stayed elsewhere");

        assert_eq!(
            error.downcast_ref::<WindowError>(),
            Some(&WindowError::FocusRefused {
                window: "123".to_string(),
                holder: "999".to_string(),
            })
        );
        assert_eq!(
            fs::read_to_string(&calls).expect("read the calls"),
            "keyup Shift_L\nwindowfocus 123\ngetwindowfocus -f\n",
            "the keyup goes out before the refused focus is reported"
        );
    }

    #[test]
    fn interpolate_hits_both_ends_and_the_middle() {
        assert_eq!(interpolate(10, 20, 0), 10);
        assert_eq!(interpolate(10, 20, DRAG_STEPS), 20);
        assert_eq!(interpolate(20, 10, DRAG_STEPS / 2), 15);
        assert_eq!(interpolate(i32::MIN, i32::MAX, DRAG_STEPS), i32::MAX);
    }

    #[test]
    fn scroll_presses_one_wheel_button_per_notch() {
        let cases = [
            (
                3,
                strings(&["click", "--repeat", "3", "--delay", "50", "5"]),
            ),
            (
                1,
                strings(&["click", "--repeat", "1", "--delay", "50", "5"]),
            ),
            (
                -2,
                strings(&["click", "--repeat", "2", "--delay", "50", "4"]),
            ),
        ];

        for (notches, expected) in cases {
            assert_eq!(scroll_arguments(notches), expected, "{notches}");
        }
    }

    #[test]
    fn paced_timeout_adds_the_delay_of_every_step_to_the_tool_timeout() {
        let cases = [
            (0, TOOL_TIMEOUT),
            (1_000, TOOL_TIMEOUT + Duration::from_secs(30)),
            (usize::MAX, TOOL_TIMEOUT + TYPE_DELAY * u32::MAX),
        ];

        for (steps, expected) in cases {
            assert_eq!(paced_timeout(steps, TYPE_DELAY), expected, "{steps}");
        }
    }

    #[test]
    fn key_combo_puts_the_modifiers_before_the_key() {
        assert_eq!(key_combo("Return", &[]), "Return");
        assert_eq!(key_combo("s", &strings(&["ctrl", "shift"])), "ctrl+shift+s");
    }

    #[test]
    fn hold_and_release_name_every_key() {
        let keys = strings(&["Shift_L", "Control_L"]);

        assert_eq!(
            key_arguments("keydown", &keys),
            strings(&["keydown", "Shift_L", "Control_L"])
        );
    }
}
