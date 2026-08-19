# Plan 010: Clear the three small things that mislead

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 6dc6647..HEAD -- crates/lane/src/cli.rs crates/lane/src/audit.rs crates/lane/src/worktree.rs`

## Status

- **Priority**: P3
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `6dc6647`, 2026-08-18

## Why this matters

Three small things, each making a reader believe something untrue. None will cost a user
work; together they are the difference between deliberate and drifted.

### 1. A gitignore rule that can never match

`cli::init` writes `.lanes-*` into the repository's `.gitignore`, but lanes are created
*outside* the repository — `worktree::lanes_dir` returns
`root.parent()/".lanes-<root name>"`. Git never sees them. The rule tells a reader lanes
might land inside the repo.

### 2. A flag that cannot succeed

`lane done --allow-dirty` skips the dirty check, then hits `git rebase`, which refuses a
dirty index or dirty tracked files regardless. The flag can only turn a clear refusal into
an obscure one. Untracked files, meanwhile, do not block a rebase and should never have
been refused: `worktree::is_dirty` counts them because it calls `git status --porcelain`
with no `--untracked-files=no`.

### 3. A summary that describes the wrong moment

`audit::report` prints tier counts gathered before the reviewer ran, so a note the model
judged `holds` is refreshed to `fresh` but still counted under `body-drift`:

```
memory: +2 new, 7 fresh, 1 body-drift, 0 signature-changed, 0 missing
  reviewed 1 drifted note(s) via anthropic(claude-haiku-4-5-20251001)
  holds         src/auth.rs#fn verify
```

**Considered and rejected**: recounting after review. The pre-review numbers honestly
describe what the hash check found, and they feed `--json`'s `checked` key where a stable
meaning matters more than a tidy one. Say which moment they describe instead.

## Current state

```rust
    append_line(&ignore, PENDING)?;
    append_line(&ignore, ".lanes-*")?;
```

```rust
pub fn is_dirty(path: &Path) -> bool {
    !try_git(&["status", "--porcelain"], Some(path)).trim().is_empty()
}
```

```rust
    writeln!(
        w,
        "memory: +{} new, {} fresh, {} body-drift, {} signature-changed, {} missing",
```

`is_dirty` has two callers: `cli::done`'s guard and `cli::ls`'s display column.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline, all pass |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline |

## Scope

**In scope**: `crates/lane/src/cli.rs`, `audit.rs`, `worktree.rs`, `test_lane.sh` and
`USAGE.md` where they mention `--allow-dirty`.

**Out of scope**: the `.wt/pending.jsonl` gitignore line, which is live; the JSON shape of
`lane audit --json`; the ordering inside `done`.

## Steps

### Step 1: Stop writing a rule that cannot match

Delete `append_line(&ignore, ".lanes-*")?;`. Do not remove the line from any existing
repo's `.gitignore` — appending is safe, editing a user's file is not.

**Verify**: `lane init` in a fresh scratch repo → `grep -c 'lanes-' .gitignore` → `0`,
`grep -c '.wt/pending.jsonl' .gitignore` → `1`.

### Step 2: Remove `--allow-dirty`, and stop counting untracked files as dirt

1. Narrow `is_dirty`:

```rust
/// Tracked changes only: untracked files do not block a rebase.
pub fn is_dirty(path: &Path) -> bool {
    !try_git(&["status", "--porcelain", "--untracked-files=no"], Some(path))
        .trim()
        .is_empty()
}
```

2. Delete the `allow_dirty` field from the `Done` variant and its thread through `run()`
   and `done()`.

3. Replace the guard's message with one that says why stashing will not help:

```rust
    if wt::is_dirty(&lane_path) {
        eprintln!("error: lane has uncommitted changes; commit or stash first, the rebase will refuse them either way");
        return Ok(1);
    }
```

4. In `USAGE.md`, rewrite the "**`lane is dirty`**" entry: commit or stash, and untracked
   files are fine and need no stashing. Do not mention `--allow-dirty`.

**Verify**:
- `grep -rc 'allow.dirty\|allow_dirty' crates/lane/src/ USAGE.md README.md` → `0` everywhere
- `lane done --allow-dirty 2>&1 | grep -c 'unexpected argument'` → `1`

### Step 3: Say which moment the counts describe

```rust
    // Counted before the reviewer ran: what the hash check found, not what was done about it.
    writeln!(
        w,
        "memory: +{} new; checked {}: {} fresh, {} body-drift, {} signature-changed, {} missing",
        out.created.len(),
        out.stats.values().sum::<usize>(),
        ...
    )?;
```

**Verify**: `lane audit | head -1` matches `^memory: \+[0-9]+ new; checked [0-9]+:`.
Check `grep -n 'memory:' test_lane.sh` before and after; update any assertion in the same
commit.

## Done criteria

- [ ] `./test_lane.sh` passes at the baseline count; `cargo test` unchanged and passing
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `grep -c 'lanes-\*' crates/lane/src/cli.rs` → `0`
- [ ] `grep -rc 'allow_dirty' crates/lane/src/` → `0`
- [ ] `lane done` in a lane with only untracked files gets past the dirty guard
- [ ] `plans/README.md` row updated

## STOP conditions

- Narrowing `is_dirty` breaks an existing assertion. `cli::ls` prints `dirty`/`clean` for
  display only; section 2 asserts a clean lane. If a test depended on untracked files
  reading as dirty, report it — the change may be right but the call is the maintainer's.
- Removing `--allow-dirty` breaks something outside the in-scope files. `grep -rn 'allow'`
  before deleting.

## Maintenance notes

- Item 3's decision — report pre-review counts and label them — should hold even if the
  output grows. If someone wants post-review totals, add a second line rather than
  changing what the first means; `--json`'s `checked` key shares the number.
- Deferred: `cow::probe` creates its scratch directory inside the repo root
  (`tempfile::tempdir_in`), so an interrupted `lane new` can leave an untracked
  `.lane-probeXXXX/` behind. Low severity, but it is the last place `lane` writes into the
  user's worktree without being asked.
