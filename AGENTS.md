# ruta

A Rust CLI that gives agents and people a throwaway app window on a private headless Weston and
Xwayland display. Read [the development guide](docs/development.md) for local setup and
verification.

<!-- stomme:start -->
# Shared rules

- Run the full check before finishing changes: `cargo xtask check`.
- Format with `cargo fmt`.
- Follow the decisions in [docs/adr](docs/adr/README.md).
- Follow the shared ADRs in [docs/stomme/shared/adr](docs/stomme/shared/adr/README.md).
- Record new decisions in [docs/adr](docs/adr/README.md).
- Edit an ADR in place when its details change.
- Comment only what the code cannot say. Never describe history.
- Document decisions, not features. See [ADR 0001](docs/stomme/shared/adr/0001-documentation-scope.md).
- Read [the general guide](docs/stomme/shared/coding.md).
- Read [the Rust guide](docs/stomme/languages/rust/guide.md).
<!-- stomme:end -->

## Documents

- Before finishing a document, check its local links and verify the commands it gives against the
  repository.
- Read [the ADR guide](docs/adr/README.md) before adding or changing an ADR.

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
