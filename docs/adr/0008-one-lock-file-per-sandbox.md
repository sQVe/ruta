# ADR 0008: One lock file per sandbox serializes its commands

**Date**: 2026-11-27\
**Status**: Accepted\
**Related**: [ADR 0002 (One shared CLI for private app windows)](./0002-purpose-and-design.md)

## Context

Every command for one sandbox name works on one root, `/var/tmp/ruta-<name>`. The root holds the
records of the app, Weston, and Xwayland processes, the display, the logs, and the project folder.
A start writes these records one by one over many seconds, because it stops the old window, starts
the display and the app, waits for the window, and only then records the folder.

Two commands for one name can run at once, because agents and scripts call ruta in parallel. Two
starts then stop each other's apps and overwrite each other's process and display records, so ruta
can lose track of a process it started. A `down` during a start removes the root under it. A
`status` during a start can pair the new window with the folder record of the previous start,
because the start writes the folder record after the window is up.

A command can also be killed at any point, with no cleanup. So any guard must free itself when its
holder dies, and a killed start must not leave records that pair one start's window with another
start's folder.

The root itself cannot hold the guard. `down` and a finished capture remove the root, and a capture
needs a root that does not exist yet.

## Decision

Each command that uses a sandbox's processes takes an `flock` on `/var/tmp/ruta-<name>.lock`, a
file beside the root that ruta never removes. A command that changes the sandbox takes the lock
exclusively, a command that uses the window takes it shared, and a command that cannot take the
lock at once fails as busy.

### The lock

- The lock is an `flock` on an open file, taken without waiting. The kernel drops it when the
  holder's last descriptor closes, even after `SIGKILL`. So a lock never outlives its process, and
  ruta never reads, judges, or breaks another process's lock.
- The lock file is opened close-on-exec. Weston, Xwayland, the app, and every tool ruta starts
  would otherwise inherit it and keep the sandbox locked after ruta exits.
- The lock file lives beside the root and stays there. A lock inside the root, or on the root
  folder, can be removed while another process waits on it. A third process then locks a new file
  at the same path, and two commands hold the lock at once.
- A sandbox name holds only lowercase letters, digits, and hyphens, so the lock path never equals
  a root path.
- The lock file gets the root's safety rules. ruta opens it without following a symlink and creates
  it with mode 600. It refuses a lock file that is not a regular file or that another user owns,
  and the error names the path and the reason.
- A command takes the lock before its first read of the root and holds it until the command ends.

### Which commands take the lock

- Exclusive: `up`, `restart`, and `down`. `capture` takes the exclusive lock of its own root,
  `<name>-capture`, and never takes the lock of `<name>`, so it leaves a running sandbox untouched.
- Shared: `status`, `shot`, `move`, `click`, `drag`, `type`, `key`, `scroll`, `hold`, and
  `release`. They read the display and window records and act on them, so they must see the state
  that a finished command left. Shared commands do not block each other.
- None: `log` and `opened`. Each one reads one file, opened safely, and the app log is most useful
  while a slow start runs.

### A busy sandbox

A command that finds the lock held fails at once with exit status 1 and this message, where
`<name>` is the name of the root it tried to lock:

```text
ruta <name> is busy: another ruta command is using it. Run this command again after that one ends.
```

ruta does not wait or retry. A start can take minutes before its timeouts end, and a call that
waits that long outlasts the time limit of an agent's tool call. The caller decides whether to
retry.

### One window pairs with one folder

The folder record exists only while it names the folder that started the current window. A start
removes the folder record before it stops the old window, and publishes the new record after the
new window is up. A start killed between those steps leaves no folder record, and `status` reports
that the sandbox has no folder record. `status` reads the display, the window, and the folder record
under one shared lock, so it never sees a start halfway through.

## Consequences

### Positive

- Two commands never write one sandbox's records at the same time, so ruta keeps the identity of
  every process it starts.
- `status` names a folder only for the window that folder started.
- A killed command never leaves a lock behind, so no command needs a manual unlock step.

### Negative

- A caller that runs commands in parallel on one sandbox must handle the busy error and retry.
- `down` cannot interrupt a hung start. It fails as busy until the start's own timeouts end.
- A long `type` or `drag` blocks `restart` and `down` until it ends.
- Each sandbox name leaves an empty lock file in `/var/tmp`, which stays until the system cleans
  that folder.
- Another user can create the lock file first and block that name, as they can with the root today.
- `log` can show a log that a running start is about to replace.

## Alternatives considered

### Wait for the lock with a bound

The second command waits until the first ends, up to a bound longer than the slowest start, then
fails as busy. Rejected because that bound is minutes, longer than an agent's tool call allows, and
a fast failure lets the caller choose whether to wait.

### Waiting for changes and failing reads

`up`, `restart`, and `down` wait with the bound, and the commands that use the window fail at once.
Rejected because it gives the lock two behaviors to document and test, and the changes still wait
for minutes.

### A lock inside the root or on the root folder

Lock a file in the root, or the root folder itself. Rejected because `down` and capture remove the
root while waiters may hold the old file, so two commands can hold the lock at once. Capture also
needs the lock before its root exists.

### A PID file with a staleness check

Write the holder's PID and start time, and take over a lock whose holder is gone. Rejected because
the check and the takeover race with another command doing the same, and `flock` already ends with
its holder.

### An `fcntl` record lock

Use a POSIX record lock instead of `flock`. Rejected because the process loses it when any of its
descriptors for that file closes, which a later change can do by accident.

### A lock in `XDG_RUNTIME_DIR`

Keep the lock in the user's runtime folder, which other users cannot write. Rejected because that
variable can be unset where ruta runs. A lock in the root's folder also sees the same mounts as
the root, so a process that sees a root always sees its lock.

### No lock

Keep each record write atomic and accept that commands overlap. Rejected because a start is a
sequence of steps across several records and processes, and atomic writes cannot keep two
sequences apart.
