# ADR 0005: ruta supports Linux only

**Date**: 2026-11-06\
**Status**: Accepted\
**Related**: [ADR 0002 (One shared CLI for private app windows)](./0002-purpose-and-design.md)

## Context

ruta's whole job is a private display: a headless Weston compositor with Xwayland, input through
xdotool XTest, and shots through ImageMagick `import`. Weston is a Linux compositor. macOS
and Windows have no headless compositor that an app can open a window on and that a tool can drive
with the same input and capture calls.

Input also depends on the versions of Weston and Xwayland. Weston 14 with Xwayland 24.1.6 and
older, as in Ubuntu 24.04 and Debian 13, abort Xwayland on the first `xdotool` input. The supported
floor is Weston 15 with Xwayland 24.1.14, as Arch Linux ships. Starting a sandbox needs Weston 15,
because a sandbox window must hold the X input focus and only Weston 15 accepts
`--no-xwm-decorations`.

## Decision

ruta supports Linux only. It does not support macOS or Windows.

- CI builds and tests ruta on Linux. There is no macOS or Windows job.
- ruta's crates hold no `cfg(windows)` or `cfg(target_os = "macos")` code.
- A report about macOS or Windows behavior is out of scope.
- A sandbox starts only on Weston 15 or newer. ruta refuses an older weston before it starts a
  display. Input also needs Xwayland 24.1.14 or newer; ruta checks that Xwayland is present, not
  its version. A test that starts a sandbox runs on that supported floor, such as an Arch Linux
  container, not on the Ubuntu runner image.

## Consequences

### Positive

- ruta has one display stack and one set of input and capture tools to get right.
- Each pull request runs one build and test job, not one per platform.

### Negative

- People on macOS and Windows cannot use ruta, so a project that supports those platforms tests
  its window there by other means.
- Current Ubuntu and Debian releases cannot start a sandbox at all, so every test that starts one
  needs a newer distribution than the default CI runner.
- ruta checks the weston version but not the Xwayland version, so Weston 15 with an older Xwayland
  starts a sandbox and can then fail on input.
- A newer Weston or Xwayland can change behavior that ruta relies on, and a test must catch it.

## Alternatives considered

### Linux and macOS

Support macOS with its own window, input, and capture tools. Rejected because macOS has no private
headless display like Weston, so a sandbox there would need a separate design with its own input
and capture tools. Projects that support macOS can still use ruta for their Linux build.

### Older Weston and Xwayland for input

Support the Weston and Xwayland in Ubuntu 24.04 and Debian 13, so CI can use the default runner.
Rejected because those versions abort Xwayland on the first `xdotool` input, which no change in ruta
can fix.

### Weston 14 for shots only

Keep the older Weston and Xwayland for shots and refuse them only for input. Rejected because it
needs a second focus policy for the older stack, and CI cannot run that policy: the shot path would
start weston without the focus guarantee, so the input commands and their tests would need a branch
for each stack.
