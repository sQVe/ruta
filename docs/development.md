# Development

## Set up

1. Install [rustup](https://rustup.rs). The first `cargo` command in the repository installs the
   toolchain that `rust-toolchain.toml` pins.
2. Install the check tools:

   ```sh
   cargo install --locked cargo-nextest cargo-deny cargo-shear cargo-mutants typos-cli taplo-cli prek
   ```

3. Install the Git hooks:

   ```sh
   prek install
   ```

## Workspace

- `crates/ruta` is the `ruta` binary. `cargo run` starts it.
- `crates/ruta-test-app` is a small X11 app that the end-to-end tests open in a sandbox. It is not
  published.
- `xtask` holds repository tasks, run as `cargo xtask <task>`.

## Check your work

Run every check before you open a pull request:

```sh
cargo xtask check
```

It changes no files. It runs rustfmt, typos, taplo, cargo-deny, cargo-shear, clippy, nextest, and
rustdoc, and reports every failed check at the end. Warnings fail clippy, the tests, and rustdoc.
Every Cargo build runs with `--locked`, so a stale `Cargo.lock` fails too.

`cargo xtask check tidy` runs only the checks that do not compile code. `cargo xtask check build`
runs only the ones that do.

Tests that send signals run themselves in a new user and PID namespace through `unshare`, so they
can reach only test processes. They fail instead of running on the host when the namespace cannot
be made, such as when `unshare` is missing or the system blocks unprivileged user namespaces.

The end-to-end tests start real sandboxes, so the check leaves them out. They need Weston,
Xwayland, xdotool, and ImageMagick, in the versions that [ADR 0005](adr/0005-linux-only.md)
names. Run them with:

```sh
cargo build -p ruta-test-app
cargo test -p ruta --test end_to_end -- --ignored
```

The tests run without network, so build the test app first to download its dependencies.

Fix problems with these commands:

| Problem | Command |
|---|---|
| Rust formatting | `cargo fmt --all` |
| TOML formatting | `taplo fmt` |
| Spelling | `typos --write-changes` |
| Unused dependencies | `cargo shear --fix` |
| Some clippy warnings | `cargo clippy --workspace --all-targets --fix` |

The pre-commit hook runs rustfmt, clippy, typos, and taplo, but only in check mode. Run the fix
commands yourself.

## Mutation testing

Mutation testing changes the code in small ways and reports each change that no test catches.
`cargo xtask mutation` runs [cargo-mutants](https://mutants.rs) and prints one JSON report on stdout
in the format Tau's `mutation_check` tool reads.

```sh
cargo xtask mutation changed main

cargo xtask mutation audit xtask/src/check.rs

cargo xtask mutation changed main --max-mutants 50
```

`.cargo/mutants.toml` leaves out test code and test helpers. A missed mutant is a survivor: add a
test that fails when that line changes. The command exits 0 when every mutant is caught, 2 for
survivors in changed mode, and 1 for an error or an unmutated file. cargo-mutants writes its logs
to `target/mutants/mutants.out`.

## CI

`.github/workflows/ci.yml` runs on pull requests, on pushes to `main`, and weekly, so new security
advisories fail cargo-deny without a code change.

- `Check` runs `cargo xtask check tidy` on Linux.
- `Test (Linux)` runs `cargo xtask check build`. ruta supports Linux only; see
  [ADR 0005](adr/0005-linux-only.md).
- `End to end (Linux)` runs the end-to-end tests in an Arch Linux container on pull requests and
  pushes. On failure it uploads the sandbox logs and shots.
- `Mutation (changed)` runs on pull requests. It runs `cargo xtask mutation changed` against the
  pull request's base, prints the JSON report, and lists survivors in the job summary. Survivors
  pass the job; an error or an unmutated file fails it.
- `Mutation audit` runs on the weekly schedule and on manual runs from `main`. It runs
  `cargo mutants --workspace` and uploads `mutants.out` as an artifact. Survivors and timeouts
  pass the job; other cargo-mutants failures fail it.

Only runs on `main` save the build cache. Renovate updates dependencies, actions, and the toolchain
every Monday, after a release is three days old.

## Release

A release is a tag on a commit of `main`. Other repositories install ruta by tag, with the command
that [ADR 0002](adr/0002-purpose-and-design.md) gives, so the tag is the version.

1. Name the tag `v<major>.<minor>.<patch>`.
2. Create the tag signed and annotated, with the message `ruta <tag>`:

   ```sh
   git tag --sign --message "ruta <tag>" <tag> <commit>
   ```

3. Check the signature, then push the tag:

   ```sh
   git tag --verify <tag>
   git push origin <tag>
   ```

The `version` in `crates/ruta/Cargo.toml` stays at `0.0.0`. Do not change it for a release. So
`cargo install --list` shows every installed ruta as `v0.0.0`, with the tag in the source URL beside
it.
