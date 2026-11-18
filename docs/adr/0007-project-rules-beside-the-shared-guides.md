# ADR 0007: ruta keeps only its own rules beside the shared guides

**Date**: 2026-11-18\
**Status**: Accepted\
**Related**: [Shared coding guide](../stomme/shared/coding.md), [Rust guide](../stomme/languages/rust/guide.md), [ADR 0003 (Toolchain and quality gates)](./0003-toolchain-and-quality-gates.md)

## Context

The shared coding guide and the Rust guide state ruta's coding rules. A project guide that repeats
them drifts when a shared rule changes. Three rules have no shared equivalent: how `xtask` reports
errors, where a test gets its file system folder, and how the manual QA procedures are kept.

## Decision

ruta keeps no coding guide of its own. The shared guides state its coding rules, and this ADR holds
the rules that are ruta's own.

### xtask errors

`xtask` counts as app code. Its tasks report errors with `anyhow`, because a task only passes the
error to the person who ran it. An `xtask` error that a test tells apart gets a `thiserror` type,
because the test matches a variant instead of message text.

### Test folders

A test that touches the file system gets its folder from `tempfile`, whose guard removes the folder
even when the test panics. In `xtask`, the test gets its folder through `xtask/src/test_support.rs`,
so the crate keeps one helper.

### QA procedures

`.pi/qa.md` and the recipes beside it hold the manual QA procedures. They state what to check, not
how the code works. A change that alters a result a procedure names updates the procedure in the same
change, so the procedure does not describe an old result.

### Rules held elsewhere

Every other rule from the retired ADRs is stated by the shared guides or by ADR 0003.

## Consequences

### Positive

- Each coding rule has one home, so a shared change does not leave a project copy behind.
- The ruta-specific rules keep a written reason.

### Negative

- A reader must follow the shared guides and this ADR to know every ruta rule.
- Nothing checks the QA rule, so review must catch a procedure that no longer matches.

## Alternatives considered

### Keep a project coding guide for these rules

Keep a small project guide that holds only these rules. Rejected because a project guide beside the
shared guides drifts, and the rules fit one short ADR.

### Put the rules in `AGENTS.md`

State the rules in the agent instructions. Rejected because instructions say what to do, while an
ADR keeps the lasting reason beside the rule.

### Rely on the shared guides alone

State none of these rules and let the shared guides cover them. Rejected because none has a shared
equivalent, so the rules would be lost.
