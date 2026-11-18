# Architecture decision records

Record lasting decisions and the reasons behind them, not how a feature works.

## Writing ADRs

Follow [shared ADR 0001](../stomme/shared/adr/0001-documentation-scope.md) for documentation scope,
the ADR process, and what a document may contain. Open the [template](./TEMPLATE.md) before you
write. An ADR is not a feature summary, implementation plan, or acceptance checklist. A new ADR is
Accepted. Numbers are never reused. The Retired table keeps each retired ADR's last commit, and
`git show <commit>:docs/adr/<file>` restores the file.

## Index

### Purpose and platform

- [0002: One shared CLI for private app windows](./0002-purpose-and-design.md)
- [0005: ruta supports Linux only](./0005-linux-only.md)

### Project and toolchain

- [0003: Toolchain and quality gates](./0003-toolchain-and-quality-gates.md)
- [0006: ruta's modules live in one import table](./0006-modules-in-one-import-table.md)
- [0007: ruta keeps only its own rules beside the shared guides](./0007-project-rules-beside-the-shared-guides.md)

## Retired

| ADR | Title | Replaced by | Last commit |
| --- | ----- | ----------- | ----------- |
| 0001 | Documentation scope | [Stomme shared ADR 0001](../stomme/shared/adr/0001-documentation-scope.md) | a072a0eac2ab4079ef7b21b4d8aec60016465454 |
| 0004 | A Rust coding guide adapts Tau's coding rules | [Stomme shared coding guide](../stomme/shared/coding.md) | a072a0eac2ab4079ef7b21b4d8aec60016465454 |
