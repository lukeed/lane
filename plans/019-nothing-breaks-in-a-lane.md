# Plan 019: Put lanes inside the repository, and make their paths survive a move

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat cb1ef6c..HEAD -- crates/lane/src/worktree.rs crates/lane/src/cow.rs scripts/check-linux.sh scripts/test.sh`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: HIGH
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `694dd99`, 2026-08-19; re-verified at `cb1ef6c`

## Why this matters

Lane exists so that you can work in a worktree. Anything that breaks *because* you are in a
lane is lane's defect. Two such things are true today, and both were reproduced end to end
before this plan was written.

**Every lane breaks if the tree is ever moved.** `lane new` takes git's default, which
writes an absolute path into the lane's `.git` file:

```
lane .git contains: gitdir: /private/tmp/abswt.clRN/repo/.git/worktrees/spike
after moving repo and lanes together:
  fatal: not a git repository: /private/tmp/abswt.clRN/repo/.git/worktrees/spike
```

**Lanes live outside the repository they belong to.** `lanes_dir()` puts them in a sibling
directory, `../.lanes-<reponame>/<name>`. So a lane is related to its repository by a path
that assumes the two stay adjacent, and moving the repository alone breaks every lane even
with relative pointers.

Both are fixed by the same two changes: put lanes at `<repo>/.lanes/<name>`, and ask git for
relative pointers. Verified together:

```
lane .git:    gitdir: ../../.git/worktrees/spike
admin gitdir: ../../../.worktrees/spike/.git
after moving ONLY the repository:
  branch: spike        status: ''
```

Putting lanes inside the repository buys two more things. Reflink needs source and
destination on one filesystem; a subdirectory guarantees that, a sibling only usually does.
And `.lanes-*` directories stop accumulating beside every repository you have ever used.

## The design

**Placement**: `<repo root>/.lanes/<name>`.

**Pointers**: `git worktree add --relative-paths`, which landed in git 2.48. Lane has no
documented git floor, so probe for it; on an older git, fall back to today's behaviour,
which works in place and only fails on a move.

**Keeping it out of git's way**, in two layers:

1. `.git/info/exclude` gains `.lanes/`, written idempotently by `lane new`. This is the one
   that holds — it lives inside `.git`, so it survives `git clean -xfd`, and it touches no
   file the user shares with anyone.
2. `.lanes/.gitignore` containing `*`, written when the directory is created. Belt and
   braces, and it self-heals on the next `lane new`.

Both were tested. `.git/info/exclude` is *not* undermined by the usual objection that it
does not survive a clone: `.lanes/` only comes into existence when `lane new` runs on that
machine, and that is the moment the entry is written. There is no clone to survive.

Verified that this does not leak into the lanes themselves — `.lanes/.gitignore` sits above
a lane's own worktree root, so files inside a lane stage normally:

```
main worktree status: ''
in the lane: git add newfile.txt -> staged: 'newfile.txt'
```

**Guards, which are mandatory and are the risky part of this plan.** Ignoring `.lanes/` does
not hide it from lane. `git status --porcelain --ignored` still reports it:

```
!! .lanes/
```

and that is exactly what `ignored_entries()` harvests and clones into each new lane. Without
a guard, lane #2 contains a copy of lane #1. On the `--dirty` path it is worse than waste:
`clone_tree(&root, &dest, &skip)` would be writing into a subdirectory of the tree it is
walking. The default path has the same shape, since `.lanes/` is reported as an ignored
entry and `clone_entry` would clone it with the destination inside the source. Either way
the walk descends into what it is writing and nests until the path length limit stops it.

**Not a goal**: making a lane work with no access to its repository. A worktree is a
reference into another directory; copied away from it, git cannot resolve it, and the thing
that would work there is a clone — which gives up the shared object store that makes a lane
cheap. The goal is precise: *a lane is whole wherever its repository is reachable.*

**No migration.** The tool is unreleased. Lanes created by the old layout keep working while
they exist, but `lane ls` will not find them after this lands. Land or remove every open lane
before upgrading.

## Current state

`crates/lane/src/worktree.rs`, the placement rule:

```rust
const LANES_DIRNAME: &str = ".lanes";

pub fn lanes_dir(root: &Path) -> PathBuf {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    root.parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(format!("{LANES_DIRNAME}-{name}"))
}
```

`crates/lane/src/worktree.rs:60`, the filter that needs the first guard — note it already
excludes `.git` in exactly the shape yours should take:

```rust
        .filter(|p| !p.is_empty() && p != ".git")
```

`crates/lane/src/worktree.rs:243`, the `--dirty` arm's walk filter, needing the second:

```rust
            let skip = |rel: &str, _is_dir: bool| rel == ".git" || rel.starts_with(".git/");
```

`crates/lane/src/worktree.rs` calls `git worktree add` twice — once in the
`Materialization::Dirty` arm with `--no-checkout` (the argument list starts around line 233),
once at line 272 for every other arm. Both need the flag.

Line numbers in this plan were re-verified at dispatch, but confirm each excerpt by its
content rather than trusting the number — this file has been edited by several plans.

`crates/lane/src/git.rs` gives you `git` (fails loudly), `try_git` (failure is an empty
string) and `git_ok` (exit status only), each taking `cwd: Option<&Path>`.

Conventions: one-line comments, and only where the reason is not obvious from the code;
`anyhow::Result`; tests in `#[cfg(test)] mod tests` at file end. Commit subjects are
Conventional Commits, `type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 3 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./scripts/test.sh` | `failed: 0`, baseline + 4 |
| Linux gates | `./scripts/check-linux.sh` | see Step 5 |

Record both baselines before starting; at `cb1ef6c` they are 69 and 108.

## Scope

**In scope**: `crates/lane/src/worktree.rs`, `crates/lane/src/cow.rs`,
`crates/lane/tests/cow.rs`, `scripts/check-linux.sh`, `scripts/test.sh`, `README.md`, `USAGE.md`.

**Out of scope**:
- The memory store: nothing under `store.rs`, `audit.rs`, `note.rs` or `.context/`.
- `crates/lane/src/cli.rs`, except if a call site simply will not compile without an edit —
  in which case make the smallest possible change and say so in your report.
- Migrating existing lanes from the old sibling layout.
- `crates/lane/assets/skill.md`.

## Steps

### Step 1: Move lanes inside the repository

Change `lanes_dir` to return `root.join(LANES_DIRNAME)`. The per-repo name suffix exists
only to disambiguate siblings and is now meaningless — remove it, and remove the unused
`file_name()` lookup with it.

**Verify**: `lane new x` in a scratch repo creates `<repo>/.lanes/x`; `lane ls`, `lane path x`
and `lane rm x` all agree on the new location.

### Step 2: Guard against cloning the lanes directory

Two guards, and they defend different things. Do both.

**2a — never select `.lanes` as an entry to clone.** Add `.lanes` to the `ignored_entries`
filter at `worktree.rs:60`, matching the existing `.git` shape. Git reports the entry with
its trailing slash already trimmed, so a plain `p != ".lanes"` matches. Without this, lane
number two contains a copy of lane number one even if nothing recurses.

**2b — make `clone_tree` refuse to walk into its own destination.** Do not hardcode
`.lanes` here. If `dst` is lexically inside `src`, derive the relative component and skip it
in addition to whatever the caller's `skip` closure says:

```rust
// A destination inside the source would be walked into as it is written. Skip it here so
// no caller has to remember, and so renaming the lanes directory cannot reintroduce it.
let contained = dst.strip_prefix(src).ok().map(Path::to_path_buf);
```

then treat a `rel` equal to that component, or beneath it, as skipped.

2b is the durable half: a hardcoded directory name is one refactor away from being
dropped, while a self-containment check protects every present and future caller.

**2c — exclude the whole lanes directory from the `--dirty` walk.** 2b is necessary and
not sufficient. The `--dirty` arm does not go through `ignored_entries`, so 2a never
applies to it; it clones the entire tree through its `skip` closure. 2b excludes only the
destination, so *sibling* lanes are still copied in — verified:

```
second_clean_contains_first=no
second_dirty_contains_first=yes
```

Add `.lanes` to the `--dirty` skip closure alongside `.git`, in the same shape:

```rust
let skip = |rel: &str, _is_dir: bool| {
    rel == ".git" || rel.starts_with(".git/") || rel == ".lanes" || rel.starts_with(".lanes/")
};
```

`--dirty` carries your uncommitted work. Other lanes are separate worktrees and are never
part of that.

Verify this directly rather than by reading: create a lane, put a large file in it, create
a second lane, and confirm the second does not contain the first. Getting it wrong is
recoverable but expensive — the walk nests until the path length limit stops it, which on
this repo means roughly half a million directory entries and several minutes of wall time.

**Verify**:
- `lane new a`, then `lane new b`, then `[ -e <repo>/.lanes/b/.lanes ]` is false
- with an uncommitted change present, and lanes `a` and `b` already existing,
  `lane new c --dirty` completes and `[ -e <repo>/.lanes/c/.lanes ]` is false — test this
  with sibling lanes present, not just one lane, which is what the earlier version of this
  plan missed
- `lane new` output reports a file count in the same order of magnitude as before this change
- a unit test in `cow.rs` proves `clone_tree` skips a destination nested inside its source,
  using a directory name that has nothing to do with lanes — the point is that the guard is
  structural, not that it knows about `.lanes`

### Step 3: Ask git for relative pointers

Add `--relative-paths` to both `git worktree add` invocations, behind a capability probe —
the flag is git 2.48+ and an older git fails the whole command with an unknown-option error:

```rust
/// git 2.48+; older versions get absolute paths, which work in place but not after a move.
fn relative_paths_supported(root: &Path) -> bool {
    try_git(&["worktree", "add", "-h"], Some(root)).contains("--relative-paths")
}
```

Do not add the flag to `worktree list`, `repair` or `remove`.

**Verify**, in a scratch repo:
- `cat <repo>/.lanes/x/.git` → `gitdir: ../../.git/worktrees/x`, relative
- `cat <repo>/.git/worktrees/x/gitdir` → also relative
- move the whole repository to a new path, then inside the lane `git rev-parse --abbrev-ref HEAD`
  → the branch name, and `git status --porcelain` → empty
- `lane ls`, `lane why`, `lane note`, `lane audit`, `lane done` all work in such a lane

### Step 4: Keep it out of git's way

When `lane new` creates the lanes directory:

1. append `.lanes/` to `$(git rev-parse --git-path info/exclude)` if not already present —
   reuse the existing `append_line` idempotence idea, do not add a duplicate line on every run
2. write `*` to `<repo>/.lanes/.gitignore` if that file is absent

**Verify**:
- after `lane new x`, `git status --porcelain` in the main worktree is empty
- `git clean -xfd` in the main worktree prints `Skipping repository .lanes/x`, and
  `git status --porcelain` is still empty afterwards
- `lane new` twice adds exactly one `.lanes/` line to `info/exclude`
- inside a lane, `git add` on a new file stages it normally

### Step 5: Let the Linux gate run from a lane

`scripts/check-linux.sh` mounts the repository at `/w` and copies it to `/build`. From a
lane, `.git` is a file pointing at a path outside the mount, so git fails inside the
container.

Resolve the common directory and mount it read-only at the same absolute path it has on the
host:

```sh
COMMON="$(git -C "$ROOT" rev-parse --path-format=absolute --git-common-dir)"
```

Add a second `-v "$COMMON":"$COMMON":ro` only when `$COMMON` is not already inside `$ROOT`.
When it is — the main checkout — the script must behave exactly as it does today.

**Verify**: the script passes from the main checkout *and* from a lane. Both, not one. If
you are running inside a lane and cannot reach the main checkout, run the lane case, say the
main-checkout case is unverified, and the reviewer will run it.

### Step 6: Cover it, and say where lanes live

Add to `scripts/test.sh` as a new section before the summary, numbered one past the last. Note
`setup` creates the repo at `$TMP/repo`, so lanes are now at `$TMP/repo/.lanes/<name>`, not
`$TMP/.lanes-repo/<name>` — **every existing reference to the old path in this file must be
updated too, or the suite will fail for reasons unrelated to your change.** There are 18 of
them; find them with `grep -n 'lanes-repo' scripts/test.sh` before you start, and expect that
to be the bulk of the diff.

Four new assertions:

```bash
echo "== N. a lane lives inside the repo and survives a move =="
setup
"$LANE" new moved > /dev/null 2>&1
is "the lane is inside the repo" \
   "$([ -d .lanes/moved ] && echo yes || echo no)" "yes"
is "its gitdir pointer is relative" \
   "$(grep -c '^gitdir: \.\.' .lanes/moved/.git)" "1"
is "the main worktree stays clean" "$(git status --porcelain)" ""
( cd "$TMP" && mv repo moved-repo )
is "git still works in the lane after the repo moves" \
   "$(cd "$TMP/moved-repo/.lanes/moved" && git rev-parse --abbrev-ref HEAD)" "moved"
( cd "$TMP" && mv moved-repo repo )
```

Then update the docs: `README.md` and `USAGE.md` both show the old sibling layout —
`USAGE.md`'s "Layout" block and its `lane new` sample output, and `README.md` wherever it
names `.lanes-yourproject`. Say in one sentence that lanes live in `.lanes/` inside the
repository and are excluded via `.git/info/exclude`, so nothing is committed.

**Verify**: `./scripts/test.sh` → `failed: 0`, baseline + 4; `grep -rn 'lanes-' README.md USAGE.md`
→ no matches.

## Done criteria

- [ ] `cargo test` passes, baseline + 3; `./scripts/test.sh` passes, baseline + 4
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` passes from the main checkout **and** from a lane
- [ ] A second lane does not contain a copy of the first, with and without `--dirty`
- [ ] A lane's `.git` holds a relative `gitdir:`, and the repository survives being moved
- [ ] Main worktree `git status` is empty with lanes present, and stays empty after `git clean -xfd`
- [ ] `lane new/ls/path/why/note/audit/done/rm` all work against the new location
- [ ] `git diff --stat -- crates/lane/src/cow.rs crates/lane/assets/skill.md AGENTS.md` → empty

## STOP conditions

- A newly created lane contains a `.lanes` directory. Stop immediately — that is the
  recursion this plan exists to prevent. Measured behaviour: the walk nests
  `.lanes/<name>/.lanes/<name>/...` until it halts on `ENAMETOOLONG` at roughly 223 levels.
  With reflink the file data is shared, so this costs inodes and wall time rather than free
  space — on this repo, on the order of half a million directory entries and several
  minutes before it errors. Recoverable with `rm -rf`, but do not let it run.
- `--relative-paths` is rejected by the git on this machine despite the probe.
- Moving the repository leaves a lane broken even with relative pointers on both sides.
- Making `check-linux.sh` work from a lane would change its behaviour from the main checkout.
- `lane done` cannot fast-forward trunk from a lane in the new location.

## Maintenance notes

- Plan 018 fixed the mirror image of this: a lane *inheriting* state through a
  worktree-relative path, where this is a lane *losing* state through an absolute one.
  Together they say lane's invariant is "a lane's state is its own, and its location is
  nobody's business". Treat a deviation as a bug, not a quirk.
- `.lanes/` is now inside the repository, which means every tool that walks the tree without
  reading ignore rules — `find`, `du`, `tar`, Spotlight, Time Machine — will see N checkouts.
  Reflink means the disk is shared and those tools do not know it. That is the accepted cost
  of this layout; do not try to solve it here.
- Guard 2b is the durable one: stated in terms of source and destination rather than a
  directory name, so renaming `.lanes` cannot reintroduce the recursion. 2a and 2c are
  names and will need updating if the directory is ever renamed — the tests in Step 6 are
  what catch that, so do not weaken them.
- 2b alone is not enough, and the reason is worth remembering: it reasons about the
  destination, while 2a and 2c reason about the *category* of thing being cloned. A lane is
  never cloneable content, whether or not it happens to be this run's destination.
- `.git` has been excluded by these same two filters since the beginning, which is why it
  has never nested despite existing in every worktree. `.lanes` is the same problem with a
  different name.
