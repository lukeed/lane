# Plan 029: Put everything lane owns under `.lane/`

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**: `git diff --stat bdc3fac..HEAD -- crates/ test_lane.sh`

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `bdc3fac`, 2026-08-20

## Why this matters

Lane writes to two top-level directories with unrelated names: `.context/` for memory and
`.lanes/` for worktrees. Neither says which tool owns it, and a reader has to learn both.

Worse, the per-branch files are split across two trees keyed by the same thing —
`state/<branch>.json` and `log/<branch>.jsonl` — so "one branch, one file" is something you
infer rather than see, and every operation on a branch's data touches two places.

One directory fixes both:

```
.lane/
  memory/<path>/<ulid>-<slug>.md    active notes    committed
  attic/<path>/<ulid>-<slug>.md     retired notes   committed
  branch/<name>/state.json          per-writer      committed
  branch/<name>/log.jsonl           per-writer      committed
  trees/<name>/                     the lanes       ignored
```

Every name says what it is. `roll_up` becomes a merge of one directory into another;
`discard_branch_files` becomes one removal. And a reader hiding lane from their editor
excludes one path instead of two.

`memory/` also keeps the job `-` was doing: user paths nest under it, so a repository with
its own top-level `attic/` or `state/` cannot collide with lane's.

## Scope of the rename

| today | becomes |
|---|---|
| `.context/` | `.lane/` |
| `.context/-/<path>/` | `.lane/memory/<path>/` |
| `.context/attic/<path>/` | `.lane/attic/<path>/` |
| `.context/state/<branch>.json` | `.lane/branch/<branch>/state.json` |
| `.context/log/<branch>.jsonl` | `.lane/branch/<branch>/log.jsonl` |
| `.lanes/<name>/` | `.lane/trees/<name>/` |

`.git/lane/pending.jsonl` does **not** move. It lives in the git directory on purpose
(plan 018) and is not part of the store.

**No migration code.** This repository is the only one using lane and it is unreleased.
Existing data here is moved with `git mv` in your commit, described in Step 6. Do not add
detection or upgrade logic for a `.context/` directory.

## Current state

`crates/lane/src/store.rs`:

```rust
pub const CONTEXT_DIR: &str = ".context";
pub const NOTES: &str = "-";
pub const ATTIC: &str = "attic";
pub const STATE: &str = "state";
pub const LOG: &str = "log";
```

with `state_file_for` (line 103), `log_file_for` (110) and `state_file` (117) building paths
from them, `all_state` globbing the state directory, `roll_up` merging two files and
deleting one, and `discard_branch_files` removing two files.

`crates/lane/src/worktree.rs`:

```rust
const LANES_DIRNAME: &str = ".lanes";

pub fn lanes_dir(root: &Path) -> PathBuf {
    root.join(LANES_DIRNAME)
}
```

plus two guards that must follow the new path — the `ignored_entries` filter at line 60 and
the `--dirty` arm's `skip` closure — and the structural destination guard inside
`cow::clone_tree`, which needs no change because it reasons about source and destination
rather than a name.

Fifteen files mention the old names. `grep -rIl '\.context\|\.lanes\|CONTEXT_DIR\|LANES_DIRNAME'`
finds them; the list includes `test_lane.sh`, `.gitattributes`, `README.md`, `USAGE.md`,
`explainer.md`, `AGENTS.md`, `crates/lane/assets/skill.md` and `crates/example/src/scenes.rs`.

Conventions: comments only where the reason is not obvious, one line. Commit subjects are
Conventional Commits, under 28 characters, describing WHAT; the reason belongs in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | 80, unchanged |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, 128, unchanged |
| Linux gates | `./scripts/check-linux.sh` | exit 0 |

Both counts must be **unchanged**. This plan renames things; it adds no behaviour. If a
count moves, something was lost or duplicated.

Capture every exit code without a pipe (`cmd > /tmp/out 2>&1; echo $?`).

## Scope

**In scope**: `crates/lane/src/*.rs`, `crates/example/src/scenes.rs`,
`crates/example/src/main.rs`, `test_lane.sh`, `.gitattributes`, `README.md`, `USAGE.md`,
`explainer.md`, `AGENTS.md`, `crates/lane/assets/skill.md`, and this repository's existing
store data.

**Out of scope**:
- Making `lane init` write `.ignore` or `.vscode/settings.json`. The README documents the
  snippets; lane does not manage anyone's editor.
- The state file's format. It stays a JSON map. Turning it into an append-only log is a
  separate decision and must not be folded in here.
- `.git/lane/pending.jsonl`.
- Any behaviour change at all.

**`crates/lane/assets/skill.md` and `crates/example/src/scenes.rs` are prose that was
authored deliberately.** You may change **path strings only** in them. Any other edit —
rewording, reflowing, adding or removing a sentence — is a STOP condition. Your diff on
those two files must be reviewable as a pure path substitution.

## Steps

### Step 1: The constants and the path builders

Rename the constants and rebuild the paths:

```rust
pub const LANE_DIR: &str = ".lane";
pub const NOTES: &str = "memory";
pub const ATTIC: &str = "attic";
pub const BRANCH: &str = "branch";
```

`state_file_for(root, branch)` → `<root>/.lane/branch/<slug>/state.json`;
`log_file_for` → the same directory's `log.jsonl`. Keep `slug(branch, 60)` exactly as it is
— it is what makes a branch name safe as a directory name.

`all_state` must now glob `\.lane/branch/*/state.json` rather than a flat directory.

**Verify**: `cargo build`; then in a scratch repo `lane init && lane note … && lane audit`
produces `.lane/memory/...` and `.lane/branch/main/state.json`.

### Step 2: Per-branch directories in `roll_up` and `discard_branch_files`

`roll_up` currently merges two files and deletes two. It now merges one directory into
another: append the log, merge the state, then remove the source branch's directory.

`discard_branch_files` becomes one `remove_dir_all`.

**Verify**: in a scratch repo, open a lane, record a note, `lane done`, and confirm
`.lane/branch/` contains only the trunk's directory afterwards — the lane's is gone, and its
log lines are in the trunk's `log.jsonl`.

### Step 3: Worktrees under `.lane/trees/`

`lanes_dir` returns `<root>/.lane/trees`. Update both guards to the new path.

**Check what git actually reports** before writing the filter: run
`git status --porcelain -z --ignored` in a repository with a lane and see the exact string,
then filter on that. Guessing here is how the guard silently stops working.

The self-ignoring file moves to `.lane/trees/.gitignore` containing `*`, and the
`.git/info/exclude` entry becomes `.lane/trees/`.

**Verify**, with lanes present:
- `lane new a`, `lane new b` — and `[ -e .lane/trees/b/.lane/trees ]` is false
- with an uncommitted change, `lane new c --dirty` — same check, still false
- `git status --porcelain` in the main worktree is empty
- `git clean -xfd` leaves the lanes intact and the tree still clean

### Step 4: The merge rule

`.gitattributes` becomes `.lane/branch/*/log.jsonl merge=union`, and `lane init` writes that
string. It is still the only rule.

**Verify**: `lane init` in a scratch repo produces exactly that line.

### Step 5: Everything that names the old paths

Update the remaining files. `AGENTS.md`'s `PROTOCOL` and
`crates/lane/assets/skill.md` both reference `.context/`; so do `README.md`, `USAGE.md`,
`explainer.md`, `test_lane.sh` and the tour's `scenes.rs`.

`README.md` also gains a short section — after **Memory** — giving the two snippets for
keeping the store out of editor pickers:

- a repo-root `.ignore` containing `.lane/`, noting that ripgrep and fd honour it, that
  pickers built on them inherit it, and that git ignores the file completely
- `files.exclude` and `search.exclude` entries for `.vscode/settings.json`, for VS Code,
  which does not read `.ignore`

Say plainly that lane does not write either one — they are yours to add.

**Verify**: `grep -rn '\.context\|\.lanes\b\|CONTEXT_DIR\|LANES_DIRNAME' --exclude-dir=.git
--exclude-dir=target --exclude-dir=plans .` → no matches outside `plans/`.

### Step 6: Confirm the moved data still reads

**This repository's store has already been moved** — it is committed on your branch as
`chore: move store to .lane`, recorded by git as 31 pure renames with every one of the 28
note files verified byte-identical. You do not need to move anything.

Until your code change lands, `lane` in this worktree looks for `.context/` and will not
find the store. That is expected, not a bug you introduced.

Once Steps 1-5 are done, confirm the moved data reads correctly:

```
lane check
lane why crates/lane/src/cli.rs
```

Expected: `25 fresh`, everything else zero, and 11 notes listed for `cli.rs`. Those are the
counts recorded before the move. A different number means a path builder is wrong.

Then reinstall the skill, whose asset now differs from the installed copy:
`lane uninstall skill && lane install skill`.

## Done criteria

- [ ] `cargo test` 80 and `./test_lane.sh` 128 — both unchanged
- [ ] `cargo clippy --all-targets` zero warnings; `cargo fmt --all --check` exit 0
- [ ] `./scripts/check-linux.sh` exit 0
- [ ] A fresh `lane init` produces `.lane/` and nothing named `.context` or `.lanes`
- [ ] A second lane contains no copy of the first, with and without `--dirty`
- [ ] Main worktree `git status` is empty with lanes present, and after `git clean -xfd`
- [ ] On this repository, `lane check` reports 25 fresh and `lane why crates/lane/src/cli.rs` lists 11 notes
- [ ] `grep -rn '\.context\|\.lanes\b'` finds nothing outside `plans/`
- [ ] The diff on `skill.md` and `scenes.rs` is path substitutions only

## STOP conditions

- A newly created lane contains a `.lane/trees` directory. Stop at once — that is the
  recursion the guards exist to prevent.
- `cargo test` or `./test_lane.sh` changes count in either direction.
- `lane check` on this repository reports anything other than 25 fresh once the code is
  renamed. The data is already in place; a wrong count means a path builder is wrong, and
  guessing at it risks writing a second store beside the real one.
- You are about to reword anything in `skill.md` or `scenes.rs` beyond a path.
- You conclude `lane init` should write `.ignore` or `.vscode/settings.json`. It should not;
  the README documents them.

## Maintenance notes

- One directory is now the whole footprint. Anything lane writes into a user's repository
  belongs under `.lane/`, and anything that does not belongs in `.git/lane/` — the pending
  queue is the existing example.
- `branch/<name>/` groups a writer's files. If a third per-branch file ever appears it goes
  in that directory and needs no new top-level concept, which is the point of the change.
- `memory/` carries the reservation `-` used to: user paths nest under it, so a repository
  may have its own `attic/` or `branch/` without colliding.
