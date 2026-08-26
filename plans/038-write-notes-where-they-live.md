# Plan 038: Write a note where it lives, and delete the queue

> **Executor instructions**: Follow this plan step by step. Run every verification command and confirm the expected result before moving on. If a STOP condition occurs, stop and report.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: tech-debt
- **Supersedes**: plan 018. Its fix was correct for the queue it had; this removes the queue.

## Why this matters

A captured `Why:` trailer does not become a note. It becomes a line in `.git/worktrees/<name>/lane/pending.jsonl`, and only a landing turns it into a file. So memory is durable only if the author reaches for a lane-specific command, and invisible until they do.

Two notes were lost in this repository on 2026-08-26 exactly that way. Both lanes were pushed with `git push`, so nothing promoted; both were later removed, and the queue went with the worktree. Neither note was ever a file, so git had no copy:

```
.github/workflows/bench.yml   "ubuntu runners are ext4 and cannot exercise reflink at all…"
crates/lane/src/cow.rs        "the walk is pre-order, so a file's parent directory…"
```

Nothing in that sequence was a bug. Every part behaved as designed, and the memory was lost anyway. That is a design defect, not an implementation one.

**Plan 018's reason no longer applies.** 018 moved the queue out of the worktree because a new lane inherited the parent's unpromoted notes. The mechanism was that the queue was **gitignored**, and `lane new` clones ignored entries by reference. Notes written as ordinary files are not ignored, so they are not carried. Verified against the current tree:

```
$ printf 'x' > .lane/memory/src/auth.rs/01TESTUNTRACKED-a-note.md   # untracked
$ lane new spike
$ ls .lane/trees/spike/.lane/memory/src/auth.rs/
  NOT inherited
```

The queue was moved out of the worktree to escape a problem it only had because it was hidden there.

## The design

`lane capture` writes the note file. There is no second step.

```
.lane/memory/<path>/<ulid>-<slug>.md      written when the trailer is captured
```

The note is written with **no baseline**. That is the one thing the queue was genuinely protecting, and `store.rs` said so outright:

> Pending notes are resolved and fingerprinted here, never at write time, so a rebase can never leave a note anchored to a commit it rewrote.

Fingerprinting at capture would baseline every note against content a later rebase rewrites, and land it showing drift that never happened. So capture writes the finding and nothing else; `audit` takes the first baseline, after any rebase has moved what the anchor points at.

The checker already had the branch for this — *"Nothing to compare against yet, so this is a first fingerprint, not a change"* — but returned `adopted: false`, so `audit` computed the fingerprints and discarded them. It now adopts, which is what a test in the same file already asserted and nothing wired.

Three things follow, and they are the point:

- **An ordinary commit carries it.** `git add -A && git commit` is enough. No lane command stands between a finding and durability, so `git push` from a lane costs nothing.
- **It is readable at once.** `lane why` sees a note the moment it is recorded, where an unpromoted one is invisible today.
- **It is visible.** The note appears in `git status`, where the queue appears nowhere.

## Scope

In: `capture.rs`, `store.rs`, `audit.rs`, `cli.rs`, `worktree.rs`, `git.rs`, `scripts/test.sh`, `plans/README.md`.

Out: `syntax.rs`, `cow.rs`, `note.rs`'s file format — a note's shape does not change, only when it is written. Also out: `help.rs`, because a queue disappearing does not earn a clause in a help description.

## Steps

### Step 1: Capture writes the note

`capture.rs:180` calls `store::append_pending`. It calls `store::write_note` instead: choose `note_dir(root, path)/<ulid>-<slug>.md` and write the finding, with no fingerprints. The anchor is still qualified at capture, because disambiguating `fn verify` needs the file the author was looking at; only the baseline waits.

The checker's empty-baseline branch returns `adopted: true`, and `audit` persists that first baseline along with the span, since nothing else records where a note sits.

Dedup moves with it. `promote_pending` skips a record matching a live note on `(path, anchor, text, supersedes)`; the writer must make the same check before writing, or a re-run of `lane capture HEAD` doubles the note.

Failure stays non-fatal. Capture runs inside `post-commit`, and a commit must not fail because a note could not be written. Warn on stderr, as the hook already does when `lane` is off PATH.

### Step 2: `supersedes` is applied when it is written

`lane note replace` queues a `supersedes` id today, so the predecessor only reaches the attic at `audit`. Written directly, the retirement happens at write time, closing the window between asking and happening.

Two guards lived in that window and go with it: `note {id} already has a pending replacement` and `note {id} has a pending replacement and cannot be retired`. Both policed a race that no longer exists — `replace` resolves live notes only, so a second replacement of an already-retired note is refused for the plain reason that it is not live. `store::pending_supersedes` goes with them.

Keep the tolerance for a `supersedes` target that is not live: warn, clear the link, keep the finding. After this change it is reachable only from Step 4's fold, which is exactly where a target can have vanished.

### Step 3: The count becomes what is uncommitted

`store::pending_count` reads the queue. It becomes a count of notes not yet committed, from `git status --porcelain -- .lane/memory`, counting untracked and modified alike: a re-vouched note is memory that has not been shared either.

`ls` and `wt::losses` are the two consumers and both keep their wording — `N pending note(s)` still describes a note this lane has and nobody else does.

### Step 4: Fold any queue that exists, once

At the start of `audit`, for the worktree's own git dir:

- if `lane/pending.jsonl` exists, write each record as a note through Step 1's writer, then delete the file;
- unreadable records are skipped with a warning, as they are today;
- it must be idempotent and must not fail when the file is absent, which is the normal case after the first run.

This is the only code that may still name `PENDING`.

### Step 5: Delete the plumbing

`PENDING`, `pending_path`, `append_pending`, `promote_pending`, `PendingNote` and `pending_count`'s old body all go. Grep each and confirm no caller survives outside Step 4.

### Step 6: The pre-push hook loses its subject

**Only if #32 lands first.** This branch forked before it, so there is no `pre-push` hook here to remove.

`PRE_PUSH_BLOCK` warns that a hand push leaves the queue behind. With no queue it has nothing to say: a note is in the tree, and a push carries whatever was committed. Remove the hook and its spec, take `pre-push` out of `hook_specs`, and make `lane install hooks` strip the stale block from an existing `pre-push` file rather than leave it warning about a file that no longer exists.

## CASE TABLE — handle and test every row

```
W1.  Trailer, anchor resolves            → note written with the qualified anchor and NO
                                           baseline; the next audit takes the first one
W2.  Trailer, anchor does not resolve    → note written anyway; audit tiers it
                                           `unverifiable`. NOT dropped.
W3.  Trailer naming a path that does not
     exist                               → same as W2, and the commit still succeeds
W4.  Same trailer captured twice         → one note. Dedup is on
                                           (path, anchor, text, supersedes).
W5.  Trailer matching a RETIRED note in
     the attic                           → written. Retirement was a decision; recording it
                                           again is a new one.
W6.  Two trailers in one commit          → two notes
W7.  Commit made during a rebase         → hook skips; nothing written. The existing guard
                                           must keep working, or a replay writes duplicates.
W8.  `lane` not on PATH                  → hook says so, as today. Nothing written, nothing
                                           silently lost.
W9.  Capture in the primary worktree     → writes to the same place. There was never a lane
                                           to promote from.

S1.  replace, target live                → note written, predecessor moved to the attic now
S2.  replace, target already retired or
     gone                                → warn, clear the link, keep the note
S3.  Two lanes replace the SAME note     → both write. The attic move collides on one file,
                                           which is a real disagreement and must not be
                                           papered over.

C1.  Lane with 2 uncommitted notes       → `ls` says 2
C2.  Lane whose notes are committed      → `ls` says 0; they travel with the branch
C3.  Lane with a re-vouched (modified)
     note                                → counts as 1. Uncommitted is uncommitted.
C4.  Note committed, then edited again   → counts as 1

L1.  `lane new` while the parent holds
     an UNCOMMITTED note                 → NOT inherited. This is 018's bug and the reason
                                           the queue ever moved; §12 covers it and must keep
                                           passing.
L2.  `lane rm` with uncommitted notes    → `losses` reports them; refuses without --force
L3.  `git add -A && git commit` in a
     lane                                → the note is committed, with no lane command run
L4.  `git push` from a lane              → the note travels with the branch. Nothing is
                                           stranded, which is the whole point.
L5.  `lane push`                         → audit re-anchors, commits what is uncommitted,
                                           pushes
L6.  Code moved under a note between
     capture and landing                 → audit re-anchors it; lines and hashes update

M1.  Lane git dir holding a pending.jsonl → folded into notes once, file deleted
M2.  Primary git dir holding one          → same
M3.  `audit` run twice                    → second run is a no-op, no error
M4.  Repo that never had a queue          → no error
M5.  Queued record whose supersedes
     target is gone                       → note written without the link, warned
M6.  Queued record that is already a live
     note                                 → not written twice
```

## Done criteria

- A commit carrying a `Why:` trailer, followed by nothing but `git add -A && git commit`, puts the note in the tree. No lane command in the sequence.
- `lane why <path>` shows a note recorded one second earlier.
- `rg 'pending'` returns hits in Rust source only inside Step 4's fold.
- §12's "a fresh lane does not inherit the parent's queue" passes, rewritten to count uncommitted notes rather than queued ones.
- `cargo test`, `./scripts/test.sh`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all clean.

## STOP conditions

- If a fresh lane inherits an uncommitted note, stop. That is 018's bug returning, and it is the one thing the queue was protecting.
- If §12's inheritance assertion has to be deleted rather than rewritten, stop and report. The behaviour it covers still matters even though the mechanism changed.
- If capture can fail a commit, stop. A note that cannot be written is a warning; a commit that cannot be made is a broken repository.
- If the fold in Step 4 changes what `lane check` reports for any note in THIS repository, stop and compare tiers before and after.

## Migration

Automatic and one-way, in Step 4. A queue is drained on the next `audit`, which every `lane push` and `lane merge` runs. Nobody has to be told.

Repositories carrying a `pre-push` hook from #32 need Step 6's removal path, or they keep warning about a queue that no longer exists.
