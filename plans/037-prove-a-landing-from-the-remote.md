# Plan 037: Prove a landing from the remote, not from a patch

> **Executor instructions**: Follow this plan step by step. Run every verification command and confirm the expected result before moving on. If a STOP condition occurs, stop and report.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Revises**: plan 033's `landing` row. Landings still need no sharing; what 033 missed is that arrival is a fact about the remote, so a local marker alone cannot prove it.

## Why this matters

Two lanes in this repository landed and neither `ls` nor `prune` can see it:

```
skipped fast-clone: commits main does not have
skipped fix-merge-staging: commits main does not have
```

Both trace to `wt::contained_in` (`worktree.rs:609`), which answers "did this land" by comparing patch ids. A squash merge destroys the evidence that read depends on, in two different ways.

**fast-clone is a false negative.** Its collapsed probe and the diff of its own squash commit `c904e8d` differ by exactly one line:

```
<  fn registered(root: &Path, dest: &Path) -> bool {
>  pub fn registered(root: &Path, dest: &Path) -> bool {
```

#23 made `registered` public between fast-clone's merge-base and fast-clone's own merge. That line is context, not a change fast-clone made, and `patch-id` hashes context. `150dfc00` against `e0a402b8`. Any pull request landing between a lane's base and its merge can do this to any lane.

**fix-merge-staging is a question the probe cannot express.** #23 squashed its first five commits. Then `7e70718` and `66c1fc1` were committed on the same branch and shipped separately through the enter-exit lane as #25. The probe collapses a branch to one patch, so a branch that landed in two pieces can never match.

Plan 033's case table says:

```
L4. Lane prepared, PR squash-merged → same as L3; contained_in sees the patch id
```

True only when nothing lands in between. There is no row for a lane that keeps committing after its own merge.

## The design

Git already separates these states, and stores exactly two things to do it:

| `branch.<n>.remote` | `refs/remotes/origin/<n>` | git's name | meaning |
|---|---|---|---|
| absent | — | untracked | never pushed |
| present | present | tracking | pushed, still open |
| present | **absent** | **`gone`** | the remote retired the branch |

`gone` reads from local refs, compares no patches, and therefore survives a squash — a squash rewrites commits but cannot resurrect a deleted head branch. GitHub's auto-delete fires on merge only; closing a pull request leaves the branch standing.

So a **marked** lane whose upstream is `gone` has landed. `contained_in` stays exactly as it is for every lane that is not `gone`. This adds a fast path in front of the probe; it removes nothing.

`gone` cannot answer the second question — whether anything reached the lane after it was prepared. That needs the branch tip recorded at landing time, which the marker does not hold today.

## Scope

In: `store.rs`, `worktree.rs`, `cli.rs`, `scripts/test.sh`, `CONTEXT.md`, `plans/README.md`.

Out: `help.rs` — new behaviour does not earn a clause in a help description, and `prune` fetching is not a new command. Also out: `note.rs`, `audit.rs`, `syntax.rs`, `cow.rs`, and every clone path in `worktree.rs`.

## Steps

### Step 1: The marker records the tip

`store::mark_landed` writes `<id> <iso>` today. It gains the branch tip as a third field, space separated and last, so a two-field marker written by an older lane still parses.

Ordering is the trap. `mark_landed` runs inside `prepare` (`cli.rs:1267`) **before** `stage_memory` commits the memory update, so the tip at that moment is not the tip that gets pushed. Stamp the tip after that commit exists, not at the current call site.

Add `store::landed_tip(worktree) -> Option<String>`, returning `None` for a marker with no third field. `None` means unknown, never "nothing landed after".

### Step 2: `wt::upstream_gone`

```
git for-each-ref --format='%(upstream:track)' refs/heads/<branch>
```

`[gone]` means gone. Empty means either in sync or no upstream configured, and both fall through to `contained_in`.

Encode this trap in a comment: `%(push:track)` also reports `[gone]` for a branch that was never pushed, because it is computed against where a push *would* land. `clone-syscalls` in this repository proves it. Use `%(upstream:track)`, which is empty unless `branch.<n>.remote` exists.

### Step 3: `losses` counts what arrived after landing

`worktree.rs:487` reports `commits {trunk} does not have` and says in a comment that it cannot count. With a tip it can. New order inside `losses`:

- a tip is recorded → `rev-list <tip>..<branch>`; when above zero, `N commit(s) after landing`, which is exact;
- else the upstream is `gone` → the branch puts nothing at stake;
- else `!contained_in` → today's message, unchanged.

The uncommitted-change and pending-note losses are untouched and still run first.

### Step 4: `ls` and `prune` gate on gone or contained

Both call sites (`cli.rs:602`, `cli.rs:654`) treat a lane as landed when it is marked **and** (`upstream_gone` **or** `contained_in`). Order the `or` so `upstream_gone` runs first: it is one `for-each-ref` against a local ref, where `contained_in` can reach `commit-tree`.

The marker stays the first gate and short-circuits both probes, so an unmarked lane still costs no extra git process.

### Step 5: `prune` fetches before it decides

`gone` reads a cache that only empties on `git fetch --prune`. `prune` runs it first.

It must be failure tolerant: offline, unauthenticated, or no remote at all, `prune` warns on stderr and continues against the cached refs. A failed fetch is never fatal — it degrades to today's answer.

`ls` never fetches. It stays offline and instant, which is #7's win and the reason 033 kept the short-circuit.

### Step 6: `CONTEXT.md` catches up

The **Landing record** entry still describes the committed log that PR #11 deleted:

> Its presence in the base's copy of the log is what proves the branch merged

Rewrite it against what ships: a marker in the lane's own git directory, never committed, proved by the remote retiring the branch or by containment in the base.

## CASE TABLE — handle and test every row

```
G1.  Marked, gone, tip recorded, nothing after     → ls `landed`; prune collects
G2.  Marked, gone, tip recorded, 2 commits after   → ls `landed`; prune skips,
                                                     "2 commit(s) after landing"
G3.  Marked, gone, NO tip (marker from an older
     lane)                                         → tip unknown; fall back to
                                                     contained_in for losses. Never
                                                     assume the current tip is the
                                                     landing tip.
G4.  Marked, tracking, contained_in true           → ls `landed`; prune collects
G5.  Marked, tracking, contained_in false          → ls `open`; prune skips with
                                                     today's message. This is an open
                                                     pull request and must not change.
G6.  Marked, never pushed, landed by `lane merge`  → no upstream, so gone must NOT
                                                     fire; contained_in carries it
G7.  UNMARKED lane whose upstream is gone          → never pruned. The marker is the
                                                     identity gate, not the ref.
G8.  Remote branch deleted by hand, never merged   → reads as landed. Accepted risk,
                                                     stated in the plan. The dirty and
                                                     pending-note guards still fire.
G9.  Repo with no remote at all                    → no upstream config; falls to
                                                     contained_in; no error
G10. Fetch fails: offline, auth, dead remote       → warn, continue on cached refs,
                                                     exit code unchanged
G11. Repo with deleteBranchOnMerge off             → gone never fires; every lane on
                                                     contained_in, exactly as today
G12. Merged, but the stale ref not yet pruned      → the Step 5 fetch prunes it, then
                                                     gone fires. Without a fetch this
                                                     is G5.
G13. Worktree removed by hand, branch remains      → no marker to read; no panic
G14. Branch name reused by a NEW lane after the
     old one went gone                             → the new lane has its own git dir
                                                     and no marker, so it is never
                                                     pruned. §36 covers this and must
                                                     keep passing.
G15. Upstream on a remote that is not `origin`     → `%(upstream:track)` is remote
                                                     agnostic; works unchanged
G16. Lane pushed, then pushed AGAIN with more
     commits                                       → the tip is re-stamped on the
                                                     second push. A stale tip would
                                                     read the new commits as landing
                                                     after the fact.
G17. Marked and gone, but the worktree is dirty
     or holds pending notes                        → prune still skips on those
                                                     losses. Landing is not the only
                                                     guard.
```

## Done criteria

- A repro of the fast-clone shape — lane branched, another pull request lands touching an adjacent context line, lane squash-merged, head branch deleted — reports `landed` and is collected.
- A repro of the fix-merge-staging shape — commits added after the merge — is skipped with an exact count, not with `commits main does not have`.
- `lane prune` with the network unplugged still runs, warns once, and collects nothing it would not have collected before.
- `rg 'push:track'` returns no hits in Rust source.
- `cargo test`, `./scripts/test.sh`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all clean.

## STOP conditions

- If `scripts/test.sh` §36 fails, the reused-name protection broke. Stop; do not weaken the test.
- If `ls` gains a network call, or the git-process count for an unmarked lane rises above today's, stop. Both are 033's guarantees and #7's win.
- If an existing test asserting `commits main does not have` needs deleting rather than rewriting, stop and report. That message must survive for G5, which is the common case of an open pull request.
- If `upstream_gone` returns true for any lane in THIS repository that has an open pull request, stop. The signal is wrong and nothing downstream is safe.

## Migration

None, deliberately. Old markers carry no tip, and the only backfill available is "the current tip was the landing tip", which is false for exactly the fix-merge-staging shape and would let `prune` delete work that never landed. G3 keeps those lanes on today's behaviour.

The two stuck lanes in this repository are cleared by hand once:

```
lane rm fast-clone --force
lane rm fix-merge-staging --force
```

Both are verified safe: #24 carries fast-clone, and #25 carries the two commits fix-merge-staging added after #23.

`enter-exit` is **not** one of them. It already reports `landed`, and its blocker is a real pending note on `fn move_to` that never reached main. Resolve that note; do not force it away.
