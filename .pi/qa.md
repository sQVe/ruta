# QA guide

ruta ships one command-line tool, `ruta`. It opens an app in a throwaway window on a private
headless display, takes shots of the window, and sends it input. There is no web app, TUI, or API.
A worker sees ruta's work through command output, shot files, the app log, and the files under
`/var/tmp/ruta-<name>`.

## Choose sections

Most modules serve every command, so choose by behavior, not by path.

| Section         | Test it when the change touches |
| --------------- | ------------------------------- |
| Refusals        | Command parsing, `ruta.toml` reading, the root safety checks, the usage text, the `ruta guide` text, error text, exit codes |
| Running sandbox | Shots, any input command, `status`, `log`, `opened`, the check that a sandbox is up, the stub openers, tool timeouts, the environment that tools and the app get |

Cross-effects:

- The environment, root, process, and tool modules serve every command. Assign both sections.
- The config is read before every command that parses, `log` and `down` included. Only `guide`
  and `--version` read no config. Assign both sections.
- The `ruta guide` text quotes the output lines of `up`, `down`, and `status`, and it lists the
  commands. A change to `status`, to one of those lines, or to the command list falls under both
  sections.
- A change to one of these falls under Not testable:
  - `up`, `restart`, `down`, `capture`, `--headed`, or the display.
  - The app start and ready check, or process records and stopping.
  - Input, next to its section. `up` and `restart` end with a focus click from the input code.
  - The ready line. `capture` reads the ready line that the app start saves.
- `crates/ruta-test-app` is the fixture. A change there changes the log that every section reads.
- Other repositories call a tagged release. So the usage text and the `ruta guide` text are a
  contract. The output lines, the `status` line included, are a contract too. So are the record
  and capture file names and the `ruta.toml` keys. Say so in the brief when one changes.

- A change to `xtask` or `.github/` needs no `qa` launch. `full_check` and CI cover them.
- When the impact on a section is unclear, include the section.

## Before a launch

[The setup](qa/setup.md) holds the fixture file and the full steps, each with its check. It is
the manager's setup, not a recipe, so its parts are "Prepare" and "Undo".

The setup uses three sandbox names:

- `$NAME`: the sandbox that the manager starts and the worker tests.
- `$NAME-idle`: a name with no sandbox. The worker runs the refusals under it.
- `$NAME-gone`: a second sandbox, whose app the manager stopped.

The manager starts, by the steps of the setup:

- The build, for every section: Prepare steps 1 and 2. Nothing to stop.
- The fixture folder and the names, for every section: Prepare steps 3 to 5. Stop: Undo step 3.
- The sandbox `$NAME`, for Running sandbox only: Prepare step 6. Stop: Undo steps 1 and 2.
- The sandbox `$NAME-gone`, for Running sandbox only: Prepare step 7. Stop: Undo steps 1 and 2.

Ask the user:

- The end-to-end tests: their sandbox names are the same in every worktree. See Prepare step 8.

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

A command that a section or a recipe here gives wins over the `ruta guide` text:

- The Refusals section runs commands under `$NAME-idle` without the `status` check, because
  nothing is up under that name. The same holds for `$NAME-gone` in the Running sandbox section.
- The `You may run` list of the `ruta guide` text does not name `guide`. Run it anyway.
- The recipes name more: another folder, a bad name, and the default path of `shot`. Use each
  only where its recipe names it.

Data writes: a shot writes a file, input changes the app, and a config refusal edits
`$PROJECT/ruta.toml`.

Prove that the binaries serve this worktree with [the binary proof](qa/binaries.md). Never run a
bare `ruta`. That is the installed release, not this worktree. When a binary is missing or stale,
stop and ask the parent to build it.

Never:

- Test a refusal of `up`, `restart`, `down`, or `capture`. The rule of the `ruta guide` text
  covers their refusals too.
- Remove a `/var/tmp/ruta-*` folder or stop a `weston` or `Xwayland` process by hand. Other tools
  run their own Weston displays. Report leftovers.
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
`/var/tmp/ruta-$NAME-idle` absent. Three commands are no refusal and exit 0: `opened` with
nothing up, `guide`, and `--version`. The recipe gives their output.

### Recipes

- [Refusals](qa/refusals.md): the commands and the exact messages, for every run.

## Running sandbox

### Reach it

Needs a sandbox that the parent started in `$PROJECT` under `$NAME`, and a second one under
`$NAME-gone` whose app the parent stopped.

1. Run the binary proof above. `command -v xdotool import` prints two paths. Without `xdotool`,
   `status` reports a missing tool for a sandbox whose display runs.
2. `status` exits 0 and prints `ruta $NAME is up and serves $PROJECT`. That is the `status` line
   of the `ruta guide` text, with the name and the folder filled in.
3. `log` holds the line `ruta-test-app: running`.
4. When a step fails, stop and ask the parent to start the sandbox.

### What to check

- `status`, `shot`, `log`, `click`, and `opened` on the sandbox under `$NAME`.
- The refusal of `status` from another project folder.
- The results under `$NAME-gone`, where the app exited.

### Recipes

- [Sandbox](qa/sandbox.md): the commands and the exact results of the checks above, for every run.
- [Input](qa/input.md): the eight input commands, when one of them or their pacing changes.

## Not testable

- `up`, `restart`, `down`, and `capture`: the `ruta guide` text keeps them from a worker.
- `--headed`: it shows a window on the user's real screen, so only the user can judge it.
- `status` on a sandbox with no folder record: only an older release, from before the `status`
  command existed, makes such a sandbox.
- Whether the app can open a link: the test app never opens a link itself.
- Whether a shot pixel is the pixel that input reaches: the test app draws nothing, so a shot
  shows no target.
- Whether no link opens on the real desktop: a worker cannot watch the user's desktop.

Report each of these as not verified.

The end-to-end tests cover `up`, `restart`, `down`, and `capture`, but `cargo xtask check` does
not run them. [The development guide](../docs/development.md) gives their commands. The manager
also reports the output of its own `up` and `down`, which the setup tells it to keep.
