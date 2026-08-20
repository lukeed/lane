# Plan 027: Preserve the baseline that was actually compared against

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat 1f4f1ac..HEAD -- crates/lane/src/store.rs crates/lane/src/audit.rs test_lane.sh`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none (repairs plan 024)
- **Category**: bug
- **Planned at**: commit `1f4f1ac`, 2026-08-20

## Why this matters

Plan 024 promised that a drifted note stays flagged until someone resolves it. It does not.
Landing a lane erases the drift, which is the exact failure 024 was written to prevent.
Reproduced from a clean repository:

```
on trunk before:        0 drifted
in the lane:            1 drifted
lane done:              1 body-drift, review src/auth.rs#fn verify
on trunk AFTER landing: 0 drifted
```

**The cause is two different views of the same state.**

```
Checker::new   → load_state(root)        // merged across every branch   (store.rs:310)
audit::run     → store::own_state(root)  // this branch's file only      (audit.rs:126)
```

`Checker::check` resolves its baseline from the merged view, so inside a lane it correctly
finds the note's fingerprint in trunk's `state/main.json` and reports drift. But
`record_state` looks the same note up in the *lane's own* file, where it does not exist,
and `unwrap_or_default()` hands back empty strings:

```rust
    let previous = state.get(id).cloned().unwrap_or_default();
    let (sig, body_hash, raw_hash) = if unresolved {
        // Seeing drift is not enough to vouch for the new fingerprint.
        (previous.sig.clone(), previous.body_hash.clone(), previous.raw_hash.clone())
```

So 024 faithfully preserves nothing, and writes a malformed entry — status without a
fingerprint. Observed verbatim in a lane's `state/<lane>.json`:

```json
{
  "01M0G268B4G44WB6TFYRVTPB9G": {
    "status": "body-drift",
    "checked": "2026-08-20T17:06:09Z",
    "norm": "1"
  }
}
```

`roll_up` then merges that into trunk (it has the newer `checked`, so it wins), and the next
check reads it as never-fingerprinted:

```rust
        // Nothing to compare against yet, so this is a first fingerprint, not a change.
        if base.sig.is_empty() && base.body_hash.is_empty() {
            return Check { tier: FRESH, ... };
        }
```

Drift laundered into fresh, and the note's stale text now reports healthy.

**Why the tests did not catch it.** Every test written for 024 audits and asserts on a single
branch, where the note's entry *is* in that branch's own file, so `previous` is populated and
the preservation works. The bug needs a note whose baseline lives in one branch's file and is
audited from another — which is every lane, and no test. Plan 024's own maintenance note
observed that the original bug survived because tests audit and assert within one run; the
tests it added then audited and asserted within one *branch*. Same blind spot, one level up.

## The design

Delete the second lookup. `Checker::check` already resolved the baseline it compared
against; make it return that, and have `record_state` preserve exactly it.

Add the compared-against fingerprint to `Check`:

```rust
pub struct Check {
    pub tier: &'static str,
    pub sig: String,
    pub body_hash: String,
    pub raw_hash: String,
    /// The fingerprint this check compared against — the merged baseline, or the note's
    /// own creation fingerprint when nothing has confirmed it yet. Preserved verbatim when
    /// drift is unresolved, so the next check compares against the same thing.
    pub base: (String, String, String),
    pub span: Option<Span>,
    pub rebaselined: bool,
}
```

Populate it from the `base` that `check` already computes. On the early-return paths that
use `blank(...)` there is no meaningful baseline; return the note's own frontmatter
fingerprint there, or empty — those tiers (`MISSING`, `UNVERIFIABLE`, unreadable) are not
`BODY`/`SIG`, so `record_state` never preserves them. Say which you chose.

`record_state` then becomes:

```rust
    let (sig, body_hash, raw_hash) = if unresolved {
        res.base.clone()
    } else {
        (res.sig.clone(), res.body_hash.clone(), res.raw_hash.clone())
    };
```

It still needs `previous` for the `unchanged` comparison that keeps a no-op audit from
writing — keep that lookup, but do not source the preserved fingerprint from it.

**The two views now agree by construction rather than by coincidence.** That is the point:
any future change to how a baseline is resolved automatically applies to what gets preserved,
because there is only one resolution.

Do not "fix" this by making `audit::run` read merged state instead of `own_state`. The
per-branch file is deliberate — one branch, one file — and widening it would make an audit
rewrite entries belonging to other branches.

## Current state

`crates/lane/src/store.rs`:

- `pub struct Check` — quoted above, gains one field.
- `Checker::new` uses `load_state(root)`, the merged view. Leave it.
- `Checker::check` — resolves `base` from `self.state`, falling back to the note's
  frontmatter (`note.meta.sig`, `.body_hash`, `.raw_hash`, `.norm`). This is the value to
  expose.
- The early return `if base.sig.is_empty() && base.body_hash.is_empty()` → `FRESH` is what
  turns a malformed entry into a fresh one. Leave it; it is correct for a genuinely
  unfingerprinted note. This plan stops malformed entries being written in the first place.
- `roll_up` merges by `checked`, newer wins. Leave it.

`crates/lane/src/audit.rs`:

- `record_state` — quoted above.
- `run()` uses `store::own_state(root)` at line 126. Leave it.
- `refresh_holds` updates the fingerprint when a reviewer says `holds`. It must keep doing
  exactly that.

Conventions: one-line comments, and only where the reason is not obvious; `anyhow::Result`;
tests in `#[cfg(test)] mod tests` at file end. Commit subjects are Conventional Commits,
`type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 2 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 3 |
| Linux gates | `./scripts/check-linux.sh` | exit 0 from this lane |

Record both baselines before starting; at `1f4f1ac` they are 77 and 125.

Capture every exit code without a pipe (`cmd > /tmp/out 2>&1; echo $?`).

## Scope

**In scope**: `crates/lane/src/store.rs`, `crates/lane/src/audit.rs`, `test_lane.sh`.

**Out of scope**:
- `roll_up`'s merge rule, `own_state`, and `load_state`. None of them are wrong.
- The eviction ranking and the budget.
- `crates/lane/src/cli.rs`, `README.md`, `USAGE.md` — the documented behaviour does not
  change; it starts being true.
- `crates/lane/assets/skill.md`, `AGENTS.md`.

## Steps

### Step 1: Carry the baseline out of `check`

Add the field to `Check` and populate it wherever `check` returns. Then use it in
`record_state`.

**Verify**: `cargo test` passes at baseline; `cargo clippy --all-targets` clean.

### Step 2: Prove it across a branch boundary

This is the whole point of the plan, so do it by hand before writing any assertion. In a
scratch repository:

```
note on a function → lane audit → commit → lane new work
  → change that function's body inside the lane → lane audit --review none (in the lane)
  → inspect .context/state/<lane>.json
```

Expected: the lane's entry for that note carries a **non-empty** `sig`, `body_hash` and
`raw_hash` — the values from trunk's file, not empty strings — alongside
`"status": "body-drift"`.

Then `lane done --review none`, and on trunk:

```
lane check
```

Expected: `body-drift 1`, not 0.

**Verify**: paste the raw `state/<lane>.json` entry and the trunk `lane check` output.

### Step 3: Cover the branch boundary

Add to `test_lane.sh` before the summary, numbered one past the last. The assertions must
cross a branch boundary — a single-branch test cannot fail against the current code, which
is precisely why this shipped:

```bash
echo "== N. drift survives a landing =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "callers rely on the parsed shape" > /dev/null
"$LANE" audit > /dev/null
git add -A .context && git commit -qm memory > /dev/null
"$LANE" new carry > /dev/null 2>&1
( cd "$TMP/repo/.lanes/carry" \
  && sed 's/parse(token).is_valid()/parse(token).is_valid() \&\& true/' src/auth.rs > t \
  && mv t src/auth.rs && git add -A && git commit -qm "change the span" > /dev/null \
  && "$LANE" audit --review none > /dev/null )
is "a lane preserves the baseline it compared against" \
   "$(python3 -c "import json;print(int(any(v.get('body_hash') for v in json.load(open('$TMP/repo/.lanes/carry/.context/state/carry.json')).values())))")" "1"
( cd "$TMP/repo/.lanes/carry" && "$LANE" done --review none > /dev/null 2>&1 )
is "and the drift survives the landing" \
   "$("$LANE" check --json | python3 -c "import json,sys; print(sum(1 for n in json.load(sys.stdin) if n['tier']=='body-drift'))")" "1"
is "and is still reported by a later audit" \
   "$("$LANE" audit --review none > /dev/null; "$LANE" check --json | python3 -c "import json,sys; print(sum(1 for n in json.load(sys.stdin) if n['tier']=='body-drift'))")" "1"
```

Check `lane check --json`'s actual shape before relying on it — it returns a **list** of
objects with a `tier` key, not a map of counts. Adjust the extraction, never the assertion's
intent.

Confirm all three fail against the pre-Step-1 binary.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 3.

### Step 4: Keep the resolved paths working

Add unit tests in `audit.rs`: an unresolved drift whose baseline came from the merged view
preserves that baseline; a `holds` verdict still updates the fingerprint; a `fresh` note
still updates it; a no-op audit still writes nothing.

**Verify**: `cargo test` → baseline + 2; and in a scratch repo, checksum
`.context/state/<branch>.json` before and after a repeat audit — identical.

## Done criteria

- [ ] `cargo test` passes, baseline + 2; `./test_lane.sh` passes, baseline + 3
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` exit 0 from this lane
- [ ] A lane's state entry for a drifted note carries a non-empty fingerprint
- [ ] `lane check` on trunk reports the drift **after** a landing
- [ ] A later audit on trunk still reports it
- [ ] A `holds` verdict still clears drift; a no-op audit still writes nothing
- [ ] `git diff --stat -- crates/lane/src/cli.rs crates/lane/assets/skill.md AGENTS.md` → empty

## STOP conditions

- Fixing this appears to require `audit::run` to read merged state instead of `own_state`.
  It does not, and that change would have one branch rewriting another's entries.
- A no-op audit starts writing state on every run.
- A `holds` verdict stops clearing drift — that path must keep working, and it is the one
  most likely to break while making unresolved drift stick.
- `roll_up` needs changing. It does not; it was faithfully merging a malformed entry.

## Maintenance notes

- The invariant: **the fingerprint preserved on unresolved drift is the one the check
  compared against.** Two independent resolutions of "the baseline" will drift apart; there
  must be exactly one, and it lives in `Checker::check`.
- Any test for the memory store that operates on a single branch cannot see a class of bug
  that only appears across a branch boundary. Plan 024's tests were all single-branch, and
  this shipped. When testing state, cross a lane.
- `Check` now carries both the observed fingerprint and the compared-against one. Their
  names should stay unambiguous; confusing them is this bug in a different form.
