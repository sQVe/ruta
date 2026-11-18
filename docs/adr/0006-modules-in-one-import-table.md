# ADR 0006: ruta's modules live in one import table

**Date**: 2026-11-18\
**Status**: Accepted\
**Related**: [Shared ADR 0002 (Capability modules with a project-owned import table)](../stomme/shared/adr/0002-capability-modules.md), [Shared ADR 0010 (Recorded violations)](../stomme/shared/adr/0010-recorded-violations.md)

## Context

ruta is one binary crate, and its code is a set of top-level modules under `crates/ruta/src`. Shared
ADR 0002 asks a project to name its modules and the directions between them in one table, so every
agent gets the same answer to where a file belongs and which module may use it.

Without that table, a helper lands in whichever module a change finds convenient, and a reader
cannot tell which module owns a concept. ruta also carries a few shapes that the shared layout rules
reject, so the table needs a place to record them.

## Decision

ruta lists its modules in the `[modules]` table of `stomme.toml`, with one row per top-level module
of `crates/ruta/src` and one edge per allowed import. The edges come from the imports the code has
today, so the table describes the code and does not reshape it. `stomme.toml` holds the table, and
this ADR does not copy it.

### Composition

`orchestration` composes the commands from the other modules, and no module imports it. A module
that needed `orchestration` would make the commands depend on their own caller.

### Public files

The `public` lists name the extra files another module already uses, as shared ADR 0002 allows.
`identity` publishes `identity/stop.rs`, and `input` publishes `input/send.rs`.

### Test support

`test_support` is test-only. It has mutual edges with `identity`, `process`, `process_stat`, and
`tool`. It builds fixtures from their types, and their tests use it. One direction alone would move
the fixtures into production modules or copy them into each test.

### Outside the table

`xtask` and `ruta-test-app` stay outside the table. They are not modules of the `ruta` binary.

### Recorded violations

Three kinds of recorded violation stay in `stomme.toml` until later changes make the file moves
they need. The list may only shrink.

- `role-folder` for `crates/ruta`, because its folder does not end in the role's required name.
- `rust-test-placement` for nine test files under `tests/` folders.
- `rust-owner-folder` for `display_start.rs`, which sits beside `display.rs` and not in its own
  folder.

## Consequences

### Positive

- One table answers where a file belongs and which module may use it.
- The check rejects an import that crosses the table, so the structure does not rely on review.
- The violations are visible and bounded instead of silent exceptions.

### Negative

- The mutual edges around `test_support` mean a reader cannot order those modules as a sequence.
- The table carries the recorded violations until the file moves land.
- Because the table follows today's imports, it also records edges that a better layout would not
  have.

## Alternatives considered

### Fewer, grouped modules

Group the top-level modules into a few larger ones, so the table has fewer rows and edges. Rejected
because grouping needs file moves now, and this change only adopts the shared rules.

### No import table

Describe the modules in prose and rely on review. Rejected because agents copy nearby code, and a
rule without a check drifts. Without the table, the shared import check has nothing to read.
