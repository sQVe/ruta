# ADR 0002: One shared CLI for private app windows

**Date**: 2026-11-06\
**Status**: Accepted\
**Related**: [ADR 0005 (ruta supports Linux only)](./0005-linux-only.md)

## Context

Agents and people need to look at and drive a real app window. A real window is the only way to
show focus, keyboard routing, and the rendering backend. Opening it on the developer's desktop
would take over the screen, the pointer, and the keyboard, and the result would depend on the
desktop's scale and other windows.

greppa, Koja, and Tikk all need such a window. greppa has a tested sandbox in Rust inside its
`xtask`. Koja has its own Bash script with the same display, input, and shot logic. Tikk will need
the same when its desktop app opens a window. Three copies drift apart, and a bug found in one, such
as a race between spawn and exec or a process left as a zombie, must be found again in the others.

An app is a native process, so a sandbox cannot separate it from the rest of the machine. It can
separate the display and the input instead.

## Decision

ruta is one standalone binary that gives agents and people a throwaway app window on a private
headless Weston and Xwayland display. Each project installs a tagged release with
`cargo install --git https://github.com/sQVe/ruta --tag <tag> --locked` and calls it, instead of
keeping its own copy.

### What ruta owns

ruta owns the parts every project shares:

- The private display, and a headed mode that also shows that display as one window on the user's
  Wayland session for a demo.
- A private root folder with its safety checks, and its own home and XDG folders.
- A record of each process it starts, so it stops only its own processes.
- An environment allowlist, so no command falls back to the real display.
- Stub openers that log a link instead of opening it.
- Input through xdotool, window shots, the app's log, the logged links, and teardown.
- Side-by-side sandboxes, each with its own name.
- One-shot capture with an optional app-reported crop and teardown, under a separate root so it
  leaves a running sandbox untouched.
- A record of the project folder that started a sandbox, so a session that did not start the
  sandbox can prove it serves that session's project.
- The rules for the session that starts a sandbox and for a session that only drives it. ruta
  prints them as one text, so no project keeps its own copy.

Input is xdotool XTest, and a shot is ImageMagick `import` of the private window, so a shot's pixels
and an input coordinate share one scale.

### How ruta knows its own processes

ruta identifies a process it started by its PID and start time, plus the `RUTA_ROOT` marker in its
environment, not by its command line. A launch script may `exec` the app, which replaces the command
line but keeps the PID, the start time, and the environment. ruta records the app once it holds the
marker.

The app leads its own process group, and `down` stops that group. It stops the recorded leader by
its PID and start time alone, because the leader may `exec` into an environment without the marker.
It stops another member of the group only when that member holds the marker.

A process can leave the group with `setsid`, such as a daemon that the app or a ready check starts.
So after the group stops, and before the root is removed, ruta also stops every process of the
current user whose environment holds this root's marker and that started after the root was made.
ruta records the root's creation time in the clock that process start times use. A marker is only
an environment value that any process can copy, so the start time keeps the sweep to processes
this sandbox could have started, and ruta never stops itself or one of its own ancestors. It checks
the start time again through a pidfd before each signal. A process with another root's marker, with
none, or from before the root keeps running.

ruta's own tests that send signals run in a user and PID namespace with its own `/proc`, so a bug
in this selection, or a mutant during mutation testing, can reach only test processes.

### When a sandbox is up

A sandbox is up when its recorded window still exists on the private display, not when the recorded
app process lives. A launch command can start the app as a child and then exit, and the child keeps
a working window. A shot and an input need the window, so the window is what `status`, a shot, and
an input ask for. When ruta cannot ask the display, it reports that failure and does not call the
sandbox up.

### What each project owns

Each project has a small `ruta.toml`. It names the launch command, how ruta detects that the window
is ready, the window title, the window size, and extra environment. A window is ready when the app
prints a JSON ready line on stdout, or when a given command exits 0. ruta saves the JSON ready line
in the sandbox, so later commands and project scripts can read what the app reported.

Commands that only one app needs stay in that project and call ruta.

## Consequences

### Positive

- A bug fixed in ruta is fixed for every project that uses it.
- Agents and people see the real window without touching the real screen or input.
- A shot's coordinates match the input coordinates, so a click can be read from a shot.
- Any project can call ruta, whatever language its own tooling uses.

### Negative

- Each project depends on an installed ruta, and a change to ruta's commands or config can break
  every caller at once.
- The sandbox covers an app's X11 backend under Xwayland, not a native Wayland backend.
- The sandbox is not a security boundary against a process that already has host shell access.
- During a headed demo, the user's own mouse over the window mixes with the agent's input.

## Alternatives considered

### A copy in each project

Keep greppa's Rust sandbox, Koja's Bash script, and a new copy for Tikk. Rejected because the
copies drift, and each bug must be found and fixed once per copy.

### A Rust library that each project links

Publish the sandbox as a crate that each project's own task runner calls. Rejected because Koja's
tooling is not Rust, and agents need a command they can run directly in any project.

### Native Wayland with Sway

Run the app on a private headless Sway and drive it with wtype, a virtual-pointer client, and grim.
Rejected because Weston advertises no virtual keyboard, virtual pointer, or wlr-screencopy
protocol. Switching the compositor and the input and capture tools together adds risk without a
shown benefit.

### Matching a process by its command line

Record the launch command and check that the process still names it. Rejected because a launch
script that `exec`s the app replaces its command line, so a valid start fails at random, depending
on when ruta reads it.

### Calling a sandbox up while its recorded app process lives

Check the recorded app process by its PID and start time. Rejected because that process can be a
launcher that exits while its child keeps a working window, so a usable sandbox would be reported
as not up. A live process also does not prove that a window exists.

### The developer's real display

Open the window on the desktop that is already running. Rejected because the window would take over
the screen and the input, and the result would depend on the desktop's scale and other windows.
