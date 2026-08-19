# Plan 016: Fail `lane done` before it writes, not after

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 229d9ec..HEAD -- crates/lane/src/cli.rs crates/lane/src/worktree.rs`

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `229d9ec`, 2026-08-19

## Why this matters

`lane done` reads as all-or-nothing and is not. Traced against a main worktree holding an
uncommitted change to a file the lane also touched:

```
$ lane done
  rebased onto main
  memory: +0 new; checked 0: ...
  committed memory update              ← already written
  error: git merge --ff-only spike failed:
    Your local changes to the following files would be overwritten by merge:
      src/auth.rs
```

It fails at the last step having already rebased and committed. The user is left with
trunk not advanced, the lane not removed, and a memory commit they did not ask for yet.

Nothing is lost — git refused to clobber the working tree, and rerunning after a stash
completes cleanly because a second audit writes nothing new. But a command that half
finishes is a command people stop trusting, and the failure is entirely predictable
before any of the work starts.

## The design

Preflight the fast-forward before the audit. The check can be exact rather than
conservative, so no valid `done` is ever refused:

- `git diff --name-only <trunk>..<branch>` — the files a fast-forward would change
- `git -C <root> status --porcelain --untracked-files=no` — dirty tracked files in the
  main worktree
- intersect; refuse only if non-empty, naming the files

This only applies when the main worktree has trunk checked out. When trunk is checked out
nowhere, `fast_forward` takes the `update-ref` path, which touches no working tree and
cannot fail this way.

## Current state

`crates/lane/src/cli.rs`, `done()` runs in this order:

1. refuse if not inside a lane
2. refuse if the lane itself is dirty
3. `git rebase <trunk>`
4. `audit::run` — **writes notes, state and log**
5. `store::roll_up`
6. commit the memory update
7. `wt::fast_forward` — **the step that fails**
8. remove the lane

`crates/lane/src/worktree.rs`:

```rust
pub fn fast_forward(root: &Path, trunk: &str, branch: &str) -> Result<()> {
    let head = git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(root))?;
    ...
    if head == trunk {
        git(&["merge", "--ff-only", branch], Some(root))?;
        return Ok(());
    }
```

`is_dirty` already uses `--untracked-files=no`, so it is the right shape to borrow from.

Conventions: one-line comments, `anyhow::Result`, command functions return an exit code,
`#[cfg(test)] mod tests` at file end.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 2 |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 3 |

At `229d9ec` the baselines are 40 and 72.

## Scope

**In scope**: `crates/lane/src/worktree.rs` (the new check), `cli.rs` (`done`),
`test_lane.sh`, and `USAGE.md`'s "When things go wrong" section.

**Out of scope**:
- The rebase-then-audit ordering. The preflight goes *before* the rebase; the ordering of
  everything after it is load-bearing and stays.
- Making the audit itself transactional. It is idempotent, which is why rerunning works.
- `fast_forward`'s `update-ref` path.

## Steps

### Step 1: An exact preflight

In `worktree.rs`:

```rust
/// Files a fast-forward would overwrite in the main worktree. Empty means `done` can land.
pub fn blocking_changes(root: &Path, trunk: &str, branch: &str) -> Vec<String> {
    // Only the merge path touches a working tree; update-ref cannot conflict.
    if try_git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(root)) != trunk {
        return Vec::new();
    }
    let incoming: std::collections::HashSet<String> =
        try_git(&["diff", "--name-only", &format!("{trunk}..{branch}")], Some(root))
            .lines()
            .map(str::to_string)
            .collect();
    try_git(&["status", "--porcelain", "--untracked-files=no"], Some(root))
        .lines()
        .filter_map(|l| l.get(3..).map(str::to_string))
        .filter(|p| incoming.contains(p))
        .collect()
}
```

**Verify**: `cargo clippy --all-targets` clean; the end-to-end assertions in step 3 exercise
both the empty and non-empty results.

### Step 2: Call it before anything is written

In `done()`, immediately after the lane-dirty guard and **before** `git rebase`:

```rust
    let blocked = wt::blocking_changes(&root, &trunk, &branch);
    if !blocked.is_empty() {
        eprintln!(
            "error: {} has uncommitted changes to {}; commit or stash there first",
            trunk,
            blocked.join(", ")
        );
        return Ok(1);
    }
```

Exit 1, before the rebase, before the audit, before any commit.

**Verify**: the traced scenario now prints one `error:` line and leaves the lane untouched —
`git -C <lane> log` unchanged, no memory commit.

### Step 3: Cover it

Add to `test_lane.sh` before the summary:

```bash
echo "== N. done refuses before it writes =="
setup
"$LANE" new spike > /dev/null 2>&1
LP="$TMP/.lanes-repo/spike"
( cd "$LP" && printf 'pub fn verify() {\n    lane version\n}\n' > src/auth.rs \
  && git commit -qam "lane work" > /dev/null )
printf 'pub fn verify() {\n    my version\n}\n' > src/auth.rs   # dirty in main, same file
BEFORE=$(git -C "$LP" rev-parse HEAD)
( cd "$LP" && "$LANE" done > /tmp/blocked.out 2>&1 )
is "done refuses" "$(grep -c '^error:' /tmp/blocked.out)" "1"
is "and names the file" "$(grep -c 'src/auth.rs' /tmp/blocked.out)" "1"
is "nothing was committed" "$(git -C "$LP" rev-parse HEAD)" "$BEFORE"
git checkout -- src/auth.rs
( cd "$LP" && "$LANE" done > /dev/null 2>&1 )
```

The third assertion is the point: the old behaviour made a memory commit before failing.
Confirm it fails against the current code before implementing.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 3.

### Step 4: Document the new refusal

`USAGE.md`'s "When things go wrong" gains an entry for it, saying which worktree to clean
and that nothing in the lane was touched.

**Verify**: `grep -c 'commit or stash there first' USAGE.md` → `1`.

## Done criteria

- [ ] `cargo test` passes, baseline + 2; `./test_lane.sh` passes, baseline + 3
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] The traced scenario exits 1 with no rebase, no audit and no memory commit
- [ ] `lane done` still succeeds normally when the main worktree is clean, and when it is
      dirty on files the lane did **not** touch
- [ ] `plans/README.md` row updated

## STOP conditions

- The preflight refuses a `done` that would have succeeded. The intersection is meant to be
  exact — report the file lists rather than loosening it to "any dirt at all".
- `git status --porcelain` line parsing misreads a rename (`R  old -> new`) or a path with
  spaces. Use `-z` if so and say why.
- You find another step in `done` that can fail after a write. Report it; this plan covers
  the fast-forward, and a second such step deserves its own decision.

## Maintenance notes

- The rule: **everything that can refuse `done` must run before the rebase.** New work
  added to `done` goes after the preflight, never between it and the audit.
- The audit is idempotent, which is what makes a failed `done` recoverable by rerunning.
  Anything that breaks that idempotence makes this class of failure much worse.
