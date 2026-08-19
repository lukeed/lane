# Plan 002: Fail with a message, not a Python traceback

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c2f4ed4..HEAD -- lane lanelib/memory.py lanelib/worktree.py test_lane.sh USAGE.md`
> If any of those changed since this plan was written, compare the "Current
> state" excerpts against the live code before proceeding; on a mismatch,
> treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: plans/001-portable-test-suites.md (its verification gate is
  `./test_lane.sh`, which does not run on macOS until 001 lands)
- **Category**: dx
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

Every ordinary failure in `lane` reaches the user as a raw Python traceback.
Creating a lane that already exists, rebasing into a conflict, landing onto a
diverged trunk, running `done` in a dirty lane — all of these are expected
states with documented recoveries in `USAGE.md`, and all of them currently
print a stack trace with the real message on the last line:

```
$ lane new dup
Traceback (most recent call last):
  ...
  File "/Users/you/.lane/lanelib/worktree.py", line 93, in create
    raise RuntimeError("lane %s already exists at %s" % (name, dest))
RuntimeError: lane dup already exists at /Users/you/.lanes-proj/dup
```

A traceback tells the user they found a bug. They did not — they hit a normal
guardrail. Worse, it buries the recovery instructions that `USAGE.md` already
documents.

This plan also removes `--allow-dirty`, a flag that cannot work. `git rebase`
refuses to run with a dirty index or dirty tracked files regardless of what
`lane` decides, so the flag only ever converts a clean error into a confusing
one. Verified: `lane done --allow-dirty` with a staged file fails with
`error: cannot rebase: Your index contains uncommitted changes` — as a
traceback.

## Current state

Files:

- `lane` — the CLI entry point. `main()` at `lane:414` parses args and
  dispatches; there is no exception handling anywhere in it.
- `lanelib/memory.py` — holds the `git()` subprocess helper that raises on
  failure (`lanelib/memory.py:82-91`).
- `lanelib/worktree.py` — raises for "lane already exists"
  (`lanelib/worktree.py:93`) and "trunk has diverged"
  (`lanelib/worktree.py:162`).

`lanelib/memory.py:82-91`:

```python
def git(*args: str, cwd: Path = None, check: bool = True) -> str:
    proc = subprocess.run(
        ["git"] + list(args),
        cwd=str(cwd) if cwd else None,
        capture_output=True,
        text=True,
    )
    if check and proc.returncode != 0:
        raise RuntimeError("git %s failed: %s" % (" ".join(args), proc.stderr.strip()))
    return proc.stdout.strip()
```

`lane:485-486`, the end of `main()`:

```python
    a = p.parse_args(argv)
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
```

`lane:346-360`, the part of `cmd_done` this plan changes:

```python
    lane_path = repo_root()
    root = wt.main_root()
    if lane_path == root:
        print("error: not inside a lane", file=sys.stderr)
        return 2
    branch = current_branch()
    trunk = a.trunk or wt.trunk_name(root)

    if wt.is_dirty(lane_path) and not a.allow_dirty:
        print("error: lane is dirty; commit or stash first", file=sys.stderr)
        return 1

    git("rebase", trunk, cwd=lane_path)
    print("rebased onto %s" % trunk)
```

`lane:466` declares the flag: `s.add_argument("--allow-dirty", action="store_true")`

`lanelib/worktree.py:45-46`, the dirtiness probe:

```python
def is_dirty(path: Path) -> bool:
    return bool(git("status", "--porcelain", cwd=path, check=False).strip())
```

Note this counts untracked files as dirt. `git rebase` does not care about
untracked files, so a lane with a stray scratch file is refused today for no
reason.

Repo conventions to match:

- `%`-style string formatting throughout. **No f-strings anywhere in this
  repo** — match that.
- Every module starts with `# SPDX-License-Identifier: MIT` and a docstring.
- Command functions return an int exit code; `main()` returns it and
  `sys.exit()` carries it out. `cmd_done` already models the good pattern at
  `lane:348-350`: print `error: ...` to stderr and return a code.
- Comments explain *why*, not *what*. See `lanelib/worktree.py:139-141` and
  `lane:341-345` for the house voice.
- `from __future__ import annotations` at the top of every module.

Exit codes in use today: `0` success, `1` failure, `2` "not inside a lane".
Keep that meaning.

## Commands you will need

| Purpose      | Command          | Expected on success       |
|--------------|------------------|---------------------------|
| Lane suite   | `./test_lane.sh` | `failed: 0`, baseline + 3 |
| Ctx suite    | `./test_ctx.sh`  | `passed: 14   failed: 0`  |
| Syntax check | `python3 -c "import ast,io; [ast.parse(io.open(f).read()) for f in ['lane','lanelib/memory.py','lanelib/worktree.py']]"` | exit 0 |

(`./test_ctx.sh` is deleted by plan 008. If that has already landed, skip its row.)


**Record the baseline first.** Run `./test_lane.sh` before you change
anything and write down the number it prints. Plans in this directory land in
whatever order the maintainer chooses, so the only stable expectation is a
*delta*: this plan must leave the suite passing with **3 more assertions**
than that baseline. Any absolute total below is illustrative.

There is no linter, typechecker or CI in this repo.

## Scope

**In scope**:
- `lanelib/memory.py` — add the exception class; raise it from `git()`
- `lanelib/worktree.py` — raise it from the two existing sites; narrow `is_dirty`
- `lane` — catch it in `main()`; drop `--allow-dirty`; improve the rebase message
- `test_lane.sh` — new assertions
- `USAGE.md` — the `--allow-dirty` reference

**Out of scope** (do NOT touch, even though they look related):
- `ctx` — the superseded standalone script. Plan 008 decides its fate; leave
  it exactly as it is, including its own lack of error handling.
- `post-create`, `pre-done` — dead hooks, also plan 008.
- `lanelib/review.py` — its backends already swallow their own failures by
  design (`return {}` on any error, so `lane done` works offline). Do not
  convert those to exceptions.
- The `error: not inside a lane` path at `lane:348` — it is already correct.

## Git workflow

- Branch: `advisor/002-fail-with-a-message`
- Commit style: lowercase imperative subject, optional body explaining why.
  Example from this repo's history:
  `lane rm: stop discarding unlanded commits`
- Do NOT push or open a PR.

## Steps

### Step 1: Add a `LaneError` exception and raise it from `git()`

In `lanelib/memory.py`, immediately above the `git()` helper
(`lanelib/memory.py:82`), add:

```python
class LaneError(RuntimeError):
    """An expected failure with a message worth showing as-is.

    Subclasses RuntimeError so nothing that already catches RuntimeError
    changes behaviour. `main()` catches this and prints the message; anything
    else still reaches the user as a traceback, which is what a real bug
    deserves.
    """
```

Change the raise inside `git()` to use it:

```python
    if check and proc.returncode != 0:
        raise LaneError("git %s failed: %s" % (" ".join(args), proc.stderr.strip()))
```

**Verify**: `python3 -c "from lanelib.memory import LaneError; print(issubclass(LaneError, RuntimeError))"` → `True`

### Step 2: Raise `LaneError` from the two sites in `worktree.py`

Import it alongside the existing names at `lanelib/worktree.py:15`:

```python
from .memory import LaneError, git, now_iso
```

Then change `lanelib/worktree.py:93` and `lanelib/worktree.py:162` from
`raise RuntimeError(` to `raise LaneError(`. Leave both messages unchanged.

**Verify**: `grep -c 'raise RuntimeError' lanelib/worktree.py lanelib/memory.py` → `0` for both.

### Step 3: Catch it in `main()`

Import `LaneError` in `lane`'s existing `from lanelib.memory import (...)`
block (`lane:22-27`) — keep the list alphabetised as it already is.

Replace the tail of `main()` (`lane:485-486`):

```python
    a = p.parse_args(argv)
    return a.fn(a)
```

with:

```python
    a = p.parse_args(argv)
    try:
        return a.fn(a)
    except LaneError as e:
        print("error: %s" % e, file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130
```

**Verify**: from inside a git repo with an existing lane,
`./lane new <existing-name> 2>&1 | head -2` prints a single
`error: lane <name> already exists at ...` line and no `Traceback`.

### Step 4: Give `done` an actionable rebase failure

A rebase conflict is the one failure where the user has real work to do, and
`USAGE.md` already documents the recovery. Replace `lane:358`:

```python
    git("rebase", trunk, cwd=lane_path)
```

with a wrapped call that keeps git's own diagnosis and appends the recovery:

```python
    try:
        git("rebase", trunk, cwd=lane_path)
    except LaneError as e:
        # The lane is left mid-rebase on purpose: the conflict has to be
        # resolved in the tree that has both sides of it. Pending notes are
        # untouched, so rerunning `done` after --continue costs nothing.
        raise LaneError(
            "%s\n"
            "  resolve in the lane, then: git rebase --continue && lane done"
            % e)
```

**Verify**: `grep -c 'git rebase --continue && lane done' lane` → `1`

### Step 5: Remove `--allow-dirty` and stop counting untracked files as dirt

`git rebase` refuses a dirty index or dirty tracked files no matter what
`lane` decides, so the flag can never let a `done` succeed. It can only turn a
clear refusal into an obscure one. Untracked files, meanwhile, do not block a
rebase at all and should never have been refused.

1. In `lanelib/worktree.py`, narrow `is_dirty` (`lanelib/worktree.py:45-46`):

```python
def is_dirty(path: Path) -> bool:
    """Tracked changes only. Untracked files do not block a rebase, so
    refusing to land because of a stray scratch file helps nobody."""
    return bool(git("status", "--porcelain", "--untracked-files=no",
                    cwd=path, check=False).strip())
```

2. In `lane`, delete `s.add_argument("--allow-dirty", action="store_true")`
   (`lane:466`).

3. In `cmd_done`, replace the guard (`lane:354-356`) with:

```python
    if wt.is_dirty(lane_path):
        print("error: lane has uncommitted changes; commit or stash first,"
              " the rebase will refuse them either way", file=sys.stderr)
        return 1
```

4. In `USAGE.md`, find the "When things go wrong" entry that reads
   "**`lane is dirty`** — commit or stash. `--allow-dirty` skips the check but
   the rebase will likely fail anyway." Replace it with an entry that says
   commit or stash, and that untracked files are fine and do not need
   stashing. Do not mention `--allow-dirty`; it no longer exists.

**Verify**:
- `grep -rc 'allow.dirty\|allow_dirty' lane lanelib/ USAGE.md README.md` → `0` everywhere
- `./lane done --allow-dirty 2>&1 | grep -c 'unrecognized arguments'` → `1`

### Step 6: `lane ls` must survive a lane whose branch is gone

Not strictly an error-handling change, but it is the one remaining place a
normal state produces a crash, and it is one line. `cmd_ls` (`lane:95-108`)
reads `p / PENDING` with `pend.read_text()` and no encoding. Add
`encoding="utf-8"` to match every other read in the codebase.

**Verify**: `grep -n 'read_text()' lane` → no matches.

## Test plan

Add a new section to `test_lane.sh`, after the last existing section and
before the final `echo` / summary block. Model it on section 11, which is the
newest and uses the same helpers:

```bash
echo "== 12. expected failures print a message, not a traceback =="
setup
"$LANE" new dup > /dev/null 2>&1
"$LANE" new dup > /tmp/dup.out 2>&1
is "duplicate lane exits 1" "$?" "1"
is "duplicate lane has no traceback" "$(grep -c 'Traceback' /tmp/dup.out)" "0"
is "duplicate lane explains itself" "$(grep -c '^error: lane dup already exists' /tmp/dup.out)" "1"
```

Cases to cover (three assertions, matching the 46 total above):

1. exit code is 1, not a crash
2. stdout+stderr contain no `Traceback`
3. the message is a single `error: ...` line naming the problem

Do **not** add a test that drives a rebase conflict — it needs a conflicted
tree and leaves the lane mid-rebase, which the suite's `setup` does not clean
up reliably. Step 4's grep check is the gate for that path.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 3

## Done criteria

ALL must hold:

- [ ] `grep -c 'raise RuntimeError' lane lanelib/memory.py lanelib/worktree.py` → `0` for all three
- [ ] `grep -rc 'allow.dirty\|allow_dirty' lane lanelib/ USAGE.md README.md` → `0` for all
- [ ] `grep -n 'read_text()' lane` → no matches
- [ ] `./test_lane.sh` reports `failed: 0` with baseline + 3 assertions
- [ ] `./test_ctx.sh` prints `passed: 14   failed: 0`
- [ ] Running `./lane new <name>` twice prints one `error:` line and no `Traceback`
- [ ] `git status --short` lists only `lane`, `lanelib/memory.py`,
      `lanelib/worktree.py`, `test_lane.sh`, `USAGE.md`
- [ ] `plans/README.md` status row for 002 updated

## STOP conditions

Stop and report back (do not improvise) if:

- Catching `LaneError` in `main()` swallows a failure that is genuinely a bug
  (e.g. a `KeyError` or `AttributeError` now shows as `error: ...`). It should
  not — `LaneError` is only raised from the three sites in steps 1 and 2. If
  you find a fourth raise site, report it rather than converting it.
- Narrowing `is_dirty` breaks an existing assertion. Two callers exist
  (`cmd_done` and `cmd_ls`); `cmd_ls` prints `dirty`/`clean` for display only.
  If a test depended on untracked files reading as dirty, report it — the
  behaviour change may be intended but the call is the maintainer's.
- Removing `--allow-dirty` turns out to break something outside the in-scope
  files. `grep -rn 'allow' .` before deleting.

## Maintenance notes

- The rule going forward: anything the user can reasonably do wrong raises
  `LaneError` with a message they can act on. Anything that means `lane` itself
  is broken raises whatever it raises and shows a traceback. Reviewers should
  push back on `except Exception` appearing anywhere near `main()` — that would
  erase the distinction this plan creates.
- `lanelib/review.py` deliberately does not follow this pattern: a reviewer
  backend that fails returns `{}` so `lane done` still finishes offline. If a
  future change makes review failures fatal, that decision needs its own
  discussion.
- Deferred out of this plan: `lane path` and the `cd` branch of `shellenv`
  still name lanes that may not exist. Plan 006 covers the shell integration.
