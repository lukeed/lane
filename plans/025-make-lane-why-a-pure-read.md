# Plan 025: Stop counting reads, and make `lane why` a pure read

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat d21d877..HEAD -- crates/lane/src/store.rs crates/lane/src/audit.rs crates/lane/src/cli.rs`

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `7aa8d13`, 2026-08-20; re-verified at `d21d877`

## Why this matters

`lane why` is a write. It calls `store::bump_reads` (`cli.rs:749`), which increments a
counter in `.context/state/<branch>.json` — a tracked file. So inspecting memory dirties
the working tree, and on trunk that blocks the next landing:

```
$ lane done
error: git merge --ff-only drift-stays-flagged failed: error: Your local changes to the
following files would be overwritten by merge:
	.context/state/main.json
```

Hit for real. Worse, the memory commit had already landed in the lane by then, so trunk
was left unadvanced — the failure mode plan 016 was written to prevent, in a spot it does
not cover.

The counter's only consumer is the eviction ranking (`audit.rs:171-186`):

```rust
            let key = |n: &Note| {
                (
                    u8::from(!n.meta.pinned),
                    std::cmp::Reverse(counts.get(&n.meta.id).copied().unwrap_or(0)),
                    u8::from(!touched.contains(&n.path())),
                    store::tier_rank(...),
                    n.meta.id.clone(),
                )
            };
```

**The counter does not measure what the ranking needs.** Two reasons, and they are the
whole argument:

1. **Reading a bad note raises its keep-value.** You often read a note precisely to
   discover it is wrong. The act that should retire a note currently promotes it.
2. **`lane why` is the only lens into the store.** There is no browse command, no list, no
   inspect. Every look at memory — idle, diagnostic, or genuinely useful — goes through the
   one command that counts as an endorsement.

So the signal is contaminated by construction, and it costs a tracked write on every read.

**Relocating the counter does not fix this**, and would make things worse. Moving reads to
`.git/lane/` — untracked and per-worktree, as plan 018 did for the pending queue — would
make them per-machine. Eviction is a *committed* action: `store::evict` does a `git mv`
into `.context/attic/`. Machine-local counts would make two developers evict different
notes from the same repository, trading a merge conflict for nondeterministic history.
That option was considered and rejected for this reason; do not reintroduce it.

Removing reads from the ranking leaves `pinned > touched-by-this-lane > freshness > age` —
every term derived from committed data, so eviction stays deterministic. It also makes
`touched` the attention signal, which is the stronger one: changing the code a note is
about is evidence you needed it, and cannot be produced by browsing.

## Current state

Everything that touches the counter, verified at `7aa8d13`:

```
crates/lane/src/audit.rs:65     reads: previous.reads,          (carried through record_state)
crates/lane/src/audit.rs:82     let counts = store::read_counts(root);
crates/lane/src/audit.rs:175    std::cmp::Reverse(counts.get(&n.meta.id)...)
crates/lane/src/cli.rs:782      store::bump_reads(&root, &shown)?;
crates/lane/src/store.rs:100    pub reads: u32,                 (field on NoteState)
crates/lane/src/store.rs:165    pub fn read_counts(...)         (sums across all branches)
crates/lane/src/store.rs:196    pub fn bump_reads(...)
crates/lane/src/store.rs:563    let reads = have.reads + entry.reads;   (in roll_up)
crates/lane/src/store.rs:646    reads: 2,                       (test fixture)
crates/lane/src/store.rs:657    reads: 3,                       (test fixture)
```

`NoteState` is serialized with serde into `.context/state/<branch>.json`. This repository
is the only consumer and the tool is unreleased, so the field can simply go.

`lane done`'s preflight lives in `cli.rs`'s `done()`. Plan 016 added it; read it before
Step 3 so the new check matches its shape and its error style.

Conventions: one-line comments, and only where the reason is not obvious; `anyhow::Result`;
tests in `#[cfg(test)] mod tests` at file end. Commit subjects are Conventional Commits,
`type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline − 0, see Step 4 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 3 |
| Linux gates | `./scripts/check-linux.sh` | exit 0, from a lane and from the main checkout |

Record both baselines before starting; at `d21d877` they are 76 and 122.

Capture every exit code without a pipe (`cmd > /tmp/out 2>&1; echo $?`). Piping to `tail`
reports `tail`'s status and has already masked a failure in this project.

## Scope

**In scope**: `crates/lane/src/store.rs`, `crates/lane/src/audit.rs`,
`crates/lane/src/cli.rs`, `test_lane.sh`, `README.md`, `USAGE.md`.

**Out of scope**:
- Note content, the attic, the budget numbers (5 notes / 1200 chars).
- `roll_up`'s wider design — that a lane writes trunk's state file at all is a real and
  separate question. This plan only removes the `reads` term from its merge.
- Adding a browse or list command. Tempting, and a different decision.
- `crates/lane/assets/skill.md`, `AGENTS.md`.

## Steps

### Step 1: Make `lane why` a pure read

Delete the `store::bump_reads` call at `cli.rs:782` and delete `bump_reads` itself.

**Verify**: in a scratch repo with notes, `lane why <path>` twice, then
`git status --porcelain` → empty both times. This is the headline behaviour; confirm it
before continuing.

### Step 2: Drop reads from the ranking

Remove the `counts` term from the sort key in `audit.rs`, and remove the
`let counts = store::read_counts(root);` line. Delete `store::read_counts`.

The key becomes `pinned > touched > tier_rank > id`. Do not reorder the remaining terms.

**Verify**: `cargo test` compiles and passes; eviction still happens when over budget —
`test_lane.sh` section 12 exercises the budget and must still pass.

### Step 3: Remove the field, and keep old files readable

Remove `reads` from `NoteState` and from `roll_up`'s merge, and update the two test
fixtures at `store.rs:646` and `:657`.

No compatibility shim is needed. This repository is the only one using lane, and it is
unreleased at `0.1.0` — there are no state files in the wild to keep readable. Delete the
field outright. If `.context/state/main.json` in this repository still carries `reads`
keys after your change, the next `lane audit` rewrites the file without them; that is
expected and needs no migration code.

### Step 4: Preflight trunk's state before writing anything

Even with `lane why` pure, `lane audit` on trunk legitimately modifies
`.context/state/<trunk>.json`. `lane done` must refuse *before* committing memory, not
after, and with a lane-level message rather than raw git output.

Extend the existing preflight in `done()` so it also refuses when trunk's own state file is
dirty, naming the file and saying to commit or stash it. Match the wording style already
there.

**Verify**: in a scratch repo, `lane audit` on trunk to dirty its state, then `lane done`
from a lane → exits non-zero, prints a lane-level error naming
`.context/state/<trunk>.json`, and **the lane has no new memory commit** — check with
`git -C <lane> log --oneline -1` before and after.

### Step 5: Cover it

Add to `test_lane.sh` before the summary, numbered one past the last:

```bash
echo "== N. reading context does not modify the tree =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
git add -A .context && git commit -qm "memory" > /dev/null
"$LANE" why src/auth.rs > /dev/null
is "lane why leaves the tree clean" "$(git status --porcelain)" ""
"$LANE" why src/auth.rs > /dev/null
is "and is still clean when read twice" "$(git status --porcelain)" ""
"$LANE" audit --review none > /dev/null
is "an audit that changes nothing writes nothing" "$(git status --porcelain)" ""
```

Confirm the first two fail against the pre-Step-1 binary.

Also add a unit test asserting the eviction key no longer consults any read count — the
clearest form is a budget test where the note that would have won on reads is evicted on
the remaining terms.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 3.

### Step 6: Correct the documentation

`README.md` says audit ranks by `pinned > reads > touched-by-this-lane > freshness > age`
and calls it "An LRU on attention, needing no opinion about importance."
`USAGE.md` says "Reading bumps a counter that decides what survives the budget later" and
repeats the ranking in its Budget section.

Both are now wrong. Update the ranking in both, and replace the attention framing with what
is actually true: the notes that survive are the ones you pinned, the ones about code this
lane touched, then the freshest and oldest-first. One sentence somewhere useful: reading a
note is not a vote for it, because you often read one to find out it is wrong.

**Verify**: `grep -rn 'reads' README.md USAGE.md` → no matches describing the ranking.

## Done criteria

- [ ] `lane why` leaves `git status --porcelain` empty, on any number of reads
- [ ] `cargo test` passes; `./test_lane.sh` passes, baseline + 3
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` exit 0 from a lane and from the main checkout
- [ ] Eviction still occurs over budget, and is deterministic — same repo, same result
- [ ] `lane done` refuses on a dirty trunk state file *before* writing a memory commit
- [ ] `grep -rn 'bump_reads\|read_counts' crates/` → no matches
- [ ] `git diff --stat -- crates/lane/assets/skill.md AGENTS.md` → empty

## STOP conditions

- Removing the term makes eviction non-deterministic, or makes a budget test order-dependent.
- The preflight in Step 4 makes `lane done` refuse in a case that works today.
- You conclude reads should be kept and relocated to `.git/` instead. That was considered
  and rejected — it makes a committed eviction depend on machine-local counts. Report rather
  than implement it.

## Maintenance notes

- The invariant: **a read is not a write.** `lane why` is the only lens into the store, so
  it must be free to use. Any future feature that records something on read reintroduces
  both problems at once — the contaminated signal and the dirty tree.
- `touched-by-this-lane` is now the only behavioural term in the ranking. It is the better
  signal because it cannot be produced by looking: you have to change the code the note is
  about. If a future ranking wants more evidence, prefer signals of that kind over signals
  of attention.
- Still open, deliberately not fixed here: `roll_up` has the lane write trunk's state file
  (`store.rs:576`), so `.context/state/<trunk>.json` has more than one writer. That
  contradicts the "one branch, one file" claim the README makes for the store, and it is
  what turns an ordinary landing into a rebase conflict. Removing `reads` shrinks the
  surface but does not close it.
