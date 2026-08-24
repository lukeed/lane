# Plan 011: Stop discarding notes whose anchor we cannot resolve

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 43e404f..HEAD -- crates/lane/src/syntax.rs crates/lane/src/store.rs crates/lane/src/cli.rs`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `43e404f`, 2026-08-18

## Why this matters

`lane note` accepts any anchor, and the next `lane audit` silently evicts the note
if that anchor does not resolve. Two ways to hit it, both verified:

```
$ lane note -p src/Auth.swift -a "func verify" "must stay constant time"
noted -> src/Auth.swift#func verify
$ lane audit
memory: +1 new, 0 fresh, 0 body-drift, 0 signature-changed, 1 missing
  evict   src/Auth.swift#func verify  (anchor missing)
```

```
$ lane note -p src/a.rs -a "fn verfy" "typo in the anchor"
noted -> src/a.rs#fn verfy
$ lane audit
  evict   src/a.rs#fn verfy  (anchor missing)
```

The first case is a **regression from the Python implementation**, which matched
declarations with language-loose regexes — its Go pattern
`^\s*func\s+(?:\([^)]*\)\s*)?{name}\b` matched Swift's `func verify` fine. Now
anything outside the thirteen shipped grammars resolves `@file` and nothing else,
so a Swift, Ruby, Kotlin, Zig, PHP or Lua user loses every named-anchor note on
the first audit.

The two causes need different answers. A missing symbol in a language we parse is
real information — evict it. A language we cannot parse is not information about
the note at all, and treating absence of evidence as evidence of absence is what
throws the note away. The tool also has the user right there at `lane note` time
and says nothing.

### Plan 013 made this smaller

The tier now lives in `.context/state/<branch>.json`, not in the note, so adding a tier is
a `TIERS` entry, a `tier_rank` arm and a `Checker::check` branch. Eviction already keys on
`MISSING` alone, so an `unverifiable` note survives with no change to the audit.

## Current state

- `crates/lane/src/syntax.rs` — `grammar_for` at the extension table; `Source::resolve`
  returns `Option<Span>` and cannot distinguish "no grammar" from "not found".
- `crates/lane/src/store.rs` — tier constants `FRESH`/`BODY`/`SIG`/`MISSING`,
  `TIERS`, `tier_rank`, and `Checker::check`, which maps both `None` cases to `MISSING`.
- `crates/lane/src/audit.rs` — evicts on `MISSING` when not pinned.
- `crates/lane/src/cli.rs` — `note()` checks that the file exists, and nothing else.

`grammar_for` returns `None` for any unlisted extension:

```rust
        "md" | "markdown" => MARKDOWN,
        _ => return None,
```

`Checker::check` collapses both failures:

```rust
        let Some(src) = self.source(&note.meta.path) else {
            return missing(MISSING);
        };
        let Some(span) = src.resolve(&anchor) else {
            return missing(MISSING);
        };
```

Conventions: `%`-free formatting (`format!` with inline args), one-line comments only,
`anyhow::Result`, exit codes returned from `cli::run`.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | all pass, baseline + 4 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./scripts/test.sh` | `failed: 0`, baseline + 4 |

Record the baseline counts before you start.

## Scope

**In scope**: `crates/lane/src/syntax.rs`, `store.rs`, `audit.rs`, `cli.rs`, `scripts/test.sh`,
and `README.md`'s "Still stubbed" section.

**Out of scope**:
- Adding grammars. It is a one-line table entry, but every grammar is binary size and
  plan 012 owns that trade. This plan makes an unparsed language *safe*, not *supported*.
- The `MISSING` tier's existing behaviour for languages we do parse. A deleted symbol
  must still evict.
- Anchor resolution quality inside a supported language.

## Steps

### Step 1: Let `resolve` say why it failed

In `syntax.rs`, add:

```rust
pub enum Resolution {
    Found(Span),
    NotFound,
    /// No grammar for this file type, so absence of a match means nothing.
    Unparsed,
}
```

Add `Source::resolve_detail(&self, anchor: &str) -> Resolution`, holding the current
logic, and returning `Unparsed` when `self.grammar.is_none()` and the anchor is not
`@file`. Keep `resolve` as a thin wrapper returning `Option<Span>` so existing callers
and tests are untouched.

**Verify**: `cargo test` → baseline, all pass.

### Step 2: Add an `unverifiable` tier that never evicts

In `store.rs`:

```rust
pub const UNVERIFIABLE: &str = "unverifiable";
```

Add it to `TIERS` after `MISSING`, give it a `tier_rank` of 4, and map
`Resolution::Unparsed` to it in `Checker::check`.

In `audit.rs`, the eviction guard already keys on `MISSING` only, so an `unverifiable`
note survives with no change. Confirm that by reading it; do not widen it.

**Verify**: `cargo test` → all pass; `grep -c 'UNVERIFIABLE' crates/lane/src/store.rs` → at least `2`.

### Step 3: Report the tier honestly

`lane check` prints one line per tier from `TIERS`, so it picks up the new one for free.
`lane why`'s mark table in `cli.rs` needs a case — use `?`, which reads as "unknown"
next to the existing blank, `~`, `!` and `x`.

`lane check` exits 1 on `MISSING`; leave that. `unverifiable` is not a failure.

**Verify**: with a `.swift` file and a named-anchor note, `lane check` prints
`unverifiable 1` and exits 0.

### Step 4: Tell the user at `lane note` time

In `cli.rs`'s `note()`, after the existence check, resolve the anchor and act on the
three outcomes:

- `Found` — silent, as now.
- `NotFound` — print a warning to stderr naming the anchor, and record it anyway. The
  file may be about to gain the symbol.
- `Unparsed` — print a warning saying the file type has no grammar and the note will be
  kept but not checked for drift.

Record the note in all three cases; `lane note` must not become a gate.

**Verify**: `lane note -p src/a.rs -a "fn verfy" x` prints a `warning:` line on stderr,
exits 0, and the note survives the next `lane audit`.

### Step 5: Cover it

Add to `scripts/test.sh`, before the summary block, modelled on section 12:

```bash
echo "== N. anchors we cannot resolve are kept, not discarded =="
setup
printf 'func verify(_ t: String) -> Bool {\n    return ok(t)\n}\n' > src/Auth.swift
git add -A && git commit -qm swift
"$LANE" note -p src/Auth.swift -a "func verify" "swift: constant time" 2> /tmp/w.out
is "note on an unparsed language warns" "$(grep -c 'warning:' /tmp/w.out)" "1"
"$LANE" note -p src/auth.rs -a "fn verfy" "typo anchor" > /dev/null 2>&1
"$LANE" audit > /dev/null
is "the unparsed note survives" \
   "$(grep -rl 'swift: constant time' .context --include='*.md' | grep -vc attic)" "1"
is "check reports it as unverifiable" \
   "$("$LANE" check | awk '/^unverifiable/{print $2}')" "1"
is "a typo in a language we DO parse still evicts" \
   "$(grep -rl 'typo anchor' .context/.attic | wc -l | tr -d ' ')" "1"
```

Confirm the first and third assertions fail against the current code before changing it.

**Verify**: `./scripts/test.sh` → `failed: 0`, baseline + 4.

### Step 6: Correct the README

The "Still stubbed" section says a file with no grammar "resolves `@file` and nothing
else. Named anchors there report as missing rather than as unverifiable." Rewrite it to
describe the shipped behaviour: such notes are kept and reported `unverifiable`, and the
fix for a language you care about is a table entry in `crates/lane/src/syntax.rs`.

**Verify**: `grep -c 'report as missing' README.md` → `0`.

## Done criteria

- [ ] `cargo test` passes, baseline + 4
- [ ] `./scripts/test.sh` prints `failed: 0`, baseline + 4
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] A named-anchor note on a `.swift` file survives two consecutive `lane audit` runs
- [ ] A note with a typo'd anchor on a `.rs` file is still evicted
- [ ] `git status --short` lists only the in-scope files
- [ ] `plans/README.md` row updated

## STOP conditions

- Adding a tier breaks the `--json` consumers in `scripts/test.sh` sections 5, 8 or 10.
  Those parse `lane check --json` and `lane audit --json`; the new tier is additive, so
  it should not. Report rather than reshaping the JSON.
- You find yourself wanting to evict `unverifiable` notes to stop them accumulating.
  They are subject to the same per-anchor budget as everything else, which is the
  intended pressure valve. Do not add a second one.
- `Resolution::Unparsed` starts firing for files that *do* have a grammar. That means
  `grammar_for` is returning `None` for an extension it should know, which is a
  different bug — report it.

## Maintenance notes

- The rule this encodes: **evict on evidence, never on ignorance.** Any future check
  that cannot run should produce `unverifiable`, not `anchor-missing`.
- Adding a grammar later silently upgrades existing `unverifiable` notes to real
  checking on the next audit, with no migration.
- Deferred: whether `lane note` should offer the nearest matching symbol when an anchor
  does not resolve. Nice, and entirely separable.
