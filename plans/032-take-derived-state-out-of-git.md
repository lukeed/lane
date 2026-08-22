# Plan 032: Take derived state out of git, and make a `holds` a decision

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat 7986a2c..HEAD -- crates/lane/src/store.rs crates/lane/src/audit.rs crates/lane/src/cli.rs crates/lane/src/worktree.rs`

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: design + bug
- **Planned at**: commit `7986a2c`, 2026-08-22. Every line number below read at that
  commit.

## Why this matters

`.lane/branch/<slug>/state.json` is derived data that got committed. Every open problem
in the store is downstream of that one decision, and the tool grew four mechanisms to
defend it: a per-branch split so two writers rarely touch one file, a landing lock so the
one place they do is serialized, a rollup so the split does not accumulate, and a garbage
collector so the rollup's leftovers get swept.

**1. The garbage collector destroys decisions.** `gc_state` (`store.rs:645`) deletes
`state.json` for any branch with no local ref. `lane done` then stages that deletion with
`git add .lane` (`cli.rs:1029`). A `lane holds` is stored *only* in state, so a
confirmation vanishes the first time anyone audits without a local ref for the branch that
made it. Plan 013 logged this as a survivable STOP condition, on the assumption it needed a
hand-deleted branch. It does not: a merged pull request produces exactly that state on
every collaborator's machine.

**2. The lock's premise is local-only.** `roll_up` (`store.rs:598`) runs inside the lane's
worktree, rewrites *trunk's* state file there, and commits it on the lane's branch. That is
safe only because the lock spans the fold and the fast-forward, so the next lane rebases
onto an already-folded trunk. Nothing serializes two clones. Two pull requests opened from
the same base both rewrite `.lane/branch/main/state.json` and the second one conflicts, on
JSON, with no merge driver. Resolving it "take ours" silently discards the other side's
`holds`.

**3. Nothing folds when the merge happens elsewhere.** There is no server side. A pull
request merged on GitHub leaves `.lane/branch/<branch>/` on trunk forever — `gc_state`
eventually deletes the state half (see 1) and is explicitly documented never to touch the
log half, so `log.jsonl` accumulates one orphan per merged branch.

**4. It caused plan 027.** That bug was two views of one state — `Checker` reading the
merged view, `record_state` reading the branch's own file. The two views exist only because
state was split per branch to dodge merge conflicts.

**The load-bearing observation**: `state.json` is not a cache. `Checker::check`
(`store.rs:404`) resolves the span and computes `src.hashes(...)` unconditionally on every
run; the stored entry is read for its baseline and nothing else. Its own doc comment calls
it "Disposable: losing it costs one recompute, never a wrong answer" (`store.rs:88`) — and
that is true of every field except the baseline behind a `holds`, which is not derived at
all. It is a human judgment with no other home.

Plan 013 already wrote the rule that settles this:

> A new field that changes over time goes in `state/`; a decision worth keeping goes in
> `log/`; a fact that is true forever goes in the note.

A `holds` is a decision. Move it to the log and the baseline becomes committed, immutable
data end to end — the note's creation fingerprint, overridden by the latest re-confirmation.
Then `state.json` has no remaining field, and gets deleted rather than relocated.

**What that removes**: the per-branch split, the rollup, the collector, the merged-versus-own
view, the conflict surface, and the four-mechanism defence around a file that should never
have been in git. `.lane/` becomes `memory/`, `attic/`, one union-merged `log.jsonl`, and
`trees/`.

**What it enables**: a pull-request workflow with no new machinery. Notes are pure adds, the
log is union-merged, and nothing derived is shared, so there is nothing to fold and nothing
to run after the merge.

**The honest cost**: a `holds` and an automatic re-baseline become commits to `.lane/log.jsonl`
rather than writes to an ignored file. That is correct — both are store updates — but it
means resolving drift now dirties the tree. Accept it or stop here.

## Current state

Verified at `7986a2c`:

```
crates/lane/src/store.rs:88-118    NoteState, State, branch_dir, state_file_for, log_file_for
crates/lane/src/store.rs:122-186   read/write/all_state, load_state, save_state
crates/lane/src/store.rs:189       append_log — per-branch path
crates/lane/src/store.rs:370-404   Checker { state: load_state(root) }
crates/lane/src/store.rs:590-596   own_state
crates/lane/src/store.rs:598-638   roll_up
crates/lane/src/store.rs:640-643   discard_branch_files
crates/lane/src/store.rs:645-...   gc_state
crates/lane/src/audit.rs:29-58     record_state
crates/lane/src/audit.rs:60-91     refresh_holds, holds
crates/lane/src/audit.rs:126       state = own_state(root)   ← the 027 asymmetry
crates/lane/src/audit.rs:198-202   state.retain + save_state + gc_state
crates/lane/src/cli.rs:936-947     done's trunk-state-dirty precheck
crates/lane/src/cli.rs:1020        roll_up call
crates/lane/src/cli.rs:996-1000    rm → discard_branch_files
crates/lane/src/worktree.rs:270    worktree add always passes -b
.gitattributes                     .lane/branch/*/log.jsonl merge=union
```

This repository's own store: 26 state entries, all `fresh`; 14 log lines, of which zero are
`kind: "holds"` — but **ten** are `kind: "verdict"`, the deleted reviewer's vocabulary, and
six of those say `verdict: "holds"`. See step 9: the migration is not the no-op that a
`kind` count suggests.

## Commands you will need

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
./test_lane.sh
jq -c 'select(.kind=="holds")' .lane/log.jsonl
```

## Scope

In scope: `store.rs`, `audit.rs`, `cli.rs`, `worktree.rs`, `.gitattributes`,
`test_lane.sh`, `README.md`, `crates/lane/assets/skill.md`, the `AGENTS.md` protocol text,
`www/src`, and this repository's own `.lane/`.

Out of scope: the landing lock (it still serializes trunk's ref and the commit, and stays
as-is), `pending.jsonl`, the CoW layer, `syntax.rs`, and any migration path for other
repositories — there are none.

## Steps

### Step 1: One union-merged log

Collapse `.lane/branch/<slug>/log.jsonl` to `.lane/log.jsonl`. Replace `log_file_for` with
a single `log_path(root)`; drop `branch_dir`.

Union merge is built for concurrent appends from independent branches, which is now the
only writer pattern. Update `.gitattributes` to `.lane/log.jsonl merge=union` and make
`lane init` write that rule.

Every record grows `branch` so provenance survives the collapse — `evict` (`store.rs:346`)
does not carry one today.

**Verify**: `lane note` + an audit that evicts → one `.lane/log.jsonl`, every line has
`branch`, and `.lane/branch/` is not recreated.

### Step 2: `holds` becomes a record, not a state write

`refresh_holds` (`audit.rs:60`) currently mutates a `NoteState`. Replace it with a log
append carrying the fingerprint it confirmed:

```json
{"at":"...","kind":"holds","id":"...","path":"...","anchor":"...","branch":"...",
 "sig":"...","body_hash":"...","raw_hash":"...","norm":"1"}
```

The fingerprint fields are what make it a baseline rather than an audit trail. Without
them the record says a note was vouched for but not for which shape of the code, which is
the whole content of a `holds`.

**Verify**: `lane holds <id>` on a drifted note → one new line in `.lane/log.jsonl` with all
four fingerprint fields, and `lane check` reports the note fresh on a second run.

### Step 3: Resolve the baseline from committed data

`Checker::new` (`store.rs:374`) loads `load_state(root)`. Replace that with a map built by
scanning `.lane/log.jsonl` for `holds` and `rebaseline` records, keyed by note id, newest
`at` winning. Ties broken by later position in the file; two re-confirmations of one note
inside one second is degenerate and costs a recompute at worst.

`check()` then resolves its baseline as: latest re-confirmation for the id, else the note's
creation fingerprint from frontmatter. Delete the `state` field and every read of it.

This is where plan 027's asymmetry dies. There is one view, so `record_state`'s "retain the
fingerprint we compared against" has nothing to disagree with — an unresolved drift compares
against committed data on every machine, deterministically.

**Verify**: reproduce 027's case — note created on trunk, span edited in a lane, `lane done` —
and confirm trunk still reports 1 drifted afterward. Then delete `.lane/log.jsonl`'s holds
lines and confirm the note falls back to its creation fingerprint rather than reading fresh.

### Step 4: A norm bump re-baselines into the log

Without state there is nowhere for `rebaselined` (`store.rs:459`) to write the adopted
fingerprint, so every audit after a `NORM_VERSION` bump would re-report the same notes
forever. Have audit append a `rebaseline` record per affected note, same shape as `holds`
and the same fields, distinct `kind`.

A distinct kind matters: plan 031 removed the model so that every note and every resolution
is attributable to whoever typed the command. A machine's automatic re-anchor must not read
in the log as a person's vouch.

**Verify**: bump `NORM_VERSION` by hand, audit twice → the first run reports the re-baseline
count and writes the records, the second reports zero. Revert the bump.

### Step 5: Delete the state layer

Remove `NoteState`, `State`, `read_state_file`, `write_state_file`, `all_state`,
`load_state`, `save_state`, `own_state`, `state_file`, `state_file_for`, `roll_up`,
`discard_branch_files`, `gc_state`, and `record_state`. Drop the `roll_up` call
(`cli.rs:1020`) and the trunk-state-dirty precheck (`cli.rs:936-947`).

`audit::run` keeps its in-run tier map for `eviction_key` and the report, and writes nothing
but notes, attic moves and log lines. `rm` loses its `discard_branch_files` call and with it
the landed-versus-discarded distinction — there is no longer anything per-branch to discard.

**Verify**: `rg 'state\.json|NoteState|roll_up|gc_state' crates/` → no hits outside tests
that assert their absence. `lane done` on a clean lane still lands, and `.lane/` afterward
holds only `memory/`, `attic/`, `log.jsonl` and `trees/`.

### Step 6: `lane done --no-merge`, and the landing marker

`--no-merge` stops after the memory commit: rebase, audit, commit, print the push line. No
fast-forward, no worktree removal. Default is unchanged — `done` lands.

Both paths append one marker at the point memory is committed:

```json
{"at":"...","kind":"landing","branch":"foobar"}
```

The record means the lane's memory was finalized. Its presence *in trunk's copy of the log*
means the branch merged, whichever of GitHub's three buttons was pressed, because it is tree
content rather than commit identity. That is the signal git itself cannot give: `git branch -d`
is ancestor-only and refuses after a squash or rebase merge even when the trees are identical.

Add `--ff` as the explicit spelling of the default. Do **not** add `--rebase` — the rebase
happens in every mode, so the flag would name the wrong half of the operation.

**Verify**: `lane done --no-merge` → trunk unmoved, lane still present, branch has one
`lane: sync` commit whose log contains the landing record.

### Step 7: `lane sweep`, and `lane ls` state

`lane ls` gains a state column, read from `git show <trunk>:.lane/log.jsonl`: `landed` when
that log holds a landing record for the lane's branch, else `open`. A week passes between
preparing a lane and its pull request merging; `ls` is where the user goes to remember the
lane exists, so it has to say so unprompted.

`lane sweep` removes every landed lane. Skip dirty lanes and name them. Nothing implicit:
`new`, `done` and `audit` never sweep.

**`wt::remove`'s `-d` cannot be the safety check, which this plan originally assumed.**
`-d` is ancestor-only, so it refuses every squash and rebase merge — the two cases sweep
exists for. Worse, `wt::remove` deletes the worktree before it tries the branch, so a late
refusal has already thrown the work away. Sweep needs its own gate, run first:
`merge-base --is-ancestor`, then a comparison of the branch's cumulative diff against trunk
by patch-id. That answers all three merge buttons, and only then is `wt::remove` called with
force. The refusal the ecosystem's `git branch -vv | awk '/gone]/' | xargs git branch -D`
throws away is kept, and made to work where `-d` does not.

An empty probe needs its own branch: when the branch is already an ancestor the synthesised
commit has no diff, no patch-id, and `git cherry` calls it unmerged.

When trunk has no landing record and `origin/<trunk>` is ahead, say so and stop rather than
fetching. Lane has never touched a remote and this is not the place to start.

**Verify**: prepare a lane, merge its branch into trunk by hand with `--squash`, then
`lane ls` → `landed`, and `lane sweep` → removed. Repeat with an uncommitted change in the
lane → skipped, named, exit non-zero.

### Step 8: `lane new` adopts an existing branch

`worktree.rs:270` and its sibling always pass `-b`, so there is no supported way to put a
lane on a fetched pull-request branch — which blocks landing a collaborator's work and
blocks getting back into a lane that was swept early. When `refs/heads/<name>` already
exists, omit `-b`; error when `--base` is given alongside an existing branch, and when the
branch is checked out in another worktree.

**Verify**: `git branch foo && lane new foo` → a lane on the existing branch, no new commit.
`lane new foo --base main` → one `error:` line.

### Step 9: Migrate this repository

```bash
git mv .lane/branch/main/log.jsonl .lane/log.jsonl
git rm -r --cached .lane/branch && rm -rf .lane/branch
```

Backfill `branch` on the fourteen existing log lines so they satisfy step 1's invariant.

**This step's STOP condition fired, and the answer is to proceed.** Ten of the twenty-six
state entries hold a `body_hash` and `raw_hash` that have advanced past the note's creation
fingerprint, `sig` identical in every case. They split two ways:

- **Six have a matching `verdict: "holds"` record** in the log — the reviewer plan 031
  deleted, vouching for spans in `cli.rs#PROTOCOL`, `worktree.rs#fn create`,
  `scenes.rs#@file`, `test_lane.sh#@file`, `AGENTS.md#@file` and `skill.md#@file`.
- **Four have no record at all**: `cli.rs#write_protocol`, `cli.rs#POST_COMMIT_BLOCK`, and
  both notes on `audit.rs#run`. These are residue from the behaviour plan 024 fixed — an
  audit that "rewrites every note's fingerprint before review, so drift is reported once and
  never again", which is a note in this very store.

Lift neither. The four are the output of a bug and were never confirmed by anyone. The six
are a model's judgment, and carrying them into a `holds` record — the shape a person's vouch
takes — would restore through migration exactly the authority plan 031 removed, unmarked and
indistinguishable from a human decision. That is reason 2 of plan 031, re-created.

The consequence is visible and intended: ten notes surface as drifted on the first
`lane check` after this lands. They are drifted. The old state file was hiding it, and
resolving them with `lane holds`, `lane note --supersedes` or deletion is the loop plan 031
was written to make possible.

**Verify**: `jq -c 'select(.branch==null)' .lane/log.jsonl` → empty. `lane check` reports the
ten notes above and no others beyond what this branch's own edits drifted.

### Step 10: Cover it, then say it

`test_lane.sh` asserts the old layout in roughly fifteen places, including section 18's
"done rolls the lane's state into trunk's" — that assertion is now false by design and its
section becomes the pull-request round trip: prepare, merge by hand, `ls`, `sweep`.

Then the docs: `README.md`'s memory layout block and the `done` line, `assets/skill.md:17`,
the `AGENTS.md` protocol text, and `www/src`.

**Verify**: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`./test_lane.sh` all clean; no doc mentions `state.json` or `.lane/branch/`.

## Done criteria

- [x] `.lane/` holds only `memory/`, `attic/`, `log.jsonl`, `trees/`
- [x] `rg 'state\.json|NoteState|roll_up|gc_state|discard_branch_files' crates/` → no hits
- [x] A `holds` survives a landing, and survives a fresh clone
- [x] 027's case still reports drift on trunk after `lane done`
- [x] Two lanes prepared from the same base merge in either order with no conflict
- [x] `lane done --no-merge` leaves trunk unmoved and the lane in place
- [x] `lane ls` shows `landed` after a squash merge; `lane sweep` removes it
- [x] `lane sweep` refuses a dirty lane and one with commits trunk lacks
- [x] `lane new <existing-branch>` works
- [x] `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
      `./test_lane.sh` all clean

Measured: `cargo test` 86 → 85 (six state-file tests deleted, five written);
`./test_lane.sh` 120 → 150.

## Found in review, after the steps above

Three bugs the step-level verifications missed, all in `sweep`'s neighbourhood. Each has a
regression in `test_lane.sh`.

1. **The marker named the branch, and branch names are reused.** `fix` twice in a week is
   normal; the second lane matched the first one's marker, reported `landed`, and swept
   clean because a lane with no commits is vacuously contained in trunk. Fixed by stamping
   a ULID per lane at `lane new` into `$GIT_DIR/lane/id` — per-worktree, never committed,
   the same resolution `pending.jsonl` uses — and keying the marker on it. A lane with no
   id is never swept: `sweep` is destructive, so an unrecognised lane fails safe.
2. **`sweep` and `rm` deleted the directory the caller was standing in.** `done` guards
   this by chdir'ing to the root first; the other two had no reason to and so did not.
   That is plan 006's failure — a shell left in a path that no longer exists. The guard
   belongs in `wt::remove`, which all three go through.
3. **A confirmation record with no fingerprint silenced a note forever.** `check` reads an
   empty baseline as a first fingerprint and returns fresh, so one truncated log line would
   mark a note permanently current. `confirmations` now skips records carrying no
   fingerprint, falling back to the note's own.

Also verified by hand, not covered by a step: `lane new <existing-branch> --dirty` carries
exactly one modified file, which is the invariant section 3 asserts for a new branch.

## STOP conditions

- ~~A `holds` record cannot reproduce the baseline `state.json` held, for any note in this
  repository.~~ **Fired.** Ten of twenty-six cannot, because they were never vouched by a
  person: six came from the deleted reviewer and four from a fixed bug. State was carrying
  something underived, and the answer is to drop it rather than launder it. See step 9.
- Union merge on a single `log.jsonl` produces a conflict when two branches append. It
  should not; that is the one thing union merge exists for. If it does, report the sequence
  before falling back to a per-branch split.
- Removing `state.json` makes an audit measurably slower. It should not — `check()` never
  read it to skip work. If it does, something was memoizing through it that this plan has
  not found.
- `lane sweep` proposes removing a lane whose work is not in trunk. Report the merge
  strategy and the marker's contents; the detection is wrong, not the safety check.

## Rejected

- **A local `state.json` under `$GIT_DIR`.** The first shape of this plan. Pointless once
  the baseline is committed: `check()` recomputes the current fingerprint every run
  regardless, so a local cache would store a baseline it already has and an invalidation
  problem it does not need.
- **An append-only state file.** Reconsidered from `plans/README.md`, where it was rejected
  because the landing lock already serialized the only concurrent writer. That premise is
  true per clone and false across clones. But it survives only to hold derived data that
  should not be shared at all, and it brings back unbounded growth plus compaction. Ejecting
  beats reshaping.
- **A migration command.** No repository but this one has a store. `lane init` repairing a
  stale layout was considered and dropped for the same reason; nothing reads `.lane/branch/`
  after this plan, so a leftover is inert.
- **Inverting `lane done`'s default to prepare.** Costs ~15 call sites in `test_lane.sh`,
  the skill, the protocol text, the README headline and the tour, to make the solo loop —
  the thing the tool is sold on — grow a mandatory flag. `--no-merge` is the same capability
  without the breaking change.
- **Detecting a merge by patch-id.** `git cherry` covers a rebase merge and a single-commit
  squash, and fails on a two-commit squash; the commit-tree probe that does work is
  archaeology lane does not need when it can write down a marker that survives the rewrite.
  *Partly reversed in step 7*: the marker stays the detector, but the probe earns its place
  as the safety gate, because "did this land" and "is anything on this branch missing from
  trunk" are different questions and only the second one needs to look at the work.
- **Deleting the landing lock.** It no longer protects state, but it still serializes trunk's
  ref and the memory commit between two local lanes. Out of scope.
