# QA guide

ruta ships one command-line tool, `ruta`. It opens an app in a throwaway window on a private
headless display, takes shots of the window, and sends it input. There is no web app, TUI, or API.
A worker sees ruta's work through command output, shot files, the app log, and the files under
`/var/tmp/ruta-<name>`.

## Choose sections

Most modules serve every command, so choose by behavior, not by path.

| Section | Test it when the change touches |
| ------- | ------------------------------- |
| Refusals | Command parsing, the usage text, the `guide` text, `ruta.toml` reading, error text, exit codes, the root safety checks |
| Running sandbox | Shots, any input command, `status`, the check that a sandbox is up, `log`, `opened`, the stub openers, the environment that tools and the app get, tool timeouts |

Cross-effects:

- The environment, root, process, and tool modules serve every command. Assign both sections.
- The config is read before every command that parses, `log` and `down` included. Only `guide`
  reads no config. Assign both sections.
- The `guide` text quotes the output lines of `up`, `down`, and `status`, and it lists the
  commands. A change to `status`, to one of those lines, or to the command list falls under both
  sections.
- A change to `up`, `restart`, `down`, `capture`, `--headed`, the display, the app start and ready
  check, or process records and stopping falls under Not testable.
- `up` and `restart` end with a focus click from the input code. An input change also falls under
  Not testable.
- `capture` reads the ready line that the app start saves. A ready-line change falls under Not
  testable.
- `crates/ruta-test-app` is the fixture. A change there changes what every section reads in the log.
- The usage text, the `guide` text, the output lines, the `status` line included, the record and
  capture file names, and the `ruta.toml` keys are a contract. Other repositories call a tagged release. Say so in the brief when one changes.

- A change to `xtask` or `.github/` needs no `qa` launch. `full_check` and CI cover them.
- When the impact on a section is unclear, include the section.

## Before a launch

[The setup](qa/setup.md) holds the fixture file and the full steps.

After the build, run `target/debug/ruta guide` in the worktree. Follow its parts "Every session"
and "The session that starts the sandbox". The steps below add what is specific to testing ruta
itself.

The manager starts:

- The build. Start: `cargo build --locked -p ruta -p ruta-test-app` in the worktree. Up:
  `target/debug/ruta` and `target/debug/ruta-test-app` exist. Stop: nothing.
- The fixture folder. Start: `PROJECT=$(mktemp -d /tmp/qa-ruta-XXXXXX)`, then write
  `$PROJECT/ruta.toml` from the setup. Up: `$PROJECT/ruta.toml` exists. Stop: `rm -r $PROJECT`.
- The sandbox, for Running sandbox only. Pick a `$NAME` that starts with `qa-`, is at most 24
  bytes, and has no `/var/tmp/ruta-$NAME`, `/var/tmp/ruta-$NAME-idle`, or
  `/var/tmp/ruta-$NAME-gone`. Start:
  `cd $PROJECT && RUTA_NAME=$NAME $RUTA up`. Up: it prints the `up` line that the guide gives, with
  `$NAME` as the name. Stop: `cd $PROJECT && RUTA_NAME=$NAME $RUTA down` prints the `down` line
  that the guide gives, and `/var/tmp/ruta-$NAME` is gone.
- The sandbox whose app exited, for Running sandbox only. Start:
  `cd $PROJECT && RUTA_NAME=$NAME-gone $RUTA up`, then
  `kill $(cut -d' ' -f1 /var/tmp/ruta-$NAME-gone/pid)`. Up:
  `cd $PROJECT && RUTA_NAME=$NAME-gone $RUTA status` exits 1 and prints
  `ruta $NAME-gone is not up`. Stop: `cd $PROJECT && RUTA_NAME=$NAME-gone $RUTA down` prints the
  `down` line, and `/var/tmp/ruta-$NAME-gone` is gone.

Ask the user:

- The end-to-end tests: their sandbox names are the same in every worktree.

The brief must give:

- `$RUTA` and `$APP`: the absolute paths of `target/debug/ruta` and `target/debug/ruta-test-app`
  in this worktree, and the commit they were built from.
- `$PROJECT` and `$NAME`, and for Running sandbox the output of the manager's `up`.
- That the worker may also use the name `$NAME-idle`, and for Running sandbox `$NAME-gone`.
- That data writes are allowed in `$PROJECT` and `/var/tmp/ruta-$NAME`, and not in
  `/var/tmp/ruta-$NAME-gone`.

## Rules for every section

Run `$RUTA guide` first. Follow its parts "Every session" and "A session that did not start the
sandbox". The rules below add what is specific to testing ruta itself.

Run every command as `cd $PROJECT && RUTA_NAME=<name> $RUTA <command>`.

A command that a section or a recipe here gives wins over the guide:

- The Refusals section runs commands under `$NAME-idle` without the `status` check, because
  nothing is up under that name. The same holds for `$NAME-gone` in the Running sandbox section.
- Some refusals need another folder or a bad name. Use them only where a row or a step names
  them.
- `guide` is not in the guide's `You may run` list. Run it.
- `shot` without a file writes `/var/tmp/ruta-$NAME/shot.png`. Run it, because that default path
  is under test. Write every other shot under `$PROJECT`.

Data writes: a shot writes a file, input changes the app, and a config refusal edits
`$PROJECT/ruta.toml`.

Prove that the binaries serve this worktree:

1. Never run a bare `ruta`. That is the installed release, not this worktree.
2. In the worktree, run
   `find crates/ruta Cargo.toml Cargo.lock -newer $RUTA \( -name '*.rs' -o -name 'Cargo.*' \)`. A file
   outside `crates/ruta/tests` in the output means a stale binary. For `$APP`, run
   `find crates/ruta-test-app Cargo.toml Cargo.lock -newer $APP \( -name '*.rs' -o -name 'Cargo.*' \)`.
   Any file in the output means a stale binary. Cargo does not build the app again when only
   `crates/ruta` changes.
3. `ruta` has no `--version`. When a binary is missing or stale, stop and ask the parent to build
   it.

Never:

- Test a refusal of `up`, `restart`, `down`, or `capture`. The guide's rule covers their refusals
  too.
- Remove a `/var/tmp/ruta-*` folder or stop a `weston` or `Xwayland` process by hand. Other tools run
  their own Weston displays. Report leftovers.
- Run `cargo`. Every `cargo` command here builds.

## Refusals

### Reach it

Needs the binary and `$PROJECT`, and nothing running. Run the binary proof above. Then:

1. `command -v xdotool import` prints two paths. Without these tools, `shot` and the input
   commands report a missing tool instead of the refusal.
2. `ls -d /var/tmp/ruta-$NAME-idle` fails.
3. When a step gives another result, stop and ask the parent.

### Use it safely

Use the name `$NAME-idle`, which has no sandbox, so a mistake cannot reach the parent's sandbox.

### What to check

Every refusal exits 1, prints one message on stderr, prints nothing on stdout, and leaves
`/var/tmp/ruta-$NAME-idle` absent. Two commands are no refusal: `opened` with nothing up exits 0
and prints nothing, and `guide` exits 0 and prints its three parts.

### Recipes

- [Refusals](qa/refusals.md): the commands and the exact messages. Use it for every run of this
  section.

## Running sandbox

### Reach it

Needs a sandbox that the parent started in `$PROJECT` under `$NAME`, and a second one under
`$NAME-gone` whose app the parent stopped.

1. Run the binary proof above. `command -v xdotool import` prints two paths. Without `xdotool`,
   `status` reports a missing tool for a sandbox whose display runs.
2. `status` exits 0 and prints `ruta $NAME is up and serves $PROJECT`. That is the `status` line
   of the guide, with the name and the folder filled in.
3. `log` holds the line `ruta-test-app: running`.
4. When a step fails, stop and ask the parent to start the sandbox.

### What to check

- `status` from another project folder is a refusal. Run `mkdir $PROJECT/other`, then
  `cp $PROJECT/ruta.toml $PROJECT/other/`. Then `cd $PROJECT/other && RUTA_NAME=$NAME $RUTA status`
  exits 1, prints nothing on stdout, and prints `ruta $NAME serves $PROJECT, not $PROJECT/other`
  on stderr. `status` in `$PROJECT` still exits 0 afterwards.
- The app exited: run these under `RUTA_NAME=$NAME-gone`. `status` exits 1, prints nothing on
  stdout, and prints `ruta $NAME-gone is not up` on stderr. `shot` and `click 1 2` exit 1 and
  print `ruta $NAME-gone is not up; run 'ruta up' first` on stderr. `log` still exits 0 and prints
  the app log. `/var/tmp/ruta-$NAME-gone/shot.png` does not exist afterwards.
- `shot` prints `/var/tmp/ruta-$NAME/shot.png`, and `file` reports a 640 x 480 PNG. `shot <file>`
  prints and writes that file instead.
- `log` prints the app log byte for byte:
  `RUTA_NAME=$NAME $RUTA log | cmp - /var/tmp/ruta-$NAME/app.log` prints nothing.
- A `click` logs a `button-press` with the `x` and `y` of the command.
- `opened` prints the links that the stub openers logged. Run
  `/var/tmp/ruta-$NAME/bin/xdg-open https://example.com/qa`, then `opened` prints that link, and
  so does `cat /var/tmp/ruta-$NAME/opened`.

### Recipes

- [Input](qa/input.md): the eight input commands, when one of them or their pacing changes.

## Not testable

- `up`, `restart`, `down`, and `capture`: the guide keeps them from a worker.
- `--headed`: it shows a window on the user's real screen, so only the user can judge it.
- `status` on a sandbox with no folder record: only a ruta from before `status` starts one.
- "The app opens a link": the test app never opens a link itself.
- "A shot pixel is the pixel that input reaches": the test app draws nothing, so a shot shows no
  target.
- "No link opens on the real desktop": a worker cannot watch the user's desktop.

Report each of these as not verified.

The end-to-end tests cover the four commands, and `cargo xtask check` leaves them out.
[The development guide](../docs/development.md) gives their commands. The manager reports the
output of its own `up` and `down`, as Before a launch states it.
