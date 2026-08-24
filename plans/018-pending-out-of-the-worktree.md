# Plan 018: Keep the pending queue out of the worktree

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat 365492a..HEAD -- crates/lane/src/store.rs crates/lane/src/cli.rs scripts/test.sh`

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `365492a`, 2026-08-19

## Why this matters

`lane` stores unpromoted notes in `.wt/pending.jsonl`. `.wt` is the name this tool had
before it was called `lane`; nothing else in the codebase uses it. Two things follow, and
both were reproduced end to end before this plan was written.

**A stray top-level directory.** `lane init` writes `.wt/pending.jsonl` into the user's
`.gitignore`, and the first `lane note` creates a `.wt/` directory beside `.context/`.
Nothing documents it. `README.md` says `.context/` "holds two kinds of file and nothing
else", and `USAGE.md`'s Layout section shows `.context/`, `.gitattributes` and `AGENTS.md`
and no `.wt/`. Both are wrong today.

**A lane inherits work it did not do.** The queue is gitignored, and `lane new` clones
everything git ignores by reference. So a lane opened while the parent has an unpromoted
note starts out holding that note:

```
$ lane note -p src/auth.rs -a "fn verify" "must stay constant-time"
$ lane new spike
$ lane ls
spike                spike                    clean  1 pending note(s)
```

The lane recorded nothing. It will promote the parent's note into its own commit at
`lane done`, and the parent's copy stays queued. The note's own `branch` field is stamped
at write time so the *note* is still attributed correctly, and `promote_pending` dedupes
after the rebase so no duplicate note file is produced — this is a lie in `lane ls` and a
note landing from the wrong lane, not data loss. Verified: two lanes opened over one
pending note both land, and one note file exists afterwards.

The fix for both is one constant. Git already gives every worktree a private directory,
and `hooks_dir()` in `cli.rs:239` already uses the idiom to find it.

## The design

```rust
pub const PENDING: &str = "lane/pending.jsonl";
```

resolved through `git rev-parse --git-path`, which is per-worktree for any path git does
not consider common. Verified:

```
main worktree:    <repo>/.git/lane/pending.jsonl
linked worktree:  <repo>/.git/worktrees/spike/lane/pending.jsonl
```

That buys three things at once: a lane cannot inherit the parent's queue (`lane new`
already filters `.git` out of the entries it clones), the queue can never be committed,
and `lane init` no longer has any reason to touch `.gitignore`.

**Not a goal**: carrying pending notes into a lane on `--dirty`. Today that happens by
accident, through the ignored-file clone, not because anyone decided it. If it turns out
to be wanted it is a separate change with its own flag semantics.

No migration. The tool is unreleased at `0.1.0`; a leftover `.wt/` in a developer's tree
is theirs to delete.

## Current state

`crates/lane/src/store.rs:18`

```rust
pub const PENDING: &str = ".wt/pending.jsonl";
```

Three call sites join it onto a path the caller supplies:

- `store.rs:220` — `promote_pending(root)`: `let pending = root.join(PENDING);`
- `store.rs:463` — `append_pending(root, rec)`: `let path = root.join(PENDING);`
  (creates the parent directory, then appends one JSON line)
- `store.rs:476` — `pending_count(worktree)`: `std::fs::read_to_string(worktree.join(PENDING))`

Their callers: `audit.rs:34`, `capture.rs:165`, `cli.rs:394` (the `lane ls` column) and
`cli.rs:427` (`lane note`).

`crates/lane/src/cli.rs:331-332`, the only `.gitignore` write in `init()`:

```rust
    let ignore = root.join(".gitignore");
    append_line(&ignore, PENDING)?;
```

The exemplar to follow is `crates/lane/src/cli.rs:239`:

```rust
fn hooks_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(git(
        &["rev-parse", "--git-path", "hooks"],
        None,
    )?))
}
```

`crates/lane/src/git.rs` gives you `git` (fails loudly), `try_git` (failure is empty
string) and `git_ok` (exit status only), each taking `cwd: Option<&Path>`.

Conventions: one-line comments only, and only where the reason is not obvious from the
code; `anyhow::Result`; tests in `#[cfg(test)] mod tests` at the end of the file. Commit
subjects are Conventional Commits, one short clause, no scope — `fix: keep the pending
queue out of the worktree`.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | 53 passing, unchanged |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./scripts/test.sh` | `failed: 0`, 86 assertions + 1 |
| Linux gates | `./scripts/check-linux.sh` | all pass |

Baselines at `365492a` are 53 and 86, confirmed by running them.

## Scope

**In scope**: `crates/lane/src/store.rs`, `crates/lane/src/cli.rs`, `scripts/test.sh`,
`explainer.md`, `USAGE.md`, and this repository's own `.gitignore`.

**Out of scope**:
- What a pending note contains, when it is promoted, or how it dedupes.
- `--dirty` semantics.
- Any other use of `append_line` — the `.gitattributes` union rule stays exactly as it is.
- Adding a migration path for an existing `.wt/`.
- `README.md`: its `.context/` layout claim becomes true once this lands, so it needs
  no edit. Do not rewrite it.

## Steps

### Step 1: Resolve the queue through git

In `store.rs`, change the constant and add a resolver next to it:

```rust
pub const PENDING: &str = "lane/pending.jsonl";
```

```rust
/// Per-worktree: git resolves an uncommon path inside .git/worktrees/<name> for a lane,
/// so a lane cannot inherit the queue its parent has not promoted yet.
pub fn pending_path(worktree: &Path) -> PathBuf {
    let resolved = git::try_git(
        &["rev-parse", "--path-format=absolute", "--git-path", PENDING],
        Some(worktree),
    );
    if resolved.is_empty() {
        worktree.join(".git").join(PENDING)
    } else {
        PathBuf::from(resolved)
    }
}
```

The fallback is what keeps `promoting_the_same_pending_note_twice_creates_one_note`
(`store.rs:853`) working — it operates on a bare `tempfile::tempdir()` that is not a git
repository, so `git rev-parse` fails there and returns empty.

Replace all three `join(PENDING)` call sites with `pending_path(...)`. `append_pending`
already does `create_dir_all` on the parent; keep it, it is now what creates `.git/lane/`.

**Verify**: `cargo test` → 53 passing. `cargo clippy --all-targets` → clean.

### Step 2: Stop `init` from writing to `.gitignore`

Delete these two lines from `init()` in `cli.rs`:

```rust
    let ignore = root.join(".gitignore");
    append_line(&ignore, PENDING)?;
```

Drop `PENDING` from the `use crate::store::{...}` import at `cli.rs:7`. `append_line` is
still used for `.gitattributes` — leave it alone.

**Verify**: `cargo build` → no unused-import warning. In a scratch git repo, `lane init`
followed by `git status --porcelain` shows `.gitignore` unmodified.

### Step 3: Prove the lane no longer inherits the queue

`scripts/test.sh` currently asserts the opposite at line 213:

```bash
is "pending notes are ignored" "$(grep -c '.wt/pending.jsonl' .gitignore)" "1"
```

Replace it with an assertion that `init` leaves `.gitignore` alone:

```bash
is "init does not touch .gitignore" "$(grep -c 'pending.jsonl' .gitignore)" "0"
```

Section 22 reads the queue directly at lines 431, 439 and 445. Those paths become
`.git/lane/pending.jsonl`.

Then add one new assertion — this is the defect, and it must fail before Step 1 and pass
after. Put it at the end of section 12, which already has a `setup` and a `lane note`:

```bash
"$LANE" note -p src/auth.rs -a "fn verify" "a note the parent has not promoted" > /dev/null
"$LANE" new inherit > /dev/null 2>&1
is "a fresh lane does not inherit the parent's queue" \
   "$("$LANE" ls | grep -c 'inherit.*0 pending')" "1"
"$LANE" rm inherit --force > /dev/null 2>&1
```

Confirm it fails against the pre-Step-1 binary before you rely on it.

**Verify**: `./scripts/test.sh` → `failed: 0`, 87 assertions.

### Step 4: Correct the documentation

`explainer.md` names `.wt/pending.jsonl` at lines 26, 75 and 113. `USAGE.md` needs two
edits: the Setup block's

```bash
git add .context .gitattributes AGENTS.md .gitignore
```

drops `.gitignore`, since `init` no longer writes it; and the Layout section gains the
queue where it now lives, one line, alongside the existing tree.

**Verify**: `grep -rn '\.wt/' *.md crates/ scripts/test.sh` → no matches outside `plans/`.

## Done criteria

- [ ] `cargo test` → 53 passing; `./scripts/test.sh` → `failed: 0`, 87 assertions
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` passes
- [ ] `grep -rn '\.wt' crates/ scripts/test.sh *.md` → no matches
- [ ] In a scratch repo: `lane init` leaves `.gitignore` unmodified; `lane note` then
      `lane new x` then `lane ls` shows `0 pending` for `x`
- [ ] `lane note`, `lane audit`, `lane why` still round-trip a note end to end

## STOP conditions

- `git rev-parse --git-path lane/pending.jsonl` returns the same path from a lane as from
  the main worktree. The whole fix rests on it not doing that; report instead of working
  around it.
- The new section-12 assertion passes *before* Step 1. That means the defect is not what
  this plan says it is — report what you observed.
- Removing the `.gitignore` write breaks a test that is not listed in Scope.
- You find a fourth reader or writer of `PENDING` beyond the three named in Current state.

## Maintenance notes

- `pending_path` and `hooks_dir` now resolve two different per-worktree paths the same
  way. If a third appears, they want one helper in `git.rs`, not three copies.
- The fallback branch in `pending_path` exists only for unit tests that run outside a
  repository. If those tests ever gain a real git repo, delete the fallback rather than
  keeping a code path production never takes.
- `.git/lane/` is now lane's private per-worktree directory. Anything else transient and
  worktree-scoped belongs there too, and nothing that a human should read does.
