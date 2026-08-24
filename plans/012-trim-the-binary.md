# Plan 012: Make the grammar set a build-time choice

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 43e404f..HEAD -- crates/lane/Cargo.toml crates/lane/src/syntax.rs`

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: LOW
- **Depends on**: plans/011-anchor-coverage.md
- **Category**: dx
- **Planned at**: commit `43e404f`, 2026-08-18

## Why this matters

The release binary is 16.2 MB. Measured against a build carrying only the Rust grammar
(5.8 MB), the other twelve cost **9.9 MB — 64% of the binary**. Per-grammar static libs,
largest first: c++ 3.3M, typescript 2.8M, bash 1.3M, rust 1.1M, markdown 784K, c 632K,
python 460K, javascript 424K, java 424K, go 228K, css 140K, html 28K.

Nobody needs all thirteen. A Rust shop pays 2.8 MB for TypeScript and 3.3 MB for C++;
a TypeScript shop pays for both C dialects and Java. Meanwhile plan 011 exists because
the set is *too small* for Swift, Ruby, Kotlin and friends — and every language added to
close that gap makes this worse. The two only stop fighting if the set is a choice.

Runtime is already lazy: `grammar_for` constructs a `Language` only for the extension in
hand, and the declaration `Query` is behind a `OnceLock`. This is purely binary size.

**Considered and rejected — runtime grammar loading** (`libloading` + `dlsym`, as Neovim
and Helix do). It would drop the binary to ~5.8 MB and make the set unbounded, but it
buys a grammar build-and-distribution story, per-platform `.so` artifacts, tree-sitter
ABI skew between binary and grammar, and a first-run failure mode for a tool whose whole
install is `cargo install`. **WASM grammars** (tree-sitter's `wasm` feature) avoid the
per-platform builds but link wasmtime, which is larger than the grammars it replaces.
Revisit only if the grammar list grows past roughly thirty.

### Trimming a grammar moves `@file` hashes

Measured, not assumed. Hashing the same Rust body with and without the grammar:

```
normalized equal? false
raw equal?        true
```

A named anchor in a trimmed-out language is `unverifiable` under plan 011, so nothing
compares its hashes. But `@file` resolves without a grammar, and its *normalized* hash
changes because comments are no longer stripped — so a trimmed build would report drift on
every `@file` note for that language.

`raw_hash` is exactly the escape hatch for this and it does not fire, because `NORM_VERSION`
is a global constant that a trimmed build does not change. The fix is one line: make the
stored marker record whether a grammar was applied, `"1"` against `"1n"`, so trimming flips
it, the raw fallback engages, and identical bytes re-baseline silently. Do this in step 2,
not as an afterthought.

## Current state

`crates/lane/Cargo.toml` lists twelve grammar crates unconditionally.

`crates/lane/src/syntax.rs` names them in three places: the `const` grammar definitions,
the `grammar_for` extension table, and `comment_language`'s SFC arm.

```rust
fn grammar_for(ext: &str) -> Option<Grammar> {
    Some(match ext {
        "rs" => RUST,
        ...
        _ => return None,
    })
}
```

`Grammar` holds a `fn() -> Language` and a query string, so a gated grammar is a gated
`const` plus a gated match arm.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Default build | `cargo build --release` | succeeds |
| Minimal build | `cargo build --release --no-default-features` | succeeds |
| Tests | `cargo test` | baseline, all pass |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./scripts/test.sh` | `failed: 0`, baseline |

Measure size with `ls -l "$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys;print(json.load(sys.stdin)["target_directory"])')/release/lane"`.
The shared `target-dir` in `~/.cargo/config.toml` means a build in another checkout
overwrites that path — rebuild immediately before measuring.

## Scope

**In scope**: `crates/lane/Cargo.toml`, `crates/lane/src/syntax.rs`, `README.md`.

**Out of scope**: runtime or WASM loading; the grammar list's membership (which languages
to ship by default is a judgement call — keep today's thirteen as the default set);
`Query` construction and caching.

## Steps

### Step 1: One feature per grammar

In `crates/lane/Cargo.toml`, make each grammar crate `optional = true` and add:

```toml
[features]
default = ["rust", "go", "python", "javascript", "typescript", "c", "cpp", "java", "bash", "css", "html", "markdown"]
rust = ["dep:tree-sitter-rust"]
go = ["dep:tree-sitter-go"]
# ... one line per grammar ...
# html also backs .svelte and .vue block anchors; css backs their <style> blocks.
html = ["dep:tree-sitter-html"]
```

`typescript` covers both `.ts` and `.tsx` — one crate, one feature.

**Verify**: `cargo build --release` succeeds and the binary size is unchanged (within
noise) from before this plan.

### Step 2: Gate the three sites in `syntax.rs`

Put `#[cfg(feature = "...")]` on each grammar `const`, on its arms in `grammar_for`, and
on the SFC arms in `comment_language`. The `_ => return None` fallthrough already handles
an extension whose grammar was compiled out, which is exactly plan 011's `Unparsed` path —
so a trimmed build degrades to "kept but unverifiable", not to data loss. That is why this
plan depends on 011.

Gate the unit tests the same way, so `--no-default-features` still runs a green suite.

**Verify**:
- `cargo build --release --no-default-features` succeeds
- `cargo test --no-default-features` passes
- `cargo test` (default features) passes at the full baseline

### Step 3: Measure and record

Build both, record the sizes, and add a short README subsection under Install: the default
set, how to trim (`cargo install --path crates/lane --no-default-features --features rust,typescript`),
the measured saving, and a pointer that a trimmed-out language behaves exactly like an
unsupported one — notes are kept and reported `unverifiable`.

**Verify**: `grep -c 'no-default-features' README.md` → at least `1`.

### Step 4: Check the default set is still the honest one

With features in place, adding a language is a Cargo line plus three `cfg` arms. Confirm
the default list still matches what `README.md` claims under Install, and that the
`Grammar` table and the feature list cannot drift — a missing feature for a listed
grammar is a compile error, a missing grammar for a listed feature is not. Add a one-line
comment above the feature block saying they must move together.

**Verify**: `./scripts/test.sh` → `failed: 0`, baseline.

## Done criteria

- [ ] `cargo build --release` and `cargo build --release --no-default-features` both succeed
- [ ] `cargo test` and `cargo test --no-default-features` both pass
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/test.sh` passes at the baseline count
- [ ] The minimal build is at least 8 MB smaller than the default build
- [ ] README documents the default set and how to trim
- [ ] `plans/README.md` row updated

## STOP conditions

- A `cfg`-gated build fails to compile because a grammar is referenced outside the three
  known sites. Report the site rather than adding a fourth gate quietly.
- `--no-default-features` produces a binary that cannot resolve `@file` anchors. `@file`
  needs no grammar at all; if it breaks, `resolve` has a dependency it should not have.
- Plan 011 has not landed. Without the `unverifiable` tier, a trimmed build silently
  evicts every named-anchor note for the languages you removed — turning a size
  optimisation into data loss.

## Maintenance notes

- Adding a grammar is now: one dependency line, one feature line, one `const`, three
  `cfg` arms, one entry in the README's default set.
- Revisit runtime loading only if the list passes roughly thirty grammars, where the
  distribution machinery starts costing less than the binary does.
- The shared `target-dir` makes size measurements order-dependent across checkouts.
  Always rebuild immediately before measuring.
