# Architecture decision records

Record lasting decisions and the reasons behind them, not how a feature works.

## Before writing

Follow [ADR 0001](./0001-documentation-scope.md) for documentation scope. Before opening the
[template](./TEMPLATE.md), answer:

- What choice are we making, and what lasting reason stands behind it?
- What credible alternative did we consider, and why did we reject it?

If the answers only restate what the code does, do not write an ADR. Code and tests hold behavior. A
feature change does not require a new document.

Use a title that names the choice, and state that choice in the first sentence of the Decision
section. Give each rejected or deferred alternative its own heading under Alternatives considered.
An ADR is not a feature summary, implementation plan, or acceptance checklist.

A new ADR is Accepted. Merging its PR is the approval, so there is no Proposed stage.

## Keeping ADRs current

An ADR states the current decision. When its details change, edit it in place. Supersede it only
when the core choice reverses. The new ADR then states the whole current rule and has a Supersedes
line that names the old ADR without a link. Delete the old file, and add a row for it to the Retired
table. Numbers are never reused. To read a retired ADR, run `git show <commit>:docs/adr/<file>` with
the commit in its row.

## Index

### Documentation

- [0001: Documentation scope](./0001-documentation-scope.md)

### Purpose and platform

- [0002: One shared CLI for private app windows](./0002-purpose-and-design.md)
- [0005: ruta supports Linux only](./0005-linux-only.md)

### Project and toolchain

- [0003: Toolchain and quality gates](./0003-toolchain-and-quality-gates.md)
- [0004: A Rust coding guide adapts Tau's coding rules](./0004-rust-coding-guide.md)

## Retired

| ADR | Title | Replaced by | Last commit |
| --- | ----- | ----------- | ----------- |
