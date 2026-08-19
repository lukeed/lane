# Plan 014: Follow a renamed file instead of discarding its memory

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat c73428e..HEAD -- crates/lane/src/audit.rs crates/lane/src/store.rs crates/lane/src/git.rs`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none — plan 013 makes step 2 simpler but is not required
- **Category**: bug
- **Planned at**: commit `c73428e`, 2026-08-19

## Why this matters

Renaming a source file destroys every note about it, on the next audit, silently:

```
$ git mv src/auth.rs src/token.rs && git commit -qm rename
$ lane audit
memory: +0 new, 0 fresh, 0 body-drift, 0 signature-changed, 1 missing
  evict   src/auth.rs#fn verify  (anchor missing)
$ lane why src/token.rs
no context for src/token.rs
```

The note is in the attic and reachable only by someone who knows to look. Nothing points
from the new path to the old one.

git knew exactly what happened the whole time:

```
$ git diff --name-status -M HEAD~1 HEAD -- src/
R100    src/auth.rs     src/token.rs
```

Renaming and moving files is routine — a refactor, a module split, a directory
reorganisation — and each one quietly costs the accumulated memory for everything it
touched. `anchor-missing` is supposed to mean the thing the note describes is gone. After
a rename the thing is right there under a new name.

The rule this plan installs: **evict when the file is gone, follow it when it moved.**

## Current state

- `crates/lane/src/audit.rs` — `run()` computes `touched` from `git::touched_paths(base)`
  and later evicts any note whose tier is `MISSING` and which is not pinned.
- `crates/lane/src/store.rs` — `Checker::check` returns `MISSING` when
  `self.source(&note.meta.path)` cannot read the file; `evict` moves the note to
  `.context/.attic/<path>/`.
- `crates/lane/src/git.rs` — `touched_paths` already runs
  `git diff --name-only <base>...HEAD`; `try_git` is the pattern for a call allowed to fail.
- `crates/lane/src/note.rs` — `path_from_location` derives a note's path from its
  directory. After plan 013 that is the only source of truth and a rename is a pure
  directory move; before 013 the `path:` field must be updated too.

Note directories are `.context/<path>/` today, and `.context/-/<path>/` after plan 013,
so following a rename is a directory move from `<old>` to `<new>` under whichever root
is current. Read the `NOTES` constant in `store.rs` rather than hardcoding the prefix.

Conventions: one-line comments, `anyhow::Result`, `#[cfg(test)] mod tests` at file end.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 2 |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 4 |

## Scope

**In scope**: `crates/lane/src/git.rs`, `store.rs`, `audit.rs`, `test_lane.sh`, and the
`README.md` / `USAGE.md` lines describing the `anchor-missing` tier.

**Out of scope**:
- Anchor renames. A note on `fn verify` where the function was renamed to `fn check` is a
  different problem and the reviewer already handles it as `superseded`.
- Content-based rename detection of our own note files. Git does that; we only read its
  answer about the *source* tree.
- Making eviction reversible in general. The attic already is.

## Steps

### Step 1: Ask git for renames

Add to `git.rs`:

```rust
/// Renames between base and HEAD, old path to new. Empty when base is unknown.
pub fn renames(base: &str) -> std::collections::HashMap<String, String> {
    let out = try_git(
        &["diff", "--name-status", "--find-renames", &format!("{base}...HEAD")],
        None,
    );
    let mut map = std::collections::HashMap::new();
    for line in out.lines() {
        let mut parts = line.split('\t');
        let Some(status) = parts.next() else { continue };
        if !status.starts_with('R') {
            continue;
        }
        if let (Some(old), Some(new)) = (parts.next(), parts.next()) {
            map.insert(old.to_string(), new.to_string());
        }
    }
    map
}
```

`--find-renames` without a threshold uses git's default, which reported `R100` for a pure
move in the case above and still catches a rename with edits.

**Verify**: a unit test is awkward here (it needs a repo); cover it in step 4 end to end,
and confirm by hand that `renames("HEAD~1")` returns one entry after a `git mv`.

### Step 2: Move the notes before deciding anything is missing

In `audit::run`, after `promote_pending` and before notes are loaded, apply the rename map:
for each `(old, new)` where the note directory for `old` exists, move its contents into
the one for `new`, creating the destination and merging into it if it already exists.

Use `std::fs::rename` per note file rather than moving the directory wholesale, so an
existing destination directory is merged rather than clobbered. Remove the old directory
when it ends up empty.

Before plan 013, also rewrite each moved note's `path:` field to the new path. After 013
the directory is the only source of truth and no content changes at all — which is why 013
makes this simpler, and why this plan must not be the thing that introduces a note rewrite
if 013 has already landed.

Report it: push one line per moved path into the audit output, `moved   <old> -> <new> (N note(s))`.

**Verify**: after `git mv src/auth.rs src/token.rs`, `lane audit` prints a `moved` line and
the note directory for `src/token.rs` holds the note.

### Step 3: Do not evict what merely moved

Renames are applied before the check pass, so by the time a note is checked its path is
already correct and `MISSING` again means what it says. Confirm that ordering holds and
add a one-line comment at the call site saying why it must.

There is one gap worth closing: `audit --base` defaults to empty, so `lane audit` with no
base sees no renames while `lane done` (which passes trunk) does. Default the rename lookup
to the trunk when `base` is empty, using `worktree::trunk_name`, and fall back to no
renames when that is not resolvable.

**Verify**: a bare `lane audit` after a committed rename follows it, not only `lane done`.

### Step 4: Cover it

Add to `test_lane.sh` before the summary. Four assertions:

```bash
echo "== N. a renamed file keeps its memory =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
git mv src/auth.rs src/token.rs && git commit -qm rename
"$LANE" audit > /tmp/mv.out 2>&1
is "audit reports the move" "$(grep -c 'moved' /tmp/mv.out)" "1"
is "the note followed the file" \
   "$("$LANE" why src/token.rs 2>/dev/null | grep -c 'constant-time')" "1"
is "nothing was evicted" \
   "$(find .context/.attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "0"
git rm -q src/token.rs && git commit -qm delete
"$LANE" audit > /dev/null 2>&1
is "a genuine deletion still evicts" \
   "$(find .context/.attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "1"
```

The last assertion is the one that keeps this honest: following renames must not turn into
never evicting. Confirm the first three fail against the current code before changing it.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 4.

### Step 5: Say so in the docs

`README.md` and `USAGE.md` both describe `anchor-missing` as "symbol gone → evict to
`.context/.attic/`". Add that a renamed or moved file is followed rather than evicted, and
that eviction means the file or symbol is genuinely gone.

**Verify**: `grep -c 'renamed' README.md USAGE.md` → at least `1` each.

## Done criteria

- [ ] `cargo test` passes, baseline + 2; `./test_lane.sh` passes, baseline + 4
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `git mv` of a noted file followed by `lane audit` leaves the attic empty and
      `lane why <new path>` showing the note
- [ ] `git rm` of a noted file followed by `lane audit` still evicts
- [ ] `plans/README.md` row updated

## STOP conditions

- Following renames makes any existing assertion fail, in particular section 7, which
  renames a *symbol* (`pub fn refresh` → `pub fn rotate_token`) inside an unmoved file and
  must still evict. If that starts passing notes through, the rename map is being applied
  to anchors rather than paths.
- Git reports a rename for a file whose notes you cannot move because the destination
  directory already holds a note with the same ULID filename. That cannot happen with real
  ULIDs; if it does, report rather than overwriting.
- A rename and a deletion of the same path both appear in one diff. Report the diff.
- Plan 013 has landed and step 2 still needs to rewrite note content. It should not — after
  013 the directory is the path.

## Maintenance notes

- The rule to defend: **evict on evidence, follow on movement.** It is the same rule plan
  011 installs for unparsed languages, applied to paths instead of grammars.
- Rename detection is only as good as the base. `lane done` passes trunk; `lane audit` now
  defaults to trunk. A rename older than the base will not be seen, and the note is evicted
  as before — which is why the attic exists.
- Deferred: a note could record the path it was created against, so a much later rename is
  still traceable. That is a durable-record question and belongs with `.attic/.log/` from
  plan 013, not here.
