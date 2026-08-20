# Plan 019: Make a lane a place where nothing is broken

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat b186fad..HEAD -- crates/lane/src/worktree.rs crates/lane/src/cow.rs scripts/check-linux.sh`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `b186fad`, 2026-08-19

## Why this matters

Lane's reason to exist is that you work in a lane. So anything that breaks *because* you
are in a lane is lane's defect, not something for the user to route around. Three were
found in a single afternoon of using lane on its own repository, and only one of them was
in lane's own tooling — which suggests the class is under-explored, not that we were
unlucky.

Each item below was reproduced end to end before this plan was written, and one candidate
was tested and cleared.

**1. An absolute symlink inside a cloned directory still points at the parent tree.**
Lane clones everything git ignores, and preserves symlinks byte for byte — deliberately:
`cow.rs:56` passes `CLONE_NOFOLLOW` so `clonefile` copies the link, not its target, and the
fallback path at `cow.rs:189-191` calls `symlink(read_link(src))`. For a relative link that
is exactly right. For an absolute link that points back inside the repository, the lane
gets a link into the *parent* worktree:

```
in the lane:
  abs-link -> /tmp/symtest2.CFgq/repo/real/tool      # the parent's path
    runs: PARENT copy
  rel-link -> ../../real/tool
    runs: LANE copy
```

The lane silently executes the parent's file. Every guarantee lane makes about isolation —
two agents in two lanes not seeing each other's work — is void along that path.

**2. Every lane breaks if the tree is ever moved.** `lane new` takes git's default,
which writes an absolute path into the worktree's `.git` file:

```
lane .git contains: gitdir: /private/tmp/abswt.clRN/repo/.git/worktrees/spike
after moving repo and lanes together:
  fatal: not a git repository: /private/tmp/abswt.clRN/repo/.git/worktrees/spike
```

Git has solved this: `git worktree add --relative-paths` makes both sides of the pointer
relative. Verified against lane's own sibling layout — `gitdir: ../../repo/.git/worktrees/spike`
on one side, `../../../../.lanes-repo/spike/.git` on the other — and the whole tree then
survives a move intact.

**3. `./scripts/check-linux.sh` cannot run from a lane.** It does `cp -r /w /build`, which
copies the `.git` *file* and its dangling pointer into the container, so every git call
inside fails. A contributor who follows lane's own instructions and works in a lane cannot
run lane's Linux gate.

**Tested and cleared, so nobody re-audits it**: a Python `.venv` cloned into a lane works
correctly. `pyvenv.cfg` records the original path in its `command =` line, but that field
is informational; Python derives `sys.prefix` from the interpreter's own location, which
resolves to the lane. No action needed.

## The boundary this plan does not cross

A worktree is a *reference* into another directory. A lane copied somewhere that cannot
see its parent repository cannot work, and no amount of effort inside lane changes that —
what you want there is a clone, which gives up the shared object store that makes a lane
cheap. So the goal is precise: **a lane is whole wherever its repository is reachable.**
Item 3 fixes lane's own script by making the parent reachable inside the container. It
does not make a lane a standalone artifact, and a plan that tries to is the wrong plan.

## Current state

`crates/lane/src/worktree.rs` calls `git worktree add` twice, and both need the same
treatment:

- line 218-230, the `Materialization::Dirty` arm:
  ```rust
              git(
                  &[
                      "worktree",
                      "add",
                      "--no-checkout",
                      "-b",
                      name,
                      &dest_str,
                      &base,
                  ],
                  Some(&root),
              )?;
  ```
- line 261-264, every other arm:
  ```rust
              git(
                  &["worktree", "add", "-b", name, &dest_str, &base],
                  Some(&root),
              )?;
  ```

`crates/lane/src/cow.rs:189-192`, the fallback tree walk:

```rust
        if entry.file_type().is_symlink() {
            let dest = fs::read_link(entry.path())?;
            std::os::unix::fs::symlink(dest, &target)?;
            stats.links += 1;
            continue;
        }
```

The reflink path does not go through this code — `clone_file` with `CLONE_NOFOLLOW`
reproduces the link inside the kernel — so the fix has to apply to both paths, and the
test has to cover both.

`crates/lane/src/git.rs` gives you `git` (fails loudly), `try_git` (failure is empty
string) and `git_ok` (exit status only), each taking `cwd: Option<&Path>`.

Conventions: one-line comments, and only where the reason is not obvious from the code;
`anyhow::Result`; tests in `#[cfg(test)] mod tests` at file end. Integration tests for the
clone layer live in `tests/cow.rs` — follow
`fallback_tree_is_byte_identical_and_symlinks_survive`, which is the test whose promise
this plan narrows. Commit subjects are Conventional Commits, `type: verb object`, one
short clause, no scope.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 3 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 3 |
| Linux gates | `./scripts/check-linux.sh` | all pass — **from the main checkout** |

Record both baselines before you start; at `b186fad` they are 53 and 87.

## Scope

**In scope**: `crates/lane/src/worktree.rs`, `crates/lane/src/cow.rs`, `tests/cow.rs`,
`test_lane.sh`, `scripts/check-linux.sh`, `USAGE.md`.

**Out of scope**:
- The memory store. This plan does not touch `.context/` or anything under `store.rs`.
- `--dirty` semantics, the exclude list, or which entries get cloned.
- Making a lane usable with no access to its repository. See "The boundary" above.
- Hardlinks, sockets, or device files inside ignored directories.

## Steps

### Step 1: Make new lanes relocatable

Add `--relative-paths` to both `git worktree add` invocations.

The flag landed in git 2.48 and lane has no documented git floor, so probe rather than
assume. `git worktree add --relative-paths` on an older git fails with an
unknown-option error, which would turn a supported operation into a hard failure.

Add a capability check next to the call sites:

```rust
/// git 2.48+; older versions get absolute paths, which still work in place.
fn relative_paths_supported(root: &Path) -> bool {
    try_git(&["worktree", "add", "-h"], Some(root)).contains("--relative-paths")
}
```

Build the argument list with the flag included only when supported. Do not add the flag to
`git worktree repair`, `list`, or `remove`.

**Verify**, in a scratch repo:
- `cat <lane>/.git` → `gitdir: ../../<repo>/.git/worktrees/<name>` — relative, not absolute
- `cat <repo>/.git/worktrees/<name>/gitdir` → also relative
- move the repository and its `.lanes-*` sibling together, then `git rev-parse
  --abbrev-ref HEAD` inside the lane → the branch name, not `fatal: not a git repository`
- `lane ls`, `lane why`, `lane done` all still work in a lane created this way

### Step 2: Re-point absolute in-repo symlinks at the lane

When cloning, a symlink whose target is an absolute path **inside the source root** must be
rewritten to the same relative position under the destination root. A symlink pointing
anywhere else — `/nix/store`, `/opt/homebrew`, a relative link of any kind — is copied
unchanged. Getting that distinction wrong is worse than the bug.

```rust
/// An absolute link into the source tree would still resolve to the parent worktree, so
/// the lane would read and execute the parent's file. Links outside it are left alone.
fn retarget(link: &Path, src_root: &Path, dst_root: &Path) -> Option<PathBuf> {
    let rel = link.strip_prefix(src_root).ok()?;
    Some(dst_root.join(rel))
}
```

Apply it in the fallback walk at `cow.rs:189`, and — this is the part that is easy to
miss — on the reflink path too. `clonefile` with `CLONE_NOFOLLOW` reproduces the link
inside the kernel, so it never passes through Rust. After cloning an entry, walk it for
symlinks needing a retarget and rewrite those in place, or read the link before cloning and
write the corrected one afterwards. Either is fine; both paths must end up equivalent.

Canonicalize `src_root` before comparing, or a `/tmp` vs `/private/tmp` mismatch on macOS
will make every comparison miss.

**Verify**: the scenario from "Why this matters", both with reflink and with it forced off
— in the lane, `abs-link` resolves to the lane's own copy, `rel-link` still does, and a
link to `/opt/homebrew/bin/anything` is byte-identical to the original.

### Step 3: Cover both paths

Add to `tests/cow.rs`, following `fallback_tree_is_byte_identical_and_symlinks_survive`:

1. an absolute symlink into the source root is retargeted into the destination root
2. an absolute symlink *outside* the source root is preserved exactly
3. a relative symlink is preserved exactly

The existing test asserts "symlinks survive", which is what this plan narrows — update its
comment so the two tests read as one intent rather than a contradiction. Do not weaken its
assertions.

Add to `test_lane.sh`, as a new section before the summary:

```bash
echo "== N. a lane is isolated from the parent tree =="
setup
mkdir -p node_modules/.bin
printf '#!/bin/sh\necho PARENT\n' > tool.sh && chmod +x tool.sh
git add -A && git commit -qm "add tool"
ln -s "$PWD/tool.sh" node_modules/.bin/abs
"$LANE" new iso > /dev/null 2>&1
( cd "$TMP/.lanes-repo/iso" && printf '#!/bin/sh\necho LANE\n' > tool.sh )
is "an absolute link resolves inside the lane" \
   "$(cd "$TMP/.lanes-repo/iso" && ./node_modules/.bin/abs)" "LANE"
is "the lane's git dir is relative" \
   "$(grep -c '^gitdir: \.\.' "$TMP/.lanes-repo/iso/.git")" "1"
"$LANE" rm iso --force > /dev/null 2>&1
```

Confirm both assertions fail against the pre-Step-1 binary.

**Verify**: `cargo test` → baseline + 3; `./test_lane.sh` → `failed: 0`, baseline + 2.

### Step 4: Let the Linux gate run from a lane

`scripts/check-linux.sh` mounts the repository at `/w` and copies it to `/build`. In a
lane, `.git` is a file pointing at an absolute host path that is not mounted.

Resolve the common directory and mount it at the same absolute path it has on the host, so
the pointer inside the container resolves:

```sh
COMMON="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)"
```

Pass a second read-only `-v "$COMMON":"$COMMON":ro` when `$COMMON` is not already inside
`$ROOT`. When it is — the main checkout — the existing single mount is already correct and
the script must behave exactly as it does today.

**Verify**: `./scripts/check-linux.sh` passes from the main checkout, and passes from a
lane created by `lane new`. Both, not one.

### Step 5: Say what a lane guarantees

`USAGE.md` claims lanes are isolated and that agents in different lanes cannot collide.
That is now true for absolute symlinks and was not before. Add two or three sentences to
"Working with agents" stating what a lane shares with its parent — the object store, the
hooks directory, git config — and what it does not. No table; this is a short paragraph.

**Verify**: `grep -c 'shares' USAGE.md` → at least `1`.

## Done criteria

- [ ] `cargo test` passes, baseline + 3; `./test_lane.sh` passes, baseline + 2
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` passes **from the main checkout and from a lane**
- [ ] A new lane's `.git` holds a relative `gitdir:`; repo + lanes survive being moved
- [ ] In a lane, an absolute symlink into the repo resolves to the lane's own copy
- [ ] A symlink to a path outside the repo is byte-identical to the original
- [ ] `lane new`, `ls`, `why`, `note`, `audit`, `done` all work in a relative-path lane

## STOP conditions

- `--relative-paths` is unsupported by the git on this machine. The probe should make this
  impossible; if it happens anyway, report rather than writing the pointer files yourself.
- Retargeting would rewrite a symlink whose target is outside the source root. That is the
  one thing this plan must never do — stop and report.
- The reflink and fallback paths cannot be made to agree on symlink handling without
  restructuring `clone_tree`. Report the design problem; do not restructure it here.
- `./scripts/check-linux.sh` starts behaving differently from the main checkout than it
  does today. The lane case is additive; the existing case must not change.

## Maintenance notes

- Steps 1 and 2 are the same bug wearing two hats: a path that points at where a lane came
  from rather than where it is. Anything new that copies, records or caches an absolute
  path is a candidate for the same defect, and the test in Step 3 is the place to add it.
- Plan 018 fixed the mirror image — a lane *inheriting* state through a worktree-relative
  path. Between them they say lane's invariant is "a lane's state is its own". A future
  reviewer should treat any deviation from that as a bug, not a quirk.
- The retarget rule is deliberately narrow: absolute, and inside the source root. Widening
  it to resolve `..` chains or to follow relative links to their absolute targets would
  start rewriting links that work correctly today.
