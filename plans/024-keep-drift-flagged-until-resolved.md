# Plan 024: Stop an audit from erasing the drift it just found

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat d8d8074..HEAD -- crates/lane/src/audit.rs crates/lane/src/store.rs test_lane.sh`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `d8d8074`, 2026-08-20

## Why this matters

`lane audit` reports drift once and then makes it undetectable.

For every note it checks, `run()` in `crates/lane/src/audit.rs` overwrites the stored
fingerprint with the *current* hashes — unconditionally, and **before** `apply_review` has
a chance to resolve anything:

```rust
        let previous = state.get(&note.meta.id).cloned().unwrap_or_default();
        let unchanged = previous.sig == res.sig
            && previous.body_hash == res.body_hash
            && previous.raw_hash == res.raw_hash
            && previous.status == res.tier
            && previous.norm == crate::syntax::NORM_VERSION;
        state.insert(
            note.meta.id.clone(),
            store::NoteState {
                sig: res.sig,
                body_hash: res.body_hash,
                raw_hash: res.raw_hash,
                ...
```

`Checker::check` uses that stored state as its baseline (`crates/lane/src/store.rs:379`).
So the next `lane check` compares the current code against hashes taken from the current
code, and answers `fresh`.

Review is **off unless configured** — the README says so, and no API key is the common
case. On that path a drifted note is never reviewed, never corrected, and never flagged
again. Its text keeps describing code that no longer exists while the store reports it
healthy.

Observed on this repository. Four notes on `scripts/check-linux.sh`, all reported `fresh`,
three of them describing a mount workaround that had been deleted hours earlier:

```
scripts/check-linux.sh#@file
    cannot run from a lane worktree: it does 'cp -r /w /build', ...      <- false
    the Linux gate needs the main checkout, not a lane                   <- false
    a linked worktree resolves its .git file through the shared Git
      directory, which must exist at the same absolute path ...          <- describes removed code
    A linked worktree .git file may contain a relative pointer ...       <- correct
```

`lane check` reported `body-drift 0`. The three wrong notes were retired by hand.

This is the tool's central promise inverted. A confidently wrong note is worse than no
note, which the design already acknowledges — it is why `contradicted` quarantines rather
than keeps. The same reasoning applies here: an unresolved drift must stay visible.

## The design

**A note's baseline may only move when the drift has actually been resolved.**

| tier this audit | what happens to the stored fingerprint |
|---|---|
| `fresh` | updated, as today |
| `unverifiable` | updated, as today — nothing was compared |
| `body-drift` / `signature-changed`, no reviewer ran | **left at its previous value** |
| `body-drift` / `signature-changed`, reviewer said `holds` | updated — that is what `holds` means |
| `superseded` / `contradicted` | note leaves `-/` for the attic; its state goes with it |
| `unsure` | **left at its previous value**, so it stays flagged |
| baseline unreadable because `NORM_VERSION` changed | updated — see below |

Keep the existing `rebaselined` path exactly as it is. That counter exists for a different
and legitimate case: a baseline written before a normalization change cannot be compared at
all, so re-anchoring is the only option (`store.rs:420`). Do not conflate the two.

The consequence to accept deliberately: an unresolved drift is re-reported by every
subsequent audit until someone resolves it. That is the point. It is a standing item, not a
notification, and `lane done` printing the same `review` line each time is the correct
behaviour — it is what makes the note's staleness visible without a reviewer configured.

## Current state

- `crates/lane/src/audit.rs`, `run()` — the state write quoted above, and the `drifted`
  vector built just before it from `res.tier == BODY || res.tier == SIG`.
- `crates/lane/src/audit.rs`, `apply_review()` — returns early when
  `drifted.is_empty() || !reviewer.enabled()`, so with no reviewer nothing downstream
  touches state. It already updates state for the verdicts it applies; that is the pattern
  to extend.
- `crates/lane/src/store.rs`, `Checker::check` — reads `self.state.get(&note.meta.id)` as
  the baseline, falling back to the note's own frontmatter hashes when absent.
- `crates/lane/src/store.rs`, `NoteState` — `sig`, `body_hash`, `raw_hash`, `status`,
  `checked`, `norm`, `reads`.
- `crates/lane/src/cli.rs`, `check()` — recomputes tiers via `Checker` and prints the
  counts; exits 1 on `anchor-missing`.

Conventions: one-line comments, and only where the reason is not obvious from the code;
`anyhow::Result`; tests in `#[cfg(test)] mod tests` at file end. Commit subjects are
Conventional Commits, `type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 3 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 3 |
| Linux gates | `./scripts/check-linux.sh` | passes from a lane *and* the main checkout |

Record both baselines before starting; at `d8d8074` they are 72 and 113.

`tests/fake-reviewer` is a working reviewer you can drive with
`--review cmd --review-cmd ./tests/fake-reviewer`; `test_lane.sh` already uses it. Use it
for the `holds` case rather than the network.

## Scope

**In scope**: `crates/lane/src/audit.rs`, `crates/lane/src/store.rs`, `test_lane.sh`,
`README.md`, `USAGE.md`.

**Out of scope**:
- The reviewer backends in `review.rs`, and anything that calls the network.
- Eviction, the budget, and the attic.
- `lane why`'s output format. It recomputes through `Checker`, so it will show `~` again
  once the baseline stops moving — no change needed.
- Making `lane check` exit non-zero on drift. Tempting, and a separate decision.

## Steps

### Step 1: Hold the baseline when drift is unresolved

In `run()`, write the current fingerprint only for notes that are not unresolved drift.
For a note whose tier is `BODY` or `SIG` and which no reviewer resolved, preserve the
previous `sig`, `body_hash` and `raw_hash`, while still recording `status` so the store
knows what it is.

Order matters: `apply_review` runs after this loop, so either defer the state write for
drifted notes until after review, or write the preserved values now and let `apply_review`
overwrite the entries it resolves. The second is smaller; say which you chose.

Keep `reads` and the `checked` semantics as they are — a no-op audit must still write
nothing.

**Verify**: `cargo test` passes at baseline + 0 so far; nothing else has changed yet.

### Step 2: Prove it end to end without a reviewer

In a scratch repo: write a note against a function, `lane audit`, edit the function's body,
`lane audit --review none`, then `lane check`.

Expected: `body-drift 1`, not `body-drift 0`. Run `lane audit --review none` a second time
and confirm it still reports `body-drift 1` — a standing item, re-reported.

**Verify**: paste the literal `lane check` output for both runs.

### Step 3: Prove `holds` still clears it

Same scenario, but audit with `--review cmd --review-cmd ./tests/fake-reviewer`. When the
verdict is `holds`, the fingerprint must be refreshed and the next `lane check` must report
`fresh`.

Read `tests/fake-reviewer` first to see what verdict it returns and for which input; if it
cannot be made to return `holds`, say so and extend it — it is a test fixture, not
production code, and `test_lane.sh` is in scope.

**Verify**: `lane check` → `fresh`, and the note is still in `.context/-/`.

### Step 4: Cover it

Add unit tests in `audit.rs`'s test module: an unresolved drift leaves the stored hashes
untouched; a `holds` verdict updates them; a `fresh` note updates them.

Add to `test_lane.sh` before the summary, numbered one past the last:

```bash
echo "== N. unresolved drift stays flagged =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
sedi 's/parse(token).is_valid()/parse(token).is_valid() \&\& true/' src/auth.rs
"$LANE" audit --review none > /dev/null
is "drift survives an audit with no reviewer" \
   "$("$LANE" check --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["body-drift"])')" "1"
"$LANE" audit --review none > /dev/null
is "and is re-reported by the next audit" \
   "$("$LANE" check --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["body-drift"])')" "1"
"$LANE" audit --review cmd --review-cmd "$FAKE" > /dev/null
is "a holds verdict clears it" \
   "$("$LANE" check --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["body-drift"])')" "0"
```

Confirm the first two fail against the pre-Step-1 binary. Check `lane check --json`'s actual
key names before relying on them; adjust the extraction, not the assertion's intent.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 3.

### Step 5: Say what it means

`README.md` and `USAGE.md` both describe the freshness tiers. Add one sentence: a drifted
note stays flagged until a reviewer resolves it or a human rewrites it, so `lane check`
keeps reporting it. `USAGE.md`'s "When things go wrong" gains a short entry for a note that
is simply wrong — delete the file and commit, which is already the documented remedy.

**Verify**: `grep -c 'stays flagged' README.md USAGE.md` → at least `1` each.

## Done criteria

- [ ] `cargo test` passes, baseline + 3; `./test_lane.sh` passes, baseline + 3
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` passes from a lane and from the main checkout
- [ ] Edit a noted span, `lane audit --review none`, `lane check` → `body-drift 1`
- [ ] A second `lane audit --review none` still reports it
- [ ] A `holds` verdict clears it and the note survives
- [ ] A `fresh` note's fingerprint still updates, and a no-op audit still writes nothing
- [ ] `git diff --stat -- crates/lane/src/review.rs crates/lane/assets/skill.md AGENTS.md` → empty

## STOP conditions

- A no-op audit starts writing state on every run. That defeats the per-branch state file's
  whole purpose and will show up as churn in `.context/state/`.
- Holding the baseline makes `lane done` fail, or makes a lane's state fail to fold into
  trunk's during `roll_up`.
- The `NORM_VERSION` rebaseline path (`store.rs:420`) stops working. It is a different case
  and must keep re-anchoring.
- You cannot make `tests/fake-reviewer` produce a `holds` verdict, and would have to reach
  the network to test Step 3.

## Maintenance notes

- The invariant this establishes: **a fingerprint is a record of the last state someone
  vouched for, not of the last state observed.** Any future code that writes `NoteState`
  must ask whether the drift was resolved, not merely whether it was seen.
- This was found by dogfooding, not by tests, and it had been true since the Rust rewrite.
  The tests all passed because every one of them audits and then asserts within the same
  run — none audit, change code, audit again, and ask what the store believes. The Step 4
  tests are the first with that shape; keep them.
- Once drift persists, a repository that has ignored it for a long time will report a large
  count on first upgrade. That is correct and should not be smoothed over.
