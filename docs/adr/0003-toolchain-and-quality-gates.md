# ADR 0003: Toolchain and quality gates

**Date**: 2026-11-06\
**Status**: Accepted

## Context

ruta starts as a new Rust workspace, so it must choose its toolchain, formatting, linting, and
license checks before code exists. greppa made the same choices after a survey of eight active Rust
projects, among them Zed, uv, Ruff, Helix, rust-analyzer, Tauri, and Bevy. All eight format
with rustfmt and fail CI on clippy warnings. Most use edition 2024 and shared workspace lints. Few
turn on the whole clippy pedantic group. ruta takes the same setup, so an agent or person who works
in both repositories meets the same rules.

ruta ships under MIT, so every dependency must have a license that allows it.

## Decision

ruta pins a stable Rust toolchain, formats with default rustfmt, lints with selected clippy lints,
checks dependencies with cargo-deny, runs formatting and linting in prek hooks, runs mutation
testing with cargo-mutants as advice, and runs every test that sends signals in its own user and
PID namespace.

### Toolchain

- `rust-toolchain.toml` pins one stable release, with clippy and rustfmt. Every developer and CI
  build with that release. Moving to a newer release is a deliberate change.
- Every crate uses edition 2024.

### Formatting

- rustfmt runs with its default settings. ruta does not loosen or tighten the line-breaking widths.
- rustfmt never adds the blank lines between logical steps, so review checks them.

### Linting

- Clippy lint levels live in `[workspace.lints]`, and every crate uses them.
- The `all` group warns and `correctness` denies. Single lints, such as `dbg_macro`, `todo`, and
  `print_stdout`, are added when they are needed, also from groups that stay off. The whole
  pedantic, nursery, and restriction groups stay off.
- A suppression is `#[expect(lint, reason = "...")]` on the smallest item that needs it. The
  `allow_attributes_without_reason` lint rejects a suppression without a reason.
- A lint that fires in some builds and not in others is suppressed with
  `#[allow(lint, reason = "...")]`, because `#[expect]` warns in the builds where the lint is
  silent.
- Unsafe code is denied.
- CI fails on any warning.

### Dependencies and licenses

- ruta is MIT.
- cargo-deny checks licenses, advisories, sources, and duplicate crates. It allows Apache-2.0 and
  other licenses compatible with an MIT tool, and rejects GPL.

### Hooks

- prek runs Git hooks from a config tracked in the repository. The hooks run formatting and linting
  before a commit.
- `cargo xtask check` is the one entry point for the full set of checks.

### Mutation testing

- cargo-mutants mutates every workspace crate. `.cargo/mutants.toml` leaves out only test code and
  test helpers, because a mutant there weakens a test, not the code it checks. Each exclusion has a
  comment that names its reason.
- CI mutates the diff of each pull request with `cargo xtask mutation changed`, and the scheduled
  run on `main` audits the whole workspace and saves the cargo-mutants report.
- Survivors are advice. They never fail a pull request or the audit, because some survivors are
  equivalent mutants that no test can catch, and a hard gate would push people to write tests
  that only kill a mutant. An error, an unmutated file, or a failing baseline fails the job.

### Test isolation

- ruta's sandbox code scans `/proc` and signals what it finds. A test that runs it on a developer's
  machine could reach the developer's own session if a bug or a mutant widens what it selects. So
  each test that sends signals re-runs itself through `unshare` in a new user and PID namespace with
  its own `/proc`, where it sees only test processes. It fails instead of running on the host when
  the namespace cannot be made.
- Inside the namespace, `/tmp` is a private tmpfs before the test body runs. An X server judges a
  lock file in `/tmp` by the PID it names, and a PID from another namespace never answers, so with a
  shared `/tmp` parallel tests take over each other's displays and could remove the host's locks.
  The mount stays in the test's own mount namespace. When it fails, the test fails, and the test
  binary refuses to run from a path under `/tmp`, so mutation testing copies the tree elsewhere.
- The namespace also has its own network namespace. X clients connect first to an abstract socket,
  which belongs to the network namespace, so a private `/tmp` alone still lets parallel tests reach
  each other's X servers.
- The re-run lives in the test binary, so every route to the tests gets it: `cargo test`, nextest,
  `cargo xtask check`, cargo-mutants, and CI. `cargo run` and an installed `ruta` do not change.
- CI must allow the namespace. The test jobs on the Ubuntu runner lift Ubuntu's AppArmor limit on
  unprivileged user namespaces with a sysctl step. The end-to-end job runs in a container started
  with `--privileged`, because Docker's default seccomp profile, its dropped capabilities, and its
  masked `/proc` paths each block the namespace on their own.

## Consequences

### Positive

- Every machine builds with the same compiler, so a lint or format result does not change between
  developers and CI.
- Default rustfmt needs no config to maintain and matches most Rust code a reader has seen.
- A GPL dependency or an unknown source fails the build before it ships.
- Format and lint failures show up before a commit, not only in CI.
- Mutation testing shows code that no test checks, on each pull request and in the weekly audit.
- A signal test cannot reach a process outside its namespace, so a wrong selection or a mutant
  costs a failed test, not the developer's session.

### Negative

- Default rustfmt keeps short calls, chains, and `if`/`else` blocks on one line, so code is more
  compact than some readers like.
- A pinned toolchain must be moved forward by hand, or ruta falls behind stable Rust.
- Selected lints catch less than the pedantic group, so some style problems are left to review.
- A new license in the dependency tree blocks the build until someone reviews and allows it.
- Hooks add time to each commit, and prek is one more tool to install.
- Mutation runs take much longer than the tests, and survivors can be left unfixed because they
  do not fail CI.
- Signal tests need `unshare` and a system that allows unprivileged user namespaces. Where neither
  is there, those tests fail rather than run.
- The end-to-end container runs with every privilege the runner grants. The job runs only this
  repository's test code.

## Alternatives considered

### Loosened rustfmt widths

Lower rustfmt's width settings so calls, chains, and `if`/`else` blocks split sooner. Rejected
because it adds config that differs from the Rust code readers know, and the more compact default
was acceptable.

### The whole clippy pedantic group

Turn on every pedantic lint and allow the ones that do not fit. Rejected because it brings many
lints ruta would turn off one by one, and only two of the eight surveyed projects do it.

### A floating stable toolchain

Build with whatever stable release is installed. Rejected because a new release can add lints or
change formatting, so the same code passes on one machine and fails on another.

### Survivors fail CI

Fail a pull request when a mutant survives. Rejected because equivalent mutants cannot be caught,
so each one would need a skip or a test that only exists to kill it.

### Allowing only the blocked parts in the container

Start the end-to-end container with a custom seccomp profile, `SYS_ADMIN`, and unmasked `/proc`
paths, instead of `--privileged`. Rejected because the three settings must change together, the
custom profile must be kept in step with Docker's default, and the job gains no protection that
matters, since it already runs only this repository's tests.

### Tests that trust their own selection on the host

Run signal tests on the host and rely on the marker, start-time, and group checks to keep them to
test processes. Rejected because a single wrong check, or a mutant that replaces one, signals the
developer's whole session.

### No license and advisory checks

Rely on review to catch a bad dependency. Rejected because the license of a transitive dependency
is easy to miss.
