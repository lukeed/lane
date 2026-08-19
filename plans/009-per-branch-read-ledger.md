# Plan 009: Bound the read ledger and make its counts survive a merge

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 43e404f..HEAD -- crates/lane/src/store.rs crates/lane/src/cli.rs`

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/003-merge-safe-notes.md
- **Category**: bug
- **Planned at**: commit `43e404f`, 2026-08-18

## Why this matters

`lane why` bumps a read counter, and that counter is the second key in the retention
ranking — `pinned > times read > touched by this lane > freshness > age`. It decides what
survives the budget. It is stored badly in two ways.

**Unbounded and committed.** One line per note shown, per invocation. 20 runs of
`lane why` over 3 notes produced 60 lines and 4 KB, and `lane done` commits `.context/`
wholesale. `AGENTS.md` tells agents to read context before every edit, so this file grows
all day.

**Counts do not survive a merge.** The file is `merge=union`; union keeps distinct lines
and collapses identical ones. Records are `{"id":...,"at":"<second-granularity ISO>"}`,
so two reads of the same note in the same second are byte-identical and merge into one.
Those 60 lines held only 9 distinct ones.

The fix follows the pattern the note store already uses: **one file per writer.** Notes
never conflict because each is its own file. Give each branch its own counts file and the
same property falls out.

## Current state

`crates/lane/src/store.rs`:

```rust
pub const READS: &str = ".reads.jsonl";
```

`bump_reads` appends one JSON line per id with `writeln!`. `read_counts` reads the file
and counts occurrences per id. The only consumer is `audit::run`, feeding the sort key.

`crates/lane/src/cli.rs`, `init()` writes both merge rules:

```rust
    append_line(&attrs, &format!("{CONTEXT_DIR}/**/*.md merge=union"))?;
    append_line(&attrs, &format!("{CONTEXT_DIR}/{READS} merge=union"))?;
```

Helpers available: `git::current_branch()`, `util::slug`, `store::load_notes`.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 2 |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 3 |

## Scope

**In scope**: `crates/lane/src/store.rs`, `cli.rs` (the `.gitattributes` rules and
section 12's expectation of two rules), `test_lane.sh`, `USAGE.md`'s Layout section.

**Out of scope**: the ranking itself; the `*.md` union rule, which is correct; making
reads a signal anywhere other than retention.

## Steps

### Step 1: One counts file per branch

```rust
pub const READS: &str = ".reads";   // directory: one counts file per branch
```

```rust
/// One file per branch, so two lanes never write the same bytes.
fn reads_file(root: &Path) -> PathBuf {
    let name = slug(&git::current_branch(), 60);
    root.join(CONTEXT_DIR).join(READS).join(format!("{name}.json"))
}
```

### Step 2: Store counts, not events

A count per note is bounded by the note count, which the budget already caps. An event log
is bounded by nothing.

`bump_reads` reads its own file, increments each id, drops ids no longer in the live store
(`load_notes`), and writes back with `serde_json::to_string_pretty` or sorted keys so the
committed diff is stable.

`read_counts` sums across every `*.json` in the directory, skipping unreadable files.
Order-independent by construction.

**Verify**: a unit test writing two branch files and asserting the sum.

### Step 3: Migrate an existing ledger once

If `.context/.reads.jsonl` exists, fold its per-id counts into this branch's file and
delete it. One-time, and the tool is pre-release.

**Verify**: a repo with a hand-written `.reads.jsonl` containing three lines for one id
ends up with that id at count 3 in `.context/.reads/<branch>.json`, and the old file gone.

### Step 4: Drop the union rule for the ledger

Per-branch files never collide, so the rule is not merely unnecessary — union-merging two
JSON objects produces invalid JSON. Reduce `init()` to the notes rule alone.

Section 12 of `test_lane.sh` asserts two `merge=union` rules; change it to one in the same
commit and say why in the message. Do not rewrite existing users' `.gitattributes`;
appending is safe, editing is not.

**Verify**: `lane init` in a fresh repo → `grep -c 'merge=union' .gitattributes` → `1`.

### Step 5: Update the documented layout

`USAGE.md` shows `.reads.jsonl    append-only, union-merged`. Replace with
`.reads/<branch>.json    read counts, one file per branch`.

**Verify**: `grep -c 'reads.jsonl' USAGE.md README.md` → `0` for both.

### Step 6: Cover it

Add to `test_lane.sh` before the summary:

```bash
echo "== N. read counts are bounded and merge-stable =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "n1" > /dev/null
"$LANE" note -p src/auth.rs -a "fn verify" "n2" > /dev/null
"$LANE" audit > /dev/null
for i in $(seq 1 20); do "$LANE" why src/auth.rs > /dev/null; done
is "one entry per note, not per read" \
   "$(python3 -c 'import json,glob;print(sum(len(json.load(open(f))) for f in glob.glob(".context/.reads/*.json")))')" "2"
git add -A && git commit -qm reads
git checkout -qb reader-a && "$LANE" why src/auth.rs > /dev/null && git add -A && git commit -qm a
git checkout -q main && git checkout -qb reader-b
"$LANE" why src/auth.rs > /dev/null && git add -A && git commit -qm b
git merge -q --no-edit reader-a > /dev/null 2>&1
is "two branches of reads merge cleanly" "$?" "0"
is "every ledger file is still valid json" \
   "$(python3 -c '
import json,glob
bad=0
for f in glob.glob(".context/.reads/*.json"):
    try: json.load(open(f))
    except Exception: bad+=1
print(bad)')" "0"
```

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 3.

## Done criteria

- [ ] `cargo test` passes, baseline + 2; `./test_lane.sh` passes, baseline + 3
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `grep -rc 'reads.jsonl' crates/lane/src/ USAGE.md README.md` → `0` for all
- [ ] After 20 `lane why` runs over 2 notes, `.context/.reads/` holds one file with 2 keys
- [ ] `lane init` in a fresh repo writes exactly one `merge=union` rule
- [ ] `plans/README.md` row updated

## STOP conditions

- Two branches produce the same filename. `slug` maps `feature/x` and `feature-x` to the
  same name. Report it — the fix is a short hash of the full branch name, but changing the
  scheme mid-plan invalidates the migration.
- The budget assertion in section 12 changes result. It depends on `why` bumping reads and
  `audit` reading them back, which is exactly the seam this moves. Report the actual counts.
- Pruning ids in `bump_reads` calls `load_notes` often enough to be slow. It runs once per
  interactive `lane why`; measure before assuming, and do not drop the prune — the
  unbounded file is the bug.

## Maintenance notes

- The invariant: **one writer per file.** `.context/` now has two families holding it —
  one file per note, one per branch — and that is what lets the whole store be committed
  without a lock. New per-repo state reintroduces conflicts; make it per-writer.
- Dead branches leave their counts file behind forever, deliberately: deleting a file
  another branch may have modified is a delete/modify conflict, worse than a few kilobytes.
  If the directory ever gets large, garbage-collect it in an explicit `lane gc` on trunk,
  not as a side effect of audit.
- The step 3 migration deletes `.context/.reads.jsonl`; during the transition a branch that
  migrated and one that still appends can produce a delete/modify conflict on that path.
  It resolves by taking the deletion.
