# Plan 033: Drop the log, and let a note carry its own baseline

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: tech-debt
- **Supersedes**: the log half of plan 032, and plan 013's split of note frontmatter

## Why this matters

lane exists to make worktrees easy while keeping memory durable and shared. A note is
that memory, and a note is a file. `.lane/log.jsonl` is machinery that serves neither
goal, and it costs a merge conflict on every pull request.

GitHub does not apply `merge=union`. The driver runs on a local merge or rebase only, so
`.gitattributes` fixes nothing on the pull request page. PR #7 and PR #6 shared exactly
one file — `.lane/log.jsonl` — and GitHub blocked the merge. Every lane pull request hits
this the moment any other lane lands.

The log holds three unrelated things:

| kind | read by | needs to be shared? |
|---|---|---|
| `evict` | nothing at all | no |
| `landing` | `prune`, `ls` — both local operations on local worktrees | no |
| `holds` / `rebaseline` | `Checker`, for a note's baseline | **yes** |

Only the third is real, and it is per-note. Notes are already one file each and have never
conflicted. Give a note its own baseline and the shared append file has no remaining job.

## The design

```
.lane/
  memory/<path>/<ulid>-<slug>.md    the note, and now its own baseline
  attic/<path>/<ulid>-<slug>.md     retired; the file's location IS the eviction record
```

No `.lane/log.jsonl`. No `.gitattributes`. Two branches conflict only when they re-vouch
the same note, which is a real disagreement about what a baseline should be.

The landing marker moves next to the lane id, inside the worktree's own git directory,
where the lane id already lives and is already never committed.

## Scope

In: `store.rs`, `audit.rs`, `cli.rs`, `note.rs`, `.gitattributes`, `README.md`,
`www/src/pages/usage.md`, `crates/lane/assets/skill.md`, `scripts/test.sh`.

Out: `syntax.rs`, `cow.rs`, `worktree.rs` clone paths, `git.rs`. Do not touch the
benchmark harness in `scripts/bench.sh`.

## Steps

### Step 1: A note carries its own baseline

`sig`, `body_hash` and `raw_hash` in note frontmatter stop meaning "the shape at creation"
and start meaning "the shape this note was last confirmed against". Add one field:

```
vouched: 2026-08-24T19:42:51Z
```

written only when a note is re-vouched, absent on a fresh note. `created` keeps its
meaning and never changes.

`Note::render` must keep field order stable and must not emit `vouched` when it is None.

### Step 2: `holds` and `rebaseline` rewrite the note

- `store::append_confirmation` is deleted. In its place, a function that loads the note,
  writes the three fingerprint fields and `vouched`, and saves it.
- `store::confirmations` is deleted. `Checker::check` reads the baseline directly from
  `note.meta.sig` / `body_hash` / `raw_hash`, which is now the confirmed baseline.
- The `norm` field keeps working exactly as today: a norm bump re-baselines, and it now
  writes the note instead of appending a record.
- `note.rs:42` guards a note whose frontmatter did not parse — **that guard must still
  hold**. An unparsed note is never rewritten, so a `holds` on it is refused, not silently
  dropped. Report it to the user.

### Step 3: The landing marker goes local

- `store::LANDING`, `store::landings`, and `cli::landed_lanes` are deleted.
- `prepare`, which both `lane push` and `lane done` run through, writes a file next to the lane id, in the
  worktree's own git dir (`git_dir/lane/landed`), holding the lane id and an ISO stamp.
- `prune` gates on that file existing instead of on the trunk log. The second gate,
  `wt::contained_in`, is unchanged and remains the authoritative merge check — it already
  sees through squash and rebase merges.
- `ls` shows `landed` only for a lane that is BOTH marked and `contained_in` trunk.
  Call `contained_in` **only** for lanes that carry the marker, so an ordinary lane costs
  no extra git process. An unmarked lane shows `open` exactly as today.

### Step 4: `evict` disappears

`store.rs:378` stops appending. The note's presence in `.lane/attic/` is the record;
`lane why` already reads the attic. Delete `store::EVICT`.

### Step 5: Fold the existing log, once

Repositories in the wild carry a `.lane/log.jsonl`. At the start of `audit`:

- if `.lane/log.jsonl` exists, replay its `holds` and `rebaseline` records newest-first by
  `at`, writing each note's baseline, then delete the file and remove the
  `.gitattributes` rule (deleting `.gitattributes` if that leaves it empty);
- `landing` and `evict` records are dropped on the floor;
- it must be idempotent and must not fail when the file is absent, which is the normal
  case after the first run.

### Step 6: Delete the plumbing

`append_log`, `log_lines`, `log_path`, `LOG`, `HOLDS`, `REBASELINE`, `LANDING`, `EVICT`
and `Confirmation` all go. Grep for each and confirm no caller survives.

## CASE TABLE — handle and test every row

  N1. Fresh note, never re-vouched      → no `vouched` field; baseline is the creation shape
  N2. Note re-vouched once              → three fingerprints and `vouched` updated in place
  N3. Note re-vouched twice             → second overwrites the first; `created` never moves
  N4. Note whose frontmatter is damaged → NOT rewritten; `holds` refuses and says so
  N5. Note in `.lane/attic/`            → still readable by `why`; no eviction record needed
  N6. A norm bump                       → re-baselines by writing the note
  N7. Two branches re-vouch DIFFERENT notes → no conflict; both land
  N8. Two branches re-vouch the SAME note   → git conflict on that one file. This is
                                              correct and must not be papered over. No
                                              merge driver, no union rule.

  L1. Lane created, never prepared      → no marker; `prune` leaves it; `ls` says `open`
  L2. Lane prepared, PR not yet merged  → marker present, `contained_in` false → `ls` says
                                           `open`, `prune` skips with today's message
  L3. Lane prepared, PR merged normally → `ls` says `landed`, `prune` collects
  L4. Lane prepared, PR squash-merged   → same as L3; `contained_in` sees the patch id
  L5. Branch name reused by a NEW lane  → the new lane has no marker, so it is never pruned.
      This is the bug the committed marker existed to prevent; the local marker must
      prevent it too. `scripts/test.sh` §36 covers it and must keep passing.
  L6. Worktree removed by hand          → no marker to read; no panic

  M1. Repo with a legacy log holding holds → folded into notes, log deleted
  M2. Repo with a legacy log holding only landings/evicts → log deleted, nothing written
  M3. `audit` run twice                    → second run is a no-op, no error
  M4. Repo that never had a log            → no error

## Done criteria

- `rg 'log\.jsonl'` returns hits in **no** Rust source file
- `.gitattributes` is gone
- `cargo test`, `./scripts/test.sh`, clippy `-D warnings`, fmt all clean
- Two lanes that re-vouch different notes both land with no conflict

## STOP conditions

- If removing `confirmations` changes what `lane check` reports for any note in THIS
  repository, stop. Fold the legacy log first and compare again; the tiers must match.
- If `scripts/test.sh` §36 fails, the local marker is not preventing the reused-name case.
  Stop and report rather than weakening the test.
