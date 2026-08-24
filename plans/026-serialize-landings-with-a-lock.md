# Plan 026: Serialize landings with a lock, and mark them in trunk's history

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat 6991fe0..HEAD -- crates/lane/src/cli.rs crates/lane/src/store.rs crates/lane/Cargo.toml`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `6991fe0`, 2026-08-20

## Why this matters

`.context/state/<branch>.json` has more than one writer and nothing serializes them.

`lane done` runs `store::roll_up` from inside the lane (`cli.rs:876`), which reads trunk's
state file, merges the lane's entries in, and writes trunk's file. Meanwhile `lane audit`
run on trunk writes the same file — and leaves it uncommitted. Verified:

```
$ lane audit --review none      # on trunk
$ git status --porcelain
 ?? .context/-/
 ?? .context/state/
 ?? .context/attic/
 ?? .context/log/
```

Two failures follow, both hit for real while working on this repository:

- trunk's tree is dirty, so the fast-forward refuses **after** the memory commit already
  landed in the lane, leaving trunk unadvanced
- trunk's file changed since the lane branched, so the rebase conflicts on a derived JSON
  cache — which is genuinely dangerous to resolve by hand. One attempt produced an entry
  whose `status` said `body-drift` while its hashes said `fresh`

Two lanes landing at once hit the same read-modify-write race on trunk's state.

The fix is mutual exclusion, not ownership. A lock held for the duration of a landing makes
the existing order correct as it stands — nothing needs to move.

## The design

**Sequence** (the non-squash path is today's order, unchanged):

```
precheck → lock → rebase → audit → roll_up → commit "lane: sync <lane> memory"
         → --squash ? merge --squash + commit "lane: merged <lane>" + branch -D
                    : fast-forward
         → unlock
```

Trunk advances exactly once, by a single ref update, to a commit that already contains the
lane's code and its memory. There is no window in which trunk holds code without memory.

**The lock is `flock(2)` on an open descriptor, not a lock file.** This matters and is the
one thing not to substitute. A lock file's *existence* survives `kill -9`, a panic, or a
power cut — that is a stale lock needing PID checks and an expiry policy. `flock` is
released by the kernel when the descriptor closes, however the process ends. Verified on
this machine with rustix 1.1.4:

```
1. first holder                ACQUIRED
2. concurrent process          BUSY (Resource temporarily unavailable, os error 35)
3. kill -9 the holder          lock file still on disk: yes
4. next process                ACQUIRED
```

Step 4 is the whole point: there is no stale state to detect or break.

**The lock file must live in the common git directory.** This is the trap. The
`--git-path` idiom lane already uses is *per-worktree* — plan 018 relies on exactly that
property — so a lock placed there is invisible to other lanes and serializes nothing, while
testing green with a single lane. Verified:

```
from a LANE:  git rev-parse --git-path lane/x  -> .git/worktrees/<name>/lane/x
from MAIN:    git rev-parse --git-path lane/x  -> .git/lane/x
both:         git rev-parse --path-format=absolute --git-common-dir  -> <repo>/.git
```

So resolve the path with `--git-common-dir` and place the lock at
`<common-dir>/lane/<trunk>.lock`.

**Non-blocking.** If the lock is held, `lane done` exits non-zero with a message naming the
situation — another lane is landing, try again. A landing takes seconds; blocking silently
behind a wedged holder is worse than a clear refusal. Do not add a `--wait` flag.

**`lane audit` on trunk takes the same lock**, because it writes the same files.
`lane audit` inside a lane does not — it writes only that lane's files.

**Ownership is not claimed.** A lane still writes trunk's state file, and that is fine under
a lock. Do not move `roll_up`, and do not rewrite the README to claim single-writer
ownership; see Step 6 for what it should say instead.

## Current state

`crates/lane/src/cli.rs`, `done()` — the sequence to preserve, currently at lines 866-897:

```rust
    let out = audit::run(&lane_path, &options(&trunk, budget, review), reviewer.as_ref())?;
    audit::report(&out, info)?;

    // Fold this lane's per-branch files into the trunk's, so nothing accumulates.
    store::roll_up(&lane_path, &branch, &trunk)?;
    ...
            &format!("memory: update context from lane {branch}"),
    ...
    wt::fast_forward(&root, &trunk, &branch)?;
```

`done()` already has a preflight from plan 016 that refuses before writing; read it and
extend it rather than adding a second one.

`crates/lane/src/git.rs` gives `git`, `try_git`, `git_ok`, each taking `cwd: Option<&Path>`.

`crates/lane/Cargo.toml` scopes rustix to Linux today:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
libc = "0.2.189"

[target.'cfg(target_os = "linux")'.dependencies]
rustix = { version = "1.1.4", features = ["fs"] }
```

Move `rustix` to the shared `[dependencies]` table. It is already in `Cargo.lock` and it
already builds on macOS — confirmed by compiling `rustix::fs::flock` there. **Leave `libc`
exactly where it is**; `clone_file` uses `libc::clonefile`, rustix offers only the fd-based
`fclonefileat`, and rewriting the clone layer is not part of this plan.

Conventions: one-line comments, and only where the reason is not obvious; `anyhow::Result`;
tests in `#[cfg(test)] mod tests` at file end. Commit subjects are Conventional Commits,
`type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 1 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./scripts/test.sh` | `failed: 0`, baseline + 5 |
| Linux gates | `./scripts/check-linux.sh` | exit 0, from a lane and the main checkout |

Record both baselines before starting; at `6991fe0` they are 75 and 117.

Capture every exit code without a pipe (`cmd > /tmp/out 2>&1; echo $?`). Piping to `tail`
reports `tail`'s status and has already masked a failure in this project.

## Scope

**In scope**: `crates/lane/src/cli.rs`, `crates/lane/src/store.rs`,
`crates/lane/Cargo.toml`, `scripts/test.sh`, `README.md`, `USAGE.md`.

**Out of scope**:
- `crates/lane/src/cow.rs` and the clone layer. Do not touch `libc::clonefile`.
- Moving `roll_up`, or changing who writes which file. The lock makes the current
  arrangement correct.
- `crates/lane/src/audit.rs`'s internals. Only the trunk-side *caller* gains a lock.
- A `--wait` flag, lock timeouts, or stale-lock recovery. `flock` makes them unnecessary.
- `crates/lane/assets/skill.md`, `AGENTS.md`.

## Steps

### Step 1: Widen rustix, and add the lock primitive

Move `rustix` from the Linux-only target table to `[dependencies]`, keeping
`features = ["fs"]`. Leave `libc` under its macOS target table.

Add a lock guard — a type holding the open `File`, acquiring
`FlockOperation::NonBlockingLockExclusive` on construction and releasing on drop. The
descriptor must stay alive for the whole landing; if the `File` is dropped early the lock
goes with it.

```rust
use rustix::fs::{FlockOperation, flock};
```

Resolve the path with
`git rev-parse --path-format=absolute --git-common-dir`, then join `lane/<trunk>.lock`, and
`create_dir_all` the parent.

**Verify**: `cargo build` on this machine; `cargo test` still passes at baseline.

### Step 2: Take it in `lane done`, before the rebase

Acquire the lock immediately after the existing preflight and before `git rebase`. The
rebase targets trunk, so another landing moving trunk mid-flight is exactly what produces
`trunk has diverged`.

On `EAGAIN`, exit non-zero with a message naming what is happening and what to do. Match
the error style already used in `done()`.

**Verify**: hold the lock from a second process and confirm `lane done` refuses immediately
with a non-zero exit and a clear message — not a hang, not a git error.

### Step 3: Take it in `lane audit` when run on trunk

`audit_cmd()` uses `git::repo_root()`, which is the current worktree. Take the lock only
when that root is the main worktree — compare against `wt::main_root()`. Inside a lane,
audit writes only that lane's files and must not block on the lock.

**Verify**: with the lock held, `lane audit` on trunk refuses; `lane audit` inside a lane
succeeds.

### Step 4: Name the commits, and add `--squash`

Rename the memory commit to `lane: sync <lane> memory` — always, both paths.

Add `--squash` to the `Done` command. When set, replace the fast-forward with a squash
merge onto trunk committed as `lane: merged <lane>`, folding the lane's own commits and its
sync-memory commit into that one commit. Afterwards git will not report the branch as
merged, so cleanup needs `git branch -D` rather than `-d`; find where `done` currently
deletes the branch and handle both cases.

Without `--squash`, behaviour is today's: rebase and fast-forward, so trunk's history ends
with `lane: sync <lane> memory`, which is the landing marker.

**Verify**:
- default: `git log --oneline` on trunk ends with `lane: sync <lane> memory`, history linear
- `--squash`: exactly one new commit on trunk, subject `lane: merged <lane>`, containing
  both the lane's file changes and its `.context/` updates
- `--squash`: the lane's branch is gone afterwards

### Step 5: Cover it

Add unit tests: two `File` handles on the same path, both flocked — the second returns an
error. That is the property the whole plan rests on, and `flock` is per open-file-
description so this works within one process.

Add to `scripts/test.sh` before the summary, numbered one past the last. Five assertions:

```bash
echo "== N. landings are serialized and marked =="
setup
"$LANE" new solo > /dev/null 2>&1
( cd "$TMP/repo/.lanes/solo" && echo "work" > src/new.rs && git add -A && git commit -qm "add work" )
( cd "$TMP/repo/.lanes/solo" && "$LANE" done --review none > /dev/null 2>&1 )
is "trunk ends with the sync marker" \
   "$(git log -1 --format=%s | grep -c '^lane: sync solo memory$')" "1"
is "history stayed linear" "$(git log -1 --format=%P | wc -w | tr -d ' ')" "1"

"$LANE" new sq > /dev/null 2>&1
( cd "$TMP/repo/.lanes/sq" && echo "a" > src/a.rs && git add -A && git commit -qm "one" \
  && echo "b" > src/b.rs && git add -A && git commit -qm "two" )
BEFORE=$(git rev-list --count HEAD)
( cd "$TMP/repo/.lanes/sq" && "$LANE" done --squash --review none > /dev/null 2>&1 )
is "squash lands exactly one commit" \
   "$(( $(git rev-list --count HEAD) - BEFORE ))" "1"
is "and names it merged" \
   "$(git log -1 --format=%s | grep -c '^lane: merged sq$')" "1"
is "and removed the branch" "$(git branch --list sq | wc -l | tr -d ' ')" "0"
```

Confirm each fails against the pre-change binary.

**Verify**: `./scripts/test.sh` → `failed: 0`, baseline + 5.

### Step 6: Say what is actually true

`README.md` claims the store needs no coordination because everything that changes is
per-writer: "one branch, one file". That is true of notes and false of state — a landing
lane writes trunk's state file.

Do not restate it as ownership. Write what holds: note files are immutable and can never
conflict; state files are written by more than one branch and their writes are serialized
by a lock held for the duration of a landing, so a landing is exclusive. `USAGE.md`'s
"When things go wrong" gains an entry for the refusal message and what to do about it.

**Verify**: `grep -c 'serial' README.md` → at least `1`.

## Done criteria

- [ ] `cargo test` passes, baseline + 1; `./scripts/test.sh` passes, baseline + 5
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` exit 0 from a lane **and** from the main checkout
- [ ] With the lock held elsewhere, `lane done` exits non-zero immediately with a clear
      message, and does not hang
- [ ] With the lock held elsewhere, `lane audit` on trunk refuses; inside a lane it succeeds
- [ ] Killing a lock holder with `kill -9` leaves the next `lane done` able to proceed
- [ ] Default landing leaves trunk linear, ending at `lane: sync <lane> memory`
- [ ] `--squash` adds exactly one commit, `lane: merged <lane>`, and deletes the branch
- [ ] `git diff --stat -- crates/lane/src/cow.rs crates/lane/assets/skill.md AGENTS.md` → empty

## STOP conditions

- The lock resolves to a per-worktree path. Confirm with
  `git rev-parse --path-format=absolute --git-common-dir` from inside a lane and from the
  main checkout — the two must be identical. If they differ, stop; a per-worktree lock
  serializes nothing and will pass a single-lane test.
- `lane done` blocks rather than failing when the lock is held.
- A `kill -9`'d holder leaves the lock unavailable. That means a lock file was used instead
  of `flock` on a descriptor; stop and report rather than adding expiry logic.
- Making `--squash` work requires changing what the default path does.
- You conclude `libc` should be replaced by rustix in `cow.rs`. It should not, here.

## Maintenance notes

- The invariant: **a landing is exclusive.** Any future command that writes trunk's
  `.context/` must take the same lock. The two today are `lane done` and `lane audit` on
  trunk.
- `flock` is held by the descriptor, so the guard's `File` must outlive the whole landing.
  A refactor that drops it early — returning the path instead of the handle, say — silently
  removes all protection while every test still passes, because tests rarely run two
  processes.
- `libc` stays for `clonefile`. rustix offers only `fclonefileat`, which needs an open
  source fd and a destination directory fd; swapping it rewrites the clone layer for no
  gain, since `libc` is already compiled transitively via `sha2`, `tempfile` and `ring`.
