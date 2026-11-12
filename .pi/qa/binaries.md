# Binary proof

Use it before every section. It proves that `$RUTA` and `$APP` serve this worktree.

## Entry points

- `$RUTA`: the `target/debug/ruta` of this worktree.
- `$APP`: the `target/debug/ruta-test-app` of this worktree.

## Driving steps

Run both commands in the worktree.

1. For `$RUTA`:

   ```sh
   find crates/ruta Cargo.toml Cargo.lock -newer $RUTA \( -name '*.rs' -o -name 'Cargo.*' \)
   ```

2. For `$APP`:

   ```sh
   find crates/ruta-test-app Cargo.toml Cargo.lock -newer $APP \( -name '*.rs' -o -name 'Cargo.*' \)
   ```

## Observable results

- Step 1: a file outside `crates/ruta/tests` in the output means a stale binary.
- Step 2: any file in the output means a stale binary.
- When a binary is missing or stale, stop and ask the parent to build it.

## Gotchas

- Never run a bare `ruta`. That is the installed release, not this worktree.
- Cargo does not build the app again when only `crates/ruta` changes.
- `$RUTA --version` prints the release, not the build's source revision. Keep the two checks
  above.
