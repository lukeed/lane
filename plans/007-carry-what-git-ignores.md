# Plan 007: Carry what git ignores, and nothing at all without reflink

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 229d9ec..HEAD -- crates/lane/src/worktree.rs crates/lane/src/cli.rs`

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `229d9ec`, 2026-08-19
- **Supersedes**: the earlier version of this plan, which made the hardcoded list
  configurable. That design could not express the case that matters and is abandoned.

## Why this matters

Two measurements against a realistic pnpm-style monorepo:

```
node_modules/root-pkg/big.bin      carried
packages/a/node_modules/big.bin    MISSING
.env                               MISSING
```

`.env` missing means the lane does not run the project. The nested `node_modules` is
worse: `skip` keys on the **top-level** path component, so for the most common JS layout
the headline feature — "your warm build cache arrives by reference" — silently does
nothing. A list of top-level names cannot express `packages/*/node_modules`.

And `cow::probe` currently gates nothing. It computes `supported`, prints one line, and
the clone runs regardless with a per-file `fs::copy` fallback. So on ext4 `lane new`
byte-copies your entire `node_modules` and `target` — the exact expensive thing the tool
exists to avoid, done quietly.

## The design

**1. The warm set comes from git.**

```
$ git status --porcelain -z --ignored
!! .env
!! node_modules/
!! packages/a/node_modules/
!! packages/b/node_modules/
!! target/
```

Verified. Six entries, already collapsed to directory roots, at any depth, derived from
the user's own ignore rules — including nested `.gitignore` files, the global one and
`.git/info/exclude`, all of which a hand-written pattern list would get wrong.

The definition is exactly right: *what git will not materialize for you* is precisely what
a fresh worktree is missing. `WARM_DEFAULT`'s ten names were an approximation of it.

Use `-z` and split on NUL — without it, paths containing spaces come back shell-quoted.
A trailing `/` marks a directory.

**2. No reflink, no clone.**

The tool is about copy-on-write. Simulating it with an expensive byte copy is the wrong
thing done loudly. When `probe()` says no, skip the clone entirely and leave a plain
worktree — which is what `git worktree add` would have given, plus the memory half, which
needs no reflink at all.

Keep the **per-file** fallback in `clone_tree`. `clone_file` can fail on one file even on
a capable filesystem (a path on a different mount), and that must not kill the lane.

**3. `lane.warm` inverts into `lane.exclude`.**

With everything ignored carried by default, configuration is for opting *out* — a giant
`target/` you do not want a second view of. Multi-valued git config, matched against the
entry paths git reports.

**4. `--fork` becomes `--dirty`.**

Once ignored files are carried by default, the flag's only remaining effect is dirty state:
uncommitted changes to tracked files, and untracked-but-not-ignored files. `--fork`
described the implementation; `--dirty` describes what it does.

Default stays clean. Opening a lane is usually "try something in isolation", and
`lane new agent-a && lane new agent-b` should not hand three agents three divergent copies
of half-finished work. But the flag must be discoverable at the moment it is wanted:

```
$ lane new spike
  reflink: yes (reflink available)
  warning: 3 uncommitted change(s) were not carried
    lane rm spike && lane new spike --dirty   to start over with them
  1284 files cloned (612.4 MiB shared, 0 copied)
```

Verified that this recovery works and exits 0: the branch has no commits trunk lacks, so
`lane rm` accepts it without `--force`, and the main worktree is untouched throughout.

## Current state

`crates/lane/src/worktree.rs`:

```rust
pub const WARM_DEFAULT: [&str; 10] = [ ... ];
```

```rust
        let skip = |rel: &str, is_dir: bool| {
            let top = rel.split(std::path::MAIN_SEPARATOR).next().unwrap_or(rel);
            if top == ".git" {
                return true;
            }
            if is_dir {
                // Descend only into warm entries, and only at the top level.
                return !rel.contains(std::path::MAIN_SEPARATOR) && !warm_set.contains(top);
            }
            tracked.contains(rel) || !warm_set.contains(top)
        };
```

`create()` takes `warm: Option<Vec<String>>`, which nothing passes. `tracked_set()` exists
only to serve this closure and becomes dead once git supplies the list.

`crates/lane/src/cli.rs` — the `New` variant's `fork` flag, and `new()`.
`crates/lane/src/git.rs` — `try_git` is the pattern for a call allowed to fail.

Conventions: one-line comments, `anyhow::Result`, `#[cfg(test)] mod tests` at file end.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 3 |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 5 |

At `229d9ec` the baselines are 40 and 72.

## Scope

**In scope**: `crates/lane/src/worktree.rs`, `cli.rs`, `test_lane.sh`, `README.md`,
`USAGE.md`.

**Out of scope**:
- `cow.rs`'s clone primitives and its per-file fallback.
- `lane done`'s atomicity — plan 016.
- Deleting `--dirty`. It is renamed, not removed.
- Carrying untracked-but-not-ignored files by default. Those stay behind `--dirty`; that
  is what keeps the two modes distinct.

## Steps

### Step 1: Ask git for the warm set

In `worktree.rs`, replace `WARM_DEFAULT` and `tracked_set` with:

```rust
/// Entries git will not materialize: exactly what a fresh worktree is missing.
/// Already collapsed to directory roots, at any depth, from the user's own ignore rules.
fn ignored_entries(root: &Path) -> Vec<String> {
    try_git(&["status", "--porcelain", "-z", "--ignored"], Some(root))
        .split('\0')
        .filter_map(|e| e.strip_prefix("!! "))
        .map(|p| p.trim_end_matches('/').to_string())
        .filter(|p| !p.is_empty() && p != ".git")
        .collect()
}
```

Then `excluded(root)` reading `git config --get-all lane.exclude`, and the default branch
of `create()` cloning each remaining entry with `cow::clone_tree` rooted at that entry
rather than walking the whole tree with a `skip` closure.

**Verify**: a unit test is impractical (needs a repo); step 5 covers it end to end.
Confirm by hand that `ignored_entries` on a monorepo fixture returns the nested paths.

### Step 2: Gate the clone on the probe

In `create()`, when `probe()` reports no support, skip the clone entirely and push a note
saying so:

```rust
        notes.push("no reflink here; leaving a plain worktree".into());
```

`lane new` then costs exactly what `git worktree add` costs.

**Verify**: hard to force locally on APFS. Assert the *logic* with a unit test on whatever
function decides, and rely on step 5's probe-conditional assertions for the rest.

### Step 3: Rename `--fork` to `--dirty`, and warn

Rename the flag on the `New` variant and the `fork` parameter through `create()`. Keep the
`--no-checkout` + `reset --mixed` + `update-index --refresh` implementation exactly as it
is; only the name changes.

When `--dirty` was **not** passed and the parent tree has uncommitted tracked changes, push
a warning note naming the recovery, as shown above. Count with
`git status --porcelain --untracked-files=no`, matching `is_dirty`.

**Verify**: `lane new x` in a dirty repo prints the warning and the two-command recovery;
`lane new x --dirty` does not.

### Step 4: Correct the docs

`README.md`'s materialization table and the paragraph under it, and `USAGE.md`'s "Open a
lane" section. They must say:

- default: git checks out tracked files, everything git ignores is cloned by reference at
  any depth, uncommitted work is not carried
- `--dirty`: the same, plus your uncommitted work
- without reflink: a plain worktree, and why that is the honest outcome
- `lane.exclude` for opting out

Delete the "untracked + ignored" column claim, which was never true.

**Verify**: `grep -c 'untracked + ignored' README.md` → `0`; `grep -c 'fork' README.md USAGE.md` → `0`.

### Step 5: Cover it

Add to `test_lane.sh` before the summary. The suite must work on a filesystem without
reflink, so assert the *relationship* rather than the outcome — that keeps the count stable
and tests both paths:

```bash
echo "== N. a lane carries what git ignores =="
setup
mkdir -p packages/a/node_modules packages/b/node_modules
echo cache > packages/a/node_modules/dep
echo cache > packages/b/node_modules/dep
echo "export const a = 1" > packages/a/index.ts
echo "SECRET=1" > .env
printf 'node_modules/\n.env\n' > .gitignore
git add -A && git commit -qm monorepo

"$LANE" new carry > /tmp/carry.out 2>&1
LP="$TMP/.lanes-repo/carry"
REFLINK=$(grep -c 'reflink: yes' /tmp/carry.out)
want() { [ "$REFLINK" = "1" ] && echo yes || echo no; }
is "a nested node_modules is carried iff reflink" \
   "$([ -f "$LP/packages/a/node_modules/dep" ] && echo yes || echo no)" "$(want)"
is "an ignored file is carried iff reflink" \
   "$([ -f "$LP/.env" ] && echo yes || echo no)" "$(want)"
is "tracked files always come from git" \
   "$([ -f "$LP/packages/a/index.ts" ] && echo yes || echo no)" "yes"
"$LANE" rm carry --force > /dev/null 2>&1

echo "// scratch" >> src/auth.rs
"$LANE" new clean > /tmp/clean.out 2>&1
is "a dirty tree without --dirty warns and names the recovery" \
   "$(grep -c 'lane rm clean && lane new clean --dirty' /tmp/clean.out)" "1"
"$LANE" rm clean --force > /dev/null 2>&1
"$LANE" new carried --dirty > /dev/null 2>&1
is "--dirty carries the change" \
   "$(grep -c 'scratch' "$TMP/.lanes-repo/carried/src/auth.rs")" "1"
```

Section 2's existing "warm dir present in lane" assertion must get the same treatment.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 5.

## Done criteria

- [ ] `cargo test` passes, baseline + 3; `./test_lane.sh` passes, baseline + 5
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `grep -c 'WARM_DEFAULT\|tracked_set' crates/lane/src/worktree.rs` → `0`
- [ ] `grep -rc 'fork' crates/lane/src/ README.md USAGE.md` → `0` everywhere
- [ ] A lane opened in a monorepo fixture contains `packages/a/node_modules/` and `.env`
- [ ] `plans/README.md` row updated

## STOP conditions

- `git status --porcelain -z --ignored` returns a file enumeration rather than collapsed
  directory roots on the installed git. It was verified here; report the version if not.
- Cloning entry-by-entry turns out slower than the single tree walk it replaces on a large
  repo. Measure before assuming; report numbers rather than reverting.
- Skipping the clone without reflink breaks an existing assertion in a way step 5's
  probe-conditional pattern does not fix.
- You find yourself needing to carry untracked-but-not-ignored files to make a test pass.
  That is `--dirty`'s job and the line between the two modes.

## Maintenance notes

- The rule: **the warm set is git's answer, not ours.** Any future hardcoded path list is
  a regression. If something needs excluding, that is `lane.exclude`.
- `probe()` now gates behaviour, not just a message. A future caller that clones without
  consulting it reintroduces the byte-copy-on-ext4 problem.
- `--dirty` is the only remaining difference between the two modes. If it ever stops being
  the only one, the name is wrong again.
- Deferred: `lane init` could seed `lane.exclude` for known-huge directories. Nothing
  suggests it is needed yet.
