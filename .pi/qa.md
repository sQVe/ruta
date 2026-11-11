# QA guide

ruta ships one command-line tool, `ruta`. It opens an app in a throwaway window on a private
headless display, takes shots of the window, and sends it input. There is no web app, TUI, or API.
A worker sees ruta's work through command output, shot files, the app log, and the files under
`/var/tmp/ruta-<name>`.

## Choose sections

Most modules serve every command, so choose by behavior, not by path.

| Section | Test it when the change touches |
| ------- | ------------------------------- |
| Refusals | Command parsing, the usage text, `ruta.toml` reading, error text, exit codes, the root safety checks |
| Running sandbox | Shots, any input command, `log`, `opened`, the stub openers, the environment that tools and the app get, tool timeouts |
| Parent only | `up`, `restart`, `down`, `capture`, `--headed`, the display, the app start and ready check, process records and stopping |

Cross-effects:

- The environment, root, process, and tool modules serve every command. Assign all sections.
- The config is read before every command, `log` and `down` included. Assign all sections.
- `up` and `restart` end with a focus click from the input code. An input change needs Parent only
  too.
- `capture` reads the ready line that the app start saves. A ready-line change needs Parent only.
- `crates/ruta-test-app` is the fixture. A change there changes what every section reads in the log.
- The usage text, the output lines, the record and capture file names, and the `ruta.toml` keys
  are a contract. Other repositories call a tagged release. Say so in the brief when one changes.
- `xtask` and `.github/` have no section. `full_check` and CI cover them.

When the impact on a section is unclear, include the section.

## Before a launch

The parent follows [the setup](qa/setup.md). The brief must then give:

- `$RUTA` and `$APP`: the absolute paths of `target/debug/ruta` and `target/debug/ruta-test-app`
  in this worktree, and the commit they were built from.
- `$PROJECT`: the fixture folder outside every repository.
- `$NAME`: the sandbox name. It starts with `qa-` and is at most 24 bytes.
- For Running sandbox: the output of the parent's `up`.
- That data writes are allowed in `$PROJECT` and `/var/tmp/ruta-$NAME`. Shots write files, and
  input changes the app.

Run every command as `cd $PROJECT && RUTA_NAME=<name> $RUTA <command>`.

Tell that the binaries serve this worktree:

1. Never run a bare `ruta`. That is the installed release, not this worktree.
2. In the worktree, run
   `find crates Cargo.toml Cargo.lock -newer $RUTA \( -name '*.rs' -o -name 'Cargo.*' \)`, and
   again with `$APP`. A file outside `crates/ruta/tests` in the output means a stale binary.
3. `ruta` has no `--version`. When a binary is missing or stale, ask the parent to build it.

Never:

- Run `up`, `restart`, `down`, or `capture`, even to test a refusal. Each can start or stop
  processes.
- Run a command without `RUTA_NAME`, or from a folder inside another project. ruta then uses the
  nearest `ruta.toml` and its name.
- Touch a sandbox with another name. `ls -d /var/tmp/ruta-*/` lists the roots, and other agents use
  them.
- Remove a `/var/tmp/ruta-*` folder or stop a `weston` or `Xwayland` process by hand. Other tools
  run their own Weston displays. Report leftovers.
- Run `cargo`. Every `cargo` command here builds.
- Write a shot outside `$PROJECT` and the root.

## Refusals

Needs the binary and `$PROJECT`. Use the name `$NAME-idle`, which has no sandbox, so a mistake
cannot reach the parent's sandbox.

Every refusal exits 1, prints one message on stderr, prints nothing on stdout, and leaves
`/var/tmp/ruta-$NAME-idle` absent. Follow [the refusals recipe](qa/refusals.md) for the commands
and the exact messages.

## Running sandbox

Needs a sandbox that the parent started in `$PROJECT` under `$NAME`. Reach it:

1. `cat /var/tmp/ruta-$NAME/display` prints a display such as `:3`.
2. `log` holds the line `ruta-test-app: running`.
3. When either fails, ask the parent to start the sandbox.

What to check:

- `shot` prints `/var/tmp/ruta-$NAME/shot.png`, and `file` reports a 640 x 480 PNG. `shot <file>`
  prints and writes that file instead.
- `log` prints the app log byte for byte:
  `RUTA_NAME=$NAME $RUTA log | cmp - /var/tmp/ruta-$NAME/app.log` prints nothing.
- Input reaches the window at the pixel a shot shows. Follow [the input recipe](qa/input.md) when
  an input command or its pacing changes.
- `opened` prints the links that the stub openers logged. Run
  `/var/tmp/ruta-$NAME/bin/xdg-open https://example.com/qa`, then `opened` prints that link, and
  so does `cat /var/tmp/ruta-$NAME/opened`. Nothing opens on the real desktop. The test app never
  opens a link itself, so report "the app opens a link" as not verified.

## Parent only

A worker cannot drive `up`, `restart`, `down`, `capture`, or `--headed`. Report each as not
verified. `--headed` shows a window on the user's real screen, so only the user can judge it.

The end-to-end tests cover these commands, and `cargo xtask check` leaves them out. The parent runs
them as [the development guide](../docs/development.md) describes, and gives the result in the
brief. The parent also passes the output of its own `up` and `down`:

- `up` prints `ruta <name> is up on <display> (log: /var/tmp/ruta-<name>/app.log)`.
- `restart` without arguments prints the same line, and `arguments` in the root is unchanged.
- `down` prints `ruta <name> is gone`, and the root is gone. A second `down` prints
  `ruta <name> is not up`.
