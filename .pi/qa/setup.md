# QA setup

For the manager. A `qa` worker never builds and never starts a sandbox, so do this before a launch
and undo it after the worker reports.

## Prepare

1. Build both binaries in the worktree:

   ```sh
   cargo build --locked -p ruta -p ruta-test-app
   ```

2. Print the guide, and follow its parts "Every session" and "The session that starts the
   sandbox" in the steps below:

   ```sh
   <worktree>/target/debug/ruta guide
   ```

3. Make the fixture folder outside every repository. A committed fixture would make the worker
   edit a tracked file for the config refusals.

   ```sh
   PROJECT=$(mktemp -d /tmp/qa-ruta-XXXXXX)
   ```

4. Write `$PROJECT/ruta.toml`. Replace `<worktree>` with the absolute path of the worktree:

   ```toml
   name = "qa"

   [app]
   command = ["<worktree>/target/debug/ruta-test-app", "ruta test app", "640", "480"]
   title = "ruta test app"
   width = 640
   height = 480

   [ready]
   line = "json"
   ```

5. Pick `$NAME`: `qa-` and a short label, at most 24 bytes. Check that `/var/tmp/ruta-$NAME`,
   `/var/tmp/ruta-$NAME-idle`, and `/var/tmp/ruta-$NAME-gone` do not exist. The worker uses
   `$NAME-idle` for the refusals.
6. For the Running sandbox section, start the sandbox and keep the output for the brief. It is
   the `up` line of the guide:

   ```sh
   cd $PROJECT && RUTA_NAME=$NAME <worktree>/target/debug/ruta up
   ```

7. For the Running sandbox section, also start a second sandbox and stop its app. Its `status`
   then exits 1 and prints `ruta $NAME-gone is not up`:

   ```sh
   cd $PROJECT && RUTA_NAME=$NAME-gone <worktree>/target/debug/ruta up
   kill $(cut -d' ' -f1 /var/tmp/ruta-$NAME-gone/pid)
   cd $PROJECT && RUTA_NAME=$NAME-gone <worktree>/target/debug/ruta status
   ```

8. For a change that falls under Not testable, ask the user before you run the end-to-end tests
   as [the development guide](../../docs/development.md) describes. Their sandbox names are the
   same in every worktree.

## Undo

1. Stop both sandboxes, and keep the output for your report. Each prints the `down` line of the
   guide:

   ```sh
   cd $PROJECT && RUTA_NAME=$NAME <worktree>/target/debug/ruta down
   cd $PROJECT && RUTA_NAME=$NAME-gone <worktree>/target/debug/ruta down
   ```

2. Check that `/var/tmp/ruta-$NAME` and `/var/tmp/ruta-$NAME-gone` are gone.
3. Remove `$PROJECT`.
