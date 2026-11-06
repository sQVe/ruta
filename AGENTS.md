# ruta

A Rust CLI that gives agents and people a throwaway app window on a private headless Weston and
Xwayland display. Read [the development guide](docs/development.md) for local setup and
verification.

- Run `cargo xtask check` before finishing changes.
- Follow [the Rust coding guide](docs/coding.md) in Rust code. It changes and adds to Tau's coding
  instructions.
- Format with `cargo fmt`. rustfmt runs with its default settings.
- Follow the decisions in [docs/adr](docs/adr/README.md), and record new decisions there. Read that
  guide before adding or changing an ADR. When details of a decision change, edit its ADR in place
  in the same change. Do not write documents that explain how a feature works; see
  [ADR 0001](docs/adr/0001-documentation-scope.md).
- Comment only what the code cannot say, such as a constraint or a workaround. Do not describe the
  code's history.
- Before finishing a document, check its local links and verify the commands it gives against the
  repository.

## Architecture

- ruta is one standalone binary that every project installs and calls. Project-specific commands
  stay in each project. See [ADR 0002](docs/adr/0002-purpose-and-design.md).
- ruta supports Linux only. Add no `cfg(windows)` or `cfg(target_os = "macos")` code. See
  [ADR 0005](docs/adr/0005-linux-only.md).
- Never send input or open an app window on the real display. Every window and every input goes to
  the private display.
- The toolchain is pinned in `rust-toolchain.toml`. Lint levels live in `[workspace.lints]`, and CI
  fails on warnings. Add single clippy lints, not whole groups. See
  [ADR 0003](docs/adr/0003-toolchain-and-quality-gates.md).
- ruta is MIT. Never depend on a GPL crate. See
  [ADR 0003](docs/adr/0003-toolchain-and-quality-gates.md).
