# Plan 007: Let a project say which directories are worth carrying, and describe what actually happens

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c2f4ed4..HEAD -- lane lanelib/worktree.py README.md USAGE.md test_lane.sh`
> If any changed since this plan was written, compare the "Current state"
> excerpts against the live code before proceeding; on a mismatch, treat it as
> a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: plans/001-portable-test-suites.md
- **Category**: bug
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

The README's materialization table promises this for the default mode:

| | tracked files | untracked + ignored | dirty state |
|---|---|---|---|
| default | git checks them out | cloned by reference | not carried |

That is not what happens. The default mode clones exactly the ten hardcoded
top-level directories in `WARM_DEFAULT`, and nothing else. Verified: a
gitignored `.env` in the parent tree is absent from the new lane.

The consequence is not academic. `.env`, `.envrc`, `.tool-versions`,
`.python-version`, local sqlite fixtures, `.tox`, `.pnpm-store`, `__pycache__`,
`.terraform` — a lane that omits these does not run the project, which is the
one thing the feature exists to guarantee. The user has no way to say so:
`create()` takes a `warm` parameter (`lanelib/worktree.py:73`) but nothing
plumbs it to a flag, an env var or a config key, so it is dead.

Two changes: make the list configurable per project, and make the docs
describe the mechanism instead of an idealised version of it.

**Considered and rejected**: making the default carry *all* untracked and
ignored files, which is what the README currently claims. It is unbounded —
editor state, OS junk, coverage output, multi-gigabyte caches nobody wants a
second copy of — and it would silently duplicate secrets into a new tree
without the user asking. `--fork` already exists for people who want the whole
tree, and it says so on the tin. The default should stay a list.

## Current state

Files:

- `lanelib/worktree.py` — `WARM_DEFAULT` at `lanelib/worktree.py:18-19`,
  `create()` at `lanelib/worktree.py:73-126`
- `lane` — `cmd_new` at `lane:84-92`, the `new` subparser at `lane:408-414`
- `README.md` — the table at lines 28-31
- `USAGE.md` — the "Open a lane" section

`lanelib/worktree.py:17-19`:

```python
LANES_DIRNAME = ".lanes"
WARM_DEFAULT = ["node_modules", "target", ".venv", "vendor", "dist",
                ".next", ".turbo", ".gradle", "build", ".cargo"]
```

`lanelib/worktree.py:88-90`, where the parameter dies:

```python
    root = main_root()
    base = base or trunk_name(root)
    warm = warm if warm is not None else WARM_DEFAULT
```

`lanelib/worktree.py:110-124`, the default branch. Read the `skip` closure
carefully — it already handles a warm entry that names a **file** rather than a
directory, because the file branch tests `top not in warm_set` where `top` is
the filename for a top-level path:

```python
    else:
        git("worktree", "add", "-b", name, str(dest), base, cwd=root)
        tracked = tracked_set(root)
        warm_set = set(warm)

        def skip(rel, is_dir):
            top = rel.split(os.sep)[0]
            if top == ".git":
                return True
            if is_dir:
                # descend only into warm dirs at the top level
                return os.sep not in rel and top not in warm_set
            return rel in tracked or top not in warm_set

        stats = cow.clone_tree(str(root), str(dest), skip=skip)
```

So `lane.warm = .env` needs no change to `skip` — only a way to get `.env`
into `warm_set`. Do not rewrite the closure.

`lane:84-92`, which reports what happened:

```python
def cmd_new(a):
    dest, stats, notes = wt.create(a.name, base=a.base, fork=a.fork)
    for n in notes:
        print("  " + n)
    print("  %s" % stats)
    print(c("1", str(dest)))
    if a.cd:
        print(dest)
    return 0
```

`notes` is a list of human-readable lines that `create()` builds; adding a line
to it is the established way to tell the user something.

The `git()` helper (`lanelib/memory.py:82`) is already imported into
`worktree.py` and takes `check=False` for calls that are allowed to fail —
`trunk_name` (`lanelib/worktree.py:34-38`) is the exemplar to copy for reading
optional config.

Repo conventions to match:

- `%`-style formatting; **no f-strings anywhere in this repo**
- Module-level constants are UPPER_SNAKE at the top of the file
- Comments justify decisions — `lanelib/worktree.py:76-87` is the house voice

## Commands you will need

| Purpose      | Command          | Expected on success       |
|--------------|------------------|---------------------------|
| Lane suite   | `./test_lane.sh` | `failed: 0`, baseline + 4 |
| Ctx suite    | `./test_ctx.sh`  | `passed: 14   failed: 0`  |
| Syntax check | `python3 -c "import ast,io; [ast.parse(io.open(f).read()) for f in ['lane','lanelib/worktree.py']]"` | exit 0 |

(`./test_ctx.sh` is deleted by plan 008. If that has already landed, skip its row.)


**Record the baseline first.** Run `./test_lane.sh` before you change
anything and write down the number it prints. Plans in this directory land in
whatever order the maintainer chooses, so the only stable expectation is a
*delta*: this plan must leave the suite passing with **4 more assertions**
than that baseline. Any absolute total below is illustrative.

## Scope

**In scope**:
- `lanelib/worktree.py`
- `lane` — `cmd_new` and the `new` subparser
- `README.md`, `USAGE.md`
- `test_lane.sh`

**Out of scope** (do NOT touch, even though they look related):
- The `--fork` branch of `create()`. It already clones everything by design.
- `cow.clone_tree` and the `skip` closure's logic. The closure already supports
  files; changing it risks the "tracked files not re-cloned" assertion in
  section 2 of `test_lane.sh`.
- `post-create`, which reads a `WT_WARM_DIRS` env var. It is a dead hook from
  the superseded design; plan 008 removes it. Do not make the new config match
  its interface for compatibility's sake — nothing depends on it.
- Adding a `.lanerc` or any new config file format.

## Git workflow

- Branch: `advisor/007-configurable-warm-list`
- Commit per step; lowercase imperative subject with a body explaining why.
- Do NOT push or open a PR.

## Steps

### Step 1: Read the warm list from git config

Git config is the right home: it is per-repository, already present, needs no
new file format, is inspectable with a command the user already knows
(`git config --get-all lane.warm`), and is not committed by accident.

Add to `lanelib/worktree.py`, below `WARM_DEFAULT`:

```python
def warm_dirs(root: Path, override=None):
    """Which top-level entries to clone by reference, most specific first.

    Git config rather than a dotfile: per-repo, already there, and one command
    to inspect. Entries may name a file as well as a directory — `.env` is the
    common case and the skip closure already handles it.
    """
    if override:
        return [d for d in override if d]
    configured = git("config", "--get-all", "lane.warm",
                     cwd=root, check=False).splitlines()
    return [d.strip() for d in configured if d.strip()] or list(WARM_DEFAULT)
```

Then use it in `create()`, replacing `lanelib/worktree.py:90`:

```python
    warm = warm_dirs(root, warm)
```

`create()`'s existing `warm=None` parameter now means "override", which is what
callers will pass from the CLI flag.

**Verify**: in a scratch repo,
`git config --add lane.warm .env && python3 -c "
import sys; sys.path.insert(0,'/path/to/lane')
from pathlib import Path
from lanelib.worktree import warm_dirs
print(warm_dirs(Path('.')))"` → `['.env']`

### Step 2: Add a `--warm` flag for one-off overrides

In `lane`'s `new` subparser (`lane:408-414`):

```python
    s.add_argument("--warm", action="append", default=None, metavar="DIR",
                   help="clone this top-level entry by reference; repeatable,"
                        " overrides lane.warm for this run")
```

and pass it through in `cmd_new`:

```python
    dest, stats, notes = wt.create(a.name, base=a.base, fork=a.fork,
                                   warm=a.warm)
```

Two sources — persistent config and a per-run flag — is the whole interface.
Do not add an environment variable; it would be a third way to say the same
thing with no new capability.

**Verify**: `./lane new probe --warm .env 2>&1 | head -3` runs without error;
clean up with `./lane rm probe --force`.

### Step 3: Say what was carried

Silence is what let the mismatch survive. In `create()`, after the default
branch's `clone_tree` call, append to `notes`:

```python
        notes.append("warm: %s" % " ".join(warm))
```

**Verify**: `./lane new probe 2>&1 | grep -c '^  warm: '` → `1`; clean up with
`./lane rm probe --force`.

### Step 4: Make the README table describe the mechanism

Replace the table at `README.md:28-31` and the sentence that follows it. The
new text must say:

- default: git checks out tracked files; the entries in the warm list — the
  ten defaults, or whatever `lane.warm` says — are cloned by reference;
  everything else untracked or ignored is **not** carried
- `--fork`: the entire tree is cloned by reference, dirty state included
- how to configure it: `git config --add lane.warm .env`, repeatable, and
  `--warm` for one run
- why the default is a list rather than everything: unbounded otherwise, and
  `--fork` is there for people who want the whole tree

Keep the table format; it is the right shape, only the middle column's claim is
wrong. Update `USAGE.md`'s "Open a lane" section the same way — it currently
says "`node_modules`, `target`, `.venv`, `dist` and friends arrive by
reference", which is true but does not tell the reader the list is theirs to
change. Add the `git config` line to `USAGE.md`'s Reference section too, beside
the environment variable table.

**Verify**:
- `grep -c 'untracked + ignored' README.md` → `0`
- `grep -c 'lane.warm' README.md USAGE.md` → at least `1` each

### Step 5: Confirm the suites still pass

**Verify**:
- `./test_lane.sh` → `failed: 0` at the baseline count (unchanged; step 6 adds more)
- `./test_ctx.sh` → `passed: 14   failed: 0`

## Test plan

Add a section to `test_lane.sh` before the final summary, modelled on section
2 (which already asserts what does and does not arrive in a new lane). Four
assertions:

```bash
echo "== 17. the warm list is configurable =="
setup
echo "SECRET=1" > .env
printf 'node_modules/\n.env\n' > .gitignore
git add -A && git commit -qm ignore-env

"$LANE" new plain > /dev/null 2>&1
is "an ignored file is not carried by default" \
   "$([ -f "$TMP/.lanes-repo/plain/.env" ] && echo yes || echo no)" "no"
is "the default warm dir still is" \
   "$([ -f "$TMP/.lanes-repo/plain/node_modules/pkg/blob.bin" ] && echo yes || echo no)" "yes"
"$LANE" rm plain --force > /dev/null 2>&1

git config --add lane.warm node_modules
git config --add lane.warm .env
"$LANE" new configured > /tmp/warm.out 2>&1
is "lane.warm carries the file it names" \
   "$([ -f "$TMP/.lanes-repo/configured/.env" ] && echo yes || echo no)" "yes"
is "new reports the warm list" "$(grep -c '^  warm: ' /tmp/warm.out)" "1"
```

Note the second config entry is deliberate: `lane.warm` **replaces** the
default list rather than extending it, so a project that configures it must
name `node_modules` itself. Assert that behaviour rather than working around
it — and say so in the README text from step 4.

Structural pattern: section 2 of `test_lane.sh`.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 4

## Done criteria

ALL must hold:

- [ ] `./test_lane.sh` reports `failed: 0` with baseline + 4 assertions
- [ ] `./test_ctx.sh` prints `passed: 14   failed: 0`
- [ ] `grep -c 'def warm_dirs' lanelib/worktree.py` → `1`
- [ ] `grep -c 'untracked + ignored' README.md` → `0`
- [ ] `git config --add lane.warm .env` then `lane new x` produces a lane
      containing `.env`
- [ ] `./lane new x` prints a `warm: ...` line
- [ ] `git status --short` lists only `lane`, `lanelib/worktree.py`,
      `README.md`, `USAGE.md`, `test_lane.sh`
- [ ] `plans/README.md` status row for 007 updated

## STOP conditions

Stop and report back (do not improvise) if:

- The "tracked files not re-cloned" or "lane status clean" assertions in
  section 2 of `test_lane.sh` change result. Those constrain the `skip`
  closure, which this plan does not modify — a failure there means the warm
  list is reaching code it should not.
- A warm entry containing a path separator (`lane.warm = target/debug`) is
  needed. The current `skip` closure keys on the top-level component only, so
  nested entries silently do nothing. Report it rather than rewriting `skip`;
  supporting nested paths is a real feature and deserves its own plan.
- `git config --get-all` behaves differently inside a lane worktree than in the
  main worktree. It should not — worktrees share `.git/config` — but `create()`
  can be invoked from inside a lane, so confirm before relying on it.

## Maintenance notes

- `lane.warm` replaces rather than extends `WARM_DEFAULT`. That is the simpler
  rule to explain and to reason about, but it means a project that adds `.env`
  loses `node_modules` unless it lists it. If this proves annoying in practice,
  the alternative is a `lane.warm-add` key — do not solve it by merging the two
  lists silently, because then there is no way to *remove* a default.
- Entries name top-level components only. `target/debug` will not work; see the
  STOP condition above.
- The `warm: ...` line in `lane new` output is load-bearing for
  discoverability. If output ever gets a `--quiet` mode, that line should
  survive it, or the mismatch this plan fixes will come back.
- Deferred out of this plan: `lane init` could seed `lane.warm` by detecting
  the project type (a `package.json` implies `node_modules`, a `Cargo.toml`
  implies `target`). That is a nice touch and entirely separable.
