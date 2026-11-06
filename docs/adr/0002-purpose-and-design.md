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
headless Weston and Xwayland display. Each project installs it with `cargo install --git` and calls
it, instead of keeping its own copy.

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

Input is xdotool XTest, and a shot is ImageMagick `import` of the private window, so a shot's pixels
and an input coordinate share one scale.

### What each project owns

Each project has a small config file. It names the launch command, how ruta detects that the window
is ready, the window title, the window size, and extra environment. A window is ready when the app
prints a JSON ready line on stdout, or when a given command exits 0.

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

### The developer's real display

Open the window on the desktop that is already running. Rejected because the window would take over
the screen and the input, and the result would depend on the desktop's scale and other windows.
