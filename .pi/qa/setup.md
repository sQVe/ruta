# QA setup

For the parent. A `qa` worker never builds and never starts a sandbox, so do this before a launch
and undo it after the worker reports.

## Prepare

1. Build both binaries in the worktree:

   ```sh
   cargo build --locked -p ruta -p ruta-test-app
   ```

2. Make the fixture folder outside every repository. A committed fixture would make the worker
   edit a tracked file for the config refusals.

   ```sh
   PROJECT=$(mktemp -d /tmp/qa-ruta-XXXXXX)
   ```

3. Write `$PROJECT/ruta.toml`. Replace `<worktree>` with the absolute path of the worktree:

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

4. Pick `$NAME`: `qa-` and a short label, at most 24 bytes. Check that `/var/tmp/ruta-$NAME` and
   `/var/tmp/ruta-$NAME-idle` do not exist.
5. For the Running sandbox section, start the sandbox and keep the output for the brief:

   ```sh
   cd $PROJECT && RUTA_NAME=$NAME <worktree>/target/debug/ruta up
   ```

6. For the Parent only section, run the end-to-end tests as
   [the development guide](../../docs/development.md) describes.

## Undo

1. Stop the sandbox, and keep the output for your report:

   ```sh
   cd $PROJECT && RUTA_NAME=$NAME <worktree>/target/debug/ruta down
   ```

2. Check that `/var/tmp/ruta-$NAME` is gone.
3. Remove `$PROJECT`.
