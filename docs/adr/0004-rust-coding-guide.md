# ADR 0004: A Rust coding guide adapts Tau's coding rules

**Date**: 2026-11-06\
**Status**: Accepted\
**Related**: [ADR 0003 (Toolchain and quality gates)](./0003-toolchain-and-quality-gates.md),
[Rust coding guide](../coding.md),
[rust-analyzer style](https://github.com/rust-lang/rust-analyzer/blob/master/docs/book/src/contributing/style.md),
[Zed rules](https://github.com/zed-industries/zed/blob/main/.rules),
[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)

## Context

Agents write most of ruta's code, and Tau's coding instructions load in every session. Those
instructions were written for TypeScript. They say that repository conventions win when they differ.

Read literally, several Tau rules conflict with ordinary Rust code. Tau bans every abbreviation,
but trait methods require names such as `fmt`, and generic parameters are named `T`. Tau asks to
assign call chains and ternary expressions to variables, but Rust code combines computing and
deciding on purpose with `?`, `if let`, `let ... else`, and value-producing `match`. Tau puts
helpers before their callers, but Rust items are in scope in the whole module.

Rust also brings concerns Tau does not cover: ownership, error types, module files, and lint
suppression. The owner is new to Rust, so these rules must be written down, short enough to follow,
and concrete enough to check in review.

Published Rust guidance does not settle the details. rust-analyzer's style guide prefers a borrowed
`&str` to a borrowed owned type. Zed and rust-analyzer disagree on short names: Zed asks for full
words, while rust-analyzer's examples use names such as `s`. Zed bans `mod.rs` files.

## Decision

ruta keeps Tau's coding instructions and adds `docs/coding.md`, which states only the Rust rules
that override, clarify, or add to them. A Tau rule the guide does not change applies as written, so
the guide never repeats one. The guide keeps one policy per item. Its reasons live in this ADR.

### Names

Names use full words, as Tau asks, and the guide lists only the exceptions. The exceptions are
names a standard or trait method requires, generic `T`, and short lifetimes such as `'a`. These
names are the vocabulary every Rust example uses, so expanding them makes ruta harder to compare
with its sources. Other short names, such as `i`, `buf`, and `tx`, gain nothing the full word lacks.

### Item order and modules

Items read top-down: public and high-level items first, private helpers after, as rust-analyzer
orders them. A reader meets what a module offers before how it works. The compiler does not need
helpers first, so Tau's helper-before-caller rule has no Rust reason behind it.

A module `foo` lives in `foo.rs`, and its children in `foo/`, as Zed requires. Many files named
`mod.rs` are hard to tell apart in editor tabs and search results.

### Errors

A failure that can happen at run time returns a `Result` and propagates with `?`. An error that
callers branch on gets its own `thiserror` type with data in fields, so callers and tests match a
variant instead of parsing text. A test that tells failures apart counts as such a caller, because
an error built with `anyhow!` carries only its message, so the test could only compare text. App
code, which is `main` and the code that only passes errors up to it, uses `anyhow` with context,
because a new error type there would add code no caller reads. A panic marks a bug, and `expect`
says why the call cannot fail, so a panic message points at the broken assumption. A discarded
`Result` hides failures, as Zed's rules warn, so `let _ =` on a `Result` is not allowed. `_ =`,
`drop(result)`, and an unused `.ok()` hide a failure the same way, so the rule names them too.

`xtask` counts as app code. Its tasks only report errors, and no caller reads a `String` error's
text to decide what to do. The automation crates of
[rust-analyzer](https://github.com/rust-lang/rust-analyzer/blob/6e37b2d62127d445d77069031b416c48c9b0b4f2/xtask/src/main.rs#L33-L90),
[Zed](https://github.com/zed-industries/zed/blob/4ba7d82f6872cb5ee8adc122529e49ba28057c3a/tooling/xtask/src/main.rs#L4-L55),
[uv](https://github.com/astral-sh/uv/blob/4470c33bd7115dfb75922a59280e445d05cc181e/crates/uv-dev/src/main.rs#L18-L77),
and [Ruff](https://github.com/astral-sh/ruff/blob/8960d6db60e495867635b8aaedbe90e352b4b19f/crates/ruff_dev/src/main.rs#L93-L129)
also report with `anyhow`. An `xtask` error that a test tells apart gets a `thiserror` type, as
anywhere else.

`Drop` cannot return an error, so a cleanup error there goes to stderr with `eprintln!` under a
narrow `#[expect(clippy::print_stderr)]`. ruta has no logger, so stderr is the one place such an
error can go. One fixed form is easier to follow than a judgment of which errors are safe to drop,
and the `expect` marks each place in review.

### Statements

`?`, `if let`, `let ... else`, value-producing `if` and `match`, and short iterator chains are
written inline. They are the normal Rust forms, and splitting them would add names that carry no
meaning. A long chain, or one whose result is a concept, still gets a name, which keeps the purpose
of Tau's rule.

### Function size

Tau's size rules apply as written. The guide only clarifies them for Rust: an exhaustive `match`
can be long and still do one job, and splitting it to meet a review signal scatters one decision.

### Ownership

Functions borrow views such as `&str`, `&[T]`, and `&Path` to read, and take owned values to keep
them. The signature then shows who owns the data, and callers can pass any matching container. A
clone is a deliberate choice, because it can copy large data or share state. `redundant_clone`
catches some needless clones.

A lint suppression uses `#[expect(lint, reason = "...")]`. The compiler warns when the expected lint
no longer fires, so a stale suppression does not linger, and the reason tells the next reader why
the lint does not apply. A lint that fires in some builds and not in others uses
`#[allow(lint, reason = "...")]` instead, because `#[expect]` would warn in the builds where the
lint is silent. ADR 0003 states the same rule.

### Doc comments

Doc comments go only on items another crate uses, and state only errors, panics, and invariants the
name and types cannot show. ruta publishes no library, so full API documentation would mostly
restate signatures, which Tau's comment rules forbid.

### Tests

Unit tests live in a `#[cfg(test)] mod tests` in the file that owns the behavior. Tests that use a
crate from outside go in as few targets under `tests/` as possible, usually one, because Cargo links
each file there as its own binary. Case tables are plain loops, which need no dependency. Fakes
replace only IO boundaries, so tests run real logic. Tests match error variants and data, because
`is_err()` passes for any failure and message text changes with wording. Test folders come from
`tempfile`, because its guard removes the folder even when the test panics, while hand-written
cleanup at the end of a test never runs after a failed assertion. Mutation survivors are advice, as
in Tau.

### Lints

`print_stdout`, `print_stderr`, `allow_attributes_without_reason`, `should_panic_without_expect`,
and `redundant_clone` check parts of these rules. ADR 0003 lists how ruta adds single lints.

## Consequences

### Positive

- Agents and the owner get Rust rules that do not contradict the code they read and write.
- The guide stays short, because it holds only the differences from Tau's rules.
- Error and test rules let tests check what failed without depending on message wording.

### Negative

- A reader must combine two documents, Tau's coding instructions and this guide, to know every rule.
- When Tau's coding instructions change, the guide may need a matching change.
- Most rules cannot be checked by a lint, so review must check them.
- Typed errors and `expect` messages take more code than `unwrap` and string errors.

## Alternatives considered

### A full Rust guide that repeats Tau's rules

Write one guide that holds every rule, including those that carry over unchanged. Rejected because
Tau's coding instructions load in every session anyway, and two copies of a rule drift apart.

### Tau's coding instructions unchanged

Follow Tau's rules as written. Rejected because several of them, read literally, forbid normal Rust
code, such as `fmt`, `T`, and `?` inside an expression.

### An external guide adopted whole

Adopt rust-analyzer's style guide or Zed's rules as written. Rejected because they disagree with
each other on names, and each was written for its own large code base, not for ruta.

### Helpers before their callers

Keep Tau's order, with helpers declared first. Rejected because Rust does not need it, and the
top-down order shows what a module offers first.

### `mod.rs` files

Store a module with children in `foo/mod.rs`. Rejected because many files share the name `mod.rs`,
which makes editor tabs and search results hard to read.

### Size limits as lints

Set clippy's `too_many_lines` and `too_many_arguments` thresholds to Tau's numbers. Rejected because
exhaustive matches would trip them while still doing one job, and the fixes would scatter code to
meet a number.

### `missing_docs` and full rustdoc

Turn on the `missing_docs` lint. Rejected because ruta publishes no library, and the lint would
force comments that restate names.

### Lints that ban `unwrap` and `expect`

Turn on `unwrap_used` and `expect_used`. Rejected because a panic is the right response to a bug,
and `expect` with a reason documents the assumption.

### One error style everywhere

Use only `thiserror` types, or only `anyhow`. Rejected because callers that branch need typed
variants, while app code that only reports an error gains nothing from a new type.

### `String` errors in `xtask`

Exempt `xtask` and let its tasks return `Result<_, String>`. Rejected because a `String` loses the
cause chain that `anyhow` keeps, and its tests can only compare message text.

### Test case libraries and mocking frameworks

Use `rstest` for case tables and `mockall` for fakes. Deferred because plain loops and small trait
fakes cover ruta's tests today, and each library is one more dependency.
