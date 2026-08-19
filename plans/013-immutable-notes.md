# Plan 013: Make note files immutable, and put everything that changes in per-writer files

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat c73428e..HEAD -- crates/lane/src/`

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `c73428e`, 2026-08-19
- **Supersedes**: plan 009 (per-branch read ledger) entirely, and the design 003 patched
  around. 003 has landed and stays landed; this removes the need for its workarounds.

## Why this matters

A note file mixes two kinds of data with opposite merge requirements:

| field | changes | written by |
|---|---|---|
| `id` `anchor` `created` `branch` `supersedes`, body | never | the author, once |
| `sig` `body_hash` `lines` `status` `checked` `reviewed` `verdict` `evicted` | most audits | every branch |

`merge=union` is correct for the first and cannot be correct for the second. One file is
being asked to be both append-only and mutable, and every symptom follows from that:
duplicate frontmatter keys, an audit that rewrites every note, and `.reads.jsonl` having
the identical shape of problem. Plans 003 and 009 are one bug wearing two hats.

Plan 003 landed a patch: skip the write when nothing changed, and keep a damaged note
inert rather than destroying it. That bounds the blast radius. It does not remove the
class — a review verdict, an eviction, or a `holds` refresh still mutates a shared file
on two branches at once.

The fix is to split the file along the line that already exists.

## The design

`.context/` holds exactly two kinds of file, and nothing else:

```
.context/
  src/auth.rs/01M0B9MBYB-must-stay-constant-time.md   immutable; the directory IS the path
  .state/<branch>.json                                disposable cache: fingerprints, verdicts
  .reads/<branch>.json                                disposable cache: read counts
  .attic/src/auth.rs/01M0B4KQTX-....md                evicted note, byte-identical
  .attic/.log/<branch>.jsonl                          durable record: why, when
```

Two rules produce the whole thing:

1. **A note file is written once and never rewritten.** Unique ULID filenames, no
   mutation, so a merge never has to merge one. `pinned` is the single exception —
   see below.
2. **Everything that changes is per-writer.** One branch, one file, so two branches never
   touch the same bytes. Some of those files are disposable caches and some are durable
   records; what matters is that each has exactly one writer.

Consequences worth stating:

- **`path:` leaves the frontmatter.** The directory already encodes it, and two sources of
  truth can drift. `path_from_location` already exists in `note.rs` from plan 003. This
  also makes a rename a pure file move — see plan 014.
- **Derived state is disposable.** A bad merge, a missing file, or a fresh clone costs one
  recompute, never a wrong answer. That is what makes reconciliation trivial: there is no
  merge rule, only "take the newest `checked`, else fall back to the creation fingerprint
  in the note."
- **`merge=union` goes away entirely.** It exists to paper over concurrent modification. With
  no concurrent modification left, it only hides real conflicts. A genuine conflict on a
  note file after this change means two people disagreed about `pinned`, and that should be
  loud.
- **Plan 003's `unreadable` flag and `raw` comparison become dead weight.** Leave them; they
  cost nothing and still guard against a hand-mangled file.

### `pinned` stays in the note file

It is the one field a human edits, deliberately and rarely. Keeping it next to the note is
worth the exception. The merge behaviour is acceptable and worth writing down:

- both branches pin → identical content → no conflict
- one pins, one leaves alone → git takes the pin
- one pins, one unpins → a real disagreement, surfaced as a real conflict

That last case is only loud once `merge=union` is removed, which this plan does.

## Current state

- `crates/lane/src/note.rs` — `Meta` carries all fifteen fields; `parse` and `render`;
  `path_from_location` already derives a path from a note's location.
- `crates/lane/src/store.rs` — `READS`, `bump_reads`, `read_counts`, `evict`, `Checker`,
  `Check`, the tier constants.
- `crates/lane/src/audit.rs` — `run` mutates `status`/`checked`/`sig`/`body_hash`/`lines`
  and writes; `apply_review` mutates `reviewed`/`verdict`/`status` and writes; both call
  `store::evict`, which sets `evicted` before moving the file.
- `crates/lane/src/cli.rs` — `init` writes the two `merge=union` rules.

Conventions: one-line comments, `anyhow::Result`, `#[cfg(test)] mod tests` at file end,
no `%`-style formatting.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 6 |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 5 |

Record both baselines before starting. At `c73428e` they are 28 and 50.

## Scope

**In scope**: `crates/lane/src/note.rs`, `store.rs`, `audit.rs`, `cli.rs`, `test_lane.sh`,
`README.md`, `USAGE.md`.

**Out of scope**:
- Rename following. Plan 014, and it is independent.
- The anchor grammar, tier semantics, ranking weights, and the budget.
- `lane done`'s ordering.
- Deleting plan 003's `unreadable`/`raw` machinery.

## Steps

### Step 1: Split `Meta` in two

In `note.rs`, reduce `Meta` to what is written once:

```rust
pub struct Meta {
    pub id: String,
    pub anchor: String,
    pub created: String,
    pub branch: String,
    pub sig: String,        // at creation, the fallback baseline
    pub body_hash: String,  // at creation
    pub lines: String,      // at creation, informational
    pub supersedes: String,
    pub pinned: bool,
}
```

Add `Note::path(&self) -> String` returning `path_from_location(file)`, and make every
`note.meta.path` reader use it. Keep `path_from_location` as the single implementation.

**Verify**: `grep -c 'meta.path' crates/lane/src/` → `0`; `cargo test` compiles.

### Step 2: A per-branch state file

In `store.rs`:

```rust
pub const STATE: &str = ".state";

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct NoteState {
    pub sig: String,
    pub body_hash: String,
    pub lines: String,
    pub status: String,
    pub checked: String,
    pub verdict: String,
}
```

`load_state(root) -> HashMap<String, NoteState>` reads every `.context/.state/*.json` and
keeps, per id, the entry with the newest `checked`. Ties resolve either way — the value is
derived, so a wrong pick costs one recheck.

`save_state(root, &HashMap<String, NoteState>)` writes only the current branch's file,
named `slug(current_branch(), 60)`, with sorted keys so the committed diff is stable, and
drops ids that are no longer in the live store.

**Verify**: a unit test writing two branch files with different `checked` values and
asserting `load_state` takes the newer.

### Step 3: `check` reads the baseline from state, then from the note

`Checker::check` compares the current hash against the latest confirmed fingerprint for
that id, falling back to the note's creation fingerprint when no state entry exists. A
fresh clone with no `.state/` is therefore correct, just noisier on its first audit.

**Verify**: a unit test — a note whose creation fingerprint is stale but whose state entry
matches the current code reports `fresh`.

### Step 4: Audit writes state, not notes

In `audit.rs`:

- the main loop computes tiers and accumulates a `HashMap<String, NoteState>`; it must not
  call `note.write` at all
- `apply_review` records `verdict` and the refreshed `status` into that map; the
  `superseded` branch still creates a **new** note file, which is a create, not a mutation
- `store::evict` becomes a pure file move plus one appended line to
  `.context/.attic/.log/<branch>.jsonl`:
  `{"id":...,"path":...,"anchor":...,"reason":...,"at":...}`
- `save_state` is called once at the end

Remove `evicted`, `status`, `checked`, `reviewed`, `verdict` from `Meta` (step 1 already
did) and from `render`'s output.

**Verify**: after `lane audit`, `git status --porcelain -- .context` shows changes only
under `.state/`, `.attic/`, and any newly created note; no existing note file is modified.

### Step 5: Stop writing merge rules

`cli::init` writes no `.gitattributes` rules. Notes never conflict because they are never
modified; state and reads never conflict because they are per-writer; a conflict on a note
is a real `pinned` disagreement and must stay loud.

Section 12 of `test_lane.sh` asserts two `merge=union` rules — change it to assert the
file is not created, or that it contains none, in the same commit.

**Verify**: `lane init` in a fresh repo → `.gitattributes` absent or without `merge=union`.

### Step 6: Migrate an existing store once

At the start of `audit::run`, if any note's frontmatter still contains `path:`, run a
one-shot migration: for each note, move its fields into the current branch's state file,
rewrite the note with the reduced frontmatter, and leave the file where it is. Log one line
saying it happened.

The tool is pre-release, so one migration path is enough; do not build a version ladder.

**Verify**: a store created before this plan audits cleanly afterwards with no notes lost.

### Step 7: Cover it

Add to `test_lane.sh` before the summary. Five assertions:

```bash
echo "== N. notes are immutable; state is per-branch =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
N=$(find .context -name '*.md' -not -path '*attic*' | head -1)
BEFORE=$(cksum < "$N")
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
"$LANE" audit > /dev/null
is "a drifted note file is not rewritten" "$(cksum < "$N")" "$BEFORE"
is "the fingerprint moved into state" \
   "$(find .context/.state -name '*.json' | wc -l | tr -d ' ')" "1"
is "the note carries no path field" "$(grep -c '^path:' "$N")" "0"
is "init writes no merge rules" "$(grep -c 'merge=union' .gitattributes 2>/dev/null || echo 0)" "0"
"$LANE" audit > /dev/null
is "a second audit still writes nothing new" \
   "$(git status --porcelain -- .context | grep -vc '.state/')" "0"
```

Confirm the first and third fail against the current code before changing it.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 5.

## Done criteria

- [ ] `cargo test` passes, baseline + 6; `./test_lane.sh` passes, baseline + 5
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `grep -rc 'merge=union' crates/lane/src/` → `0`
- [ ] `grep -c 'pub path' crates/lane/src/note.rs` → `0`
- [ ] An audit that finds drift modifies no existing `.md` file
- [ ] A store created before this plan survives one audit with no notes lost
- [ ] `plans/README.md` rows for 013 and 009 updated

## STOP conditions

- The migration in step 6 loses a note, a verdict, or an eviction reason on any store you
  can construct. Report rather than accepting the loss.
- Reconciling two state files needs a rule more complicated than "newest `checked` wins".
  It should not; the value is derived. If it does, the state is carrying something that is
  not derived and belongs in the note.
- Removing `merge=union` produces a conflict in `test_lane.sh` sections 6 or 13, which land
  two branches of memory. Those should be pure adds. A conflict there means something is
  still being mutated — report which file.
- `pinned` turns out to be read from somewhere that assumed it lived in a mutable file.

## Maintenance notes

- The invariant, worth defending in review: **`.context/` holds immutable notes and
  per-writer files. Nothing else.** A new field that changes over time goes in the state
  file; a new fact that is true forever goes in the note.
- Per-writer files come in two kinds and the distinction matters: `.state/` and `.reads/`
  are disposable caches that may be deleted at any time, `.attic/.log/` is a durable record
  that may not. Both are conflict-free for the same reason.
- Deferred: `.state/` and `.reads/` accumulate a file per branch forever. Same trade plan
  009 recorded — deleting a file another branch may have modified is a delete/modify
  conflict, worse than the kilobytes. Garbage-collect in an explicit `lane gc` on trunk if
  it ever matters, never as a side effect of audit.
