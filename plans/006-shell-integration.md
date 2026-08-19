# Plan 006: Make the shell integration survive failure and survive `done`

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c2f4ed4..HEAD -- lane test_lane.sh USAGE.md`
> If any changed since this plan was written, compare the "Current state"
> excerpts against the live code before proceeding; on a mismatch, treat it as
> a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: plans/001-portable-test-suites.md, plans/002-fail-with-a-message.md
- **Category**: bug
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

`eval "$(lane shellenv)"` is in the README's install instructions and in
`USAGE.md`'s setup section, so it is on the happy path for every user. Both of
its interesting branches are broken.

**`lane new` cannot fail.** The shell function pipes `lane new` into `tail -1`,
so the exit status it tests belongs to `tail`, which always succeeds. Verified:

```
$ p=$(command lane new work | tail -1); echo "exit=$?  p=$p"
exit=0  p=RuntimeError: lane work already exists at /private/var/folders/...
```

The `&&` then passes, and the shell tries to `cd` into a Python error message.

**`lane done` leaves you in a directory that no longer exists.** The function
runs `cd "$(git rev-parse --show-toplevel)"` *after* `lane done` has deleted
the lane worktree the shell is standing in. `git rev-parse` fails from a
deleted cwd, the command substitution yields the empty string, and `cd ""` is a
no-op — so the user is left in a path that is gone, and every subsequent
command fails until they `cd` somewhere by hand. The `os.chdir` inside
`cmd_done` (`lane:378`) changes the Python process's directory, not the
shell's.

The root cause of both is the same: the shell has no reliable channel to learn
where to go. Give it one.

## Current state

Files:

- `lane` — `cmd_new` at `lane:84-92`, `cmd_done` at `lane:340-381`,
  `cmd_shellenv` at `lane:397-406`, `cmd_path` at `lane:409-411`,
  `print_audit` at `lane:279-294`

`lane:397-406`:

```python
def cmd_shellenv(a):
    print(r'''lane() {
  case "$1" in
    new)  shift; local p; p=$(command lane new --cd "$@" | tail -1) && cd "$p" ;;
    cd)   shift; local p; p=$(command lane path "$1") && cd "$p" ;;
    done) command lane done "${@:2}" && cd "$(git rev-parse --show-toplevel)" ;;
    *)    command lane "$@" ;;
  esac
}''')
    return 0
```

`lane:84-92`:

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

`--cd` today means "print the path a second time, last". That is why the
function needs `tail -1`, and the pipe is what destroys the exit status.

`lane:409-411`:

```python
def cmd_path(a):
    print(wt.lanes_dir(wt.main_root()) / a.name)
    return 0
```

No existence check, so `lane cd nosuchlane` prints a path and the function cds
into nothing.

`lane:279-282`, `print_audit`, whose output `cmd_done` emits:

```python
def print_audit(r):
    s = r["stats"]
    print("memory: +%d new, %d fresh, %d body-drift, %d signature-changed, %d missing"
          % (len(r["created"]), s[TIER_FRESH], s[TIER_BODY], s[TIER_SIG],
             s[TIER_MISSING]))
```

`lane:374-380`, the tail of `cmd_done`:

```python
    wt.fast_forward(root, trunk, branch)
    print("fast-forwarded %s" % trunk)

    if not a.keep:
        os.chdir(str(root))
        wt.remove(Path(lane_path).name, force=True)
        print("removed lane %s" % branch)
    return 0
```

Repo conventions to match:

- `%`-style formatting; **no f-strings anywhere in this repo**
- The shell snippet is emitted from a raw triple-quoted string; keep it two-space
  indented and readable, since users paste it into their `.zshrc`
- Comments justify decisions

## Commands you will need

| Purpose      | Command          | Expected on success       |
|--------------|------------------|---------------------------|
| Lane suite   | `./test_lane.sh` | `failed: 0`, baseline + 4 |
| Ctx suite    | `./test_ctx.sh`  | `passed: 14   failed: 0`  |
| Shell syntax | `./lane shellenv \| bash -n` | exit 0 |
| Zsh syntax   | `./lane shellenv \| zsh -n`  | exit 0 |

(`./test_ctx.sh` is deleted by plan 008. If that has already landed, skip its row.)


**Record the baseline first.** Run `./test_lane.sh` before you change
anything and write down the number it prints. Plans in this directory land in
whatever order the maintainer chooses, so the only stable expectation is a
*delta*: this plan must leave the suite passing with **4 more assertions**
than that baseline. Any absolute total below is illustrative.

## Scope

**In scope**:
- `lane` — `cmd_new`, `cmd_done`, `cmd_path`, `cmd_shellenv`, `print_audit`,
  and the `done` subparser
- `test_lane.sh` — new section
- `USAGE.md` and `README.md` — only if they describe `--cd`; check with
  `grep -rn 'shellenv\|--cd' README.md USAGE.md`

**Out of scope** (do NOT touch, even though they look related):
- The order of operations inside `cmd_done`. Rebase-then-audit is load-bearing
  and correct.
- `os.chdir(str(root))` at `lane:378` — it is needed so the Python process is
  not standing in the directory it is about to remove. Leave it.
- Adding fish or PowerShell support. One shell contract, bash and zsh, is the
  scope.
- `ctx`, `post-create`, `pre-done` — plan 008.

## Git workflow

- Branch: `advisor/006-shell-integration`
- Commit per step; lowercase imperative subject with a body explaining why.
- Do NOT push or open a PR.

## Steps

### Step 1: Define one `--cd` contract for both commands

The contract, which the rest of this plan implements:

> With `--cd`, **stdout carries exactly one line: the directory to cd into.**
> Everything a human would read goes to stderr. The exit status is the
> command's own.

That lets the shell function write `p=$(command lane new --cd "$@") || return`
— no pipe, so no status laundering — while the user still sees the progress
lines on their terminal.

Change `cmd_new` (`lane:84-92`) to:

```python
def cmd_new(a):
    dest, stats, notes = wt.create(a.name, base=a.base, fork=a.fork)
    # With --cd, stdout is reserved for the path so the shell function can
    # capture it without a pipe and keep our real exit status.
    info = sys.stderr if a.cd else sys.stdout
    for n in notes:
        print("  " + n, file=info)
    print("  %s" % stats, file=info)
    print(c("1", str(dest)), file=info)
    if a.cd:
        print(dest)
    return 0
```

**Verify**: `./lane new tmp-check --cd 2>/dev/null | wc -l` → `1`, and the line
is a path. (Clean up with `./lane rm tmp-check --force`.)

### Step 2: Let `print_audit` write somewhere other than stdout

`cmd_done` needs the same split, and its report comes from `print_audit`. Give
it a destination parameter defaulting to today's behaviour:

```python
def print_audit(r, out=None):
    out = out or sys.stdout
```

and add `file=out` to every `print` inside the function (there are five print
statements across the summary, the reviewed block, the review block and the
evicted block — check them all).

`cmd_audit` (`lane:275`) keeps calling `print_audit(r)` unchanged.

**Verify**: `./lane audit | head -1` still prints the `memory: ...` summary
line on stdout.

### Step 3: Give `done` the same `--cd` flag

Add to the `done` subparser (near `lane:463-472`):

```python
    s.add_argument("--cd", action="store_true", help="print path last, for shell fn")
```

In `cmd_done`, add `info = sys.stderr if a.cd else sys.stdout` right after the
"not inside a lane" guard, pass `file=info` to every informational `print` in
the function, pass `print_audit(r, info)`, and end with:

```python
    if a.cd:
        print(root)
    return 0
```

`root` is already bound at `lane:347` (`root = wt.main_root()`) and is the
directory the user should land in — it is the main worktree, which still
exists after the lane is removed.

Note the two early-return error paths (`lane:348-350` and the dirty-lane guard)
already write to `sys.stderr` and return non-zero. Leave them; with `--cd` they
correctly produce no stdout at all, so the shell function will not cd.

**Verify**: from inside a lane with a committed change,
`./lane done --cd 2>/dev/null | wc -l` → `1`, and the line is the main repo
root.

### Step 4: Make `lane path` refuse a lane that is not there

```python
def cmd_path(a):
    dest = wt.lanes_dir(wt.main_root()) / a.name
    if not dest.exists():
        raise LaneError("no lane named %s" % a.name)
    print(dest)
    return 0
```

`LaneError` comes from plan 002; import it in `lane`'s
`from lanelib.memory import (...)` block if it is not already there.

**Verify**: `./lane path nosuchlane` → exit 1, prints
`error: no lane named nosuchlane`, and prints nothing on stdout.

### Step 5: Rewrite the shell function

Replace the body of `cmd_shellenv` (`lane:397-406`) with:

```python
def cmd_shellenv(a):
    # No pipes: a pipeline's exit status belongs to its last command, which is
    # how the previous version came to cd into a Python traceback. With --cd
    # the path is the only thing on stdout, so a plain capture is enough.
    print(r'''lane() {
  case "$1" in
    new)  shift; local p; p=$(command lane new --cd "$@")  || return; cd "$p" ;;
    cd)   shift; local p; p=$(command lane path "$1")      || return; cd "$p" ;;
    done) shift; local p; p=$(command lane done --cd "$@") || return; cd "$p" ;;
    *)    command lane "$@" ;;
  esac
}''')
    return 0
```

`return` with no argument propagates the status of the failed capture, so a
failed `lane new` leaves the shell where it was, with the error already on
stderr.

The `done` branch captures the destination from `lane done` itself rather than
asking git afterwards, which is what fixes the deleted-cwd problem: the answer
is produced by a process that is not standing in the doomed directory.

**Verify**:
- `./lane shellenv | bash -n` → exit 0
- `./lane shellenv | zsh -n` → exit 0
- `./lane shellenv | grep -c 'tail -1'` → `0`

### Step 6: Check the docs describe what now happens

`grep -rn 'shellenv' README.md USAGE.md`. Both files show the install line and
`USAGE.md` says the function "makes `lane new` cd into the lane". If either
documents the `tail -1` behaviour or tells the user to `cd` manually after
`done`, update it. If they only show the install line, no change is needed —
do not pad the diff.

**Verify**: `grep -rc 'tail -1' README.md USAGE.md` → `0` for both.

## Test plan

Add a section to `test_lane.sh` before the final summary. It needs `$ROOT` on
`PATH` so the function's `command lane` resolves to the lane under test:

```bash
echo "== 16. shell integration survives failure and survives done =="
setup
PATH="$ROOT:$PATH"
eval "$(command lane shellenv)"

is "new --cd puts only the path on stdout" \
   "$(command lane new probe --cd 2>/dev/null | wc -l | tr -d ' ')" "1"
command lane rm probe --force > /dev/null 2>&1

cd "$TMP/repo"
before=$PWD
lane new dup > /dev/null 2>&1
lane new dup > /dev/null 2>&1
is "a failed new leaves the shell where it was" "$PWD" "$TMP/.lanes-repo/dup"
cd "$TMP/repo"

lane new land > /dev/null 2>&1
echo "fn x() {}" > src/x.rs && git add -A && git commit -qm x > /dev/null
lane done > /dev/null 2>&1
is "done lands the shell in the main worktree" "$PWD" "$TMP/repo"
is "the directory the shell is in exists" "$([ -d "$PWD" ] && echo yes || echo no)" "yes"
```

Four assertions. Note the second one: after the first `lane new dup` the shell
*is* inside the lane, and the failing second call must not move it — that is
the regression, and today it moves the shell into a directory named after a
traceback.

`setup` leaves the shell in `$TMP/repo`; the section restores it explicitly
because the function under test changes the shell's directory.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 4

## Done criteria

ALL must hold:

- [ ] `./test_lane.sh` reports `failed: 0` with baseline + 4 assertions
- [ ] `./test_ctx.sh` prints `passed: 14   failed: 0`
- [ ] `./lane shellenv | bash -n` and `./lane shellenv | zsh -n` both exit 0
- [ ] `./lane shellenv | grep -c 'tail -1'` → `0`
- [ ] `./lane shellenv | grep -c 'git rev-parse'` → `0`
- [ ] `./lane new <existing> --cd` exits non-zero and prints nothing on stdout
- [ ] `./lane path nosuchlane` exits 1 with an `error:` line
- [ ] `git status --short` lists only `lane`, `test_lane.sh`, and at most
      `README.md` / `USAGE.md`
- [ ] `plans/README.md` status row for 006 updated

## STOP conditions

Stop and report back (do not improvise) if:

- Plan 002 has not landed, so `LaneError` does not exist. Step 4 needs it, and
  step 5's `|| return` only produces a good user experience when the error is
  a single `error:` line rather than a traceback.
- Any existing assertion in sections 1–15 changes result. Sections 2, 3 and 4
  parse `lane new` and `lane done` output with `grep -c`; moving those lines to
  stderr must not affect them, because those sections call `lane` **without**
  `--cd` and redirect with `> /tmp/x.out 2>&1`. If one breaks, report the
  section number rather than reverting the split.
- `zsh -n` accepts the function but `bash -n` does not, or vice versa. Both
  must pass; `local` and `$(...)` are the only non-POSIX constructs used and
  both shells support them.
- Making `cmd_done` write to stderr under `--cd` turns out to hide an error the
  user needs. It should not — errors already go to stderr unconditionally.

## Maintenance notes

- The contract from step 1 is the thing to protect: **`--cd` means stdout is
  the path and nothing else.** Any future `print()` added to `cmd_new` or
  `cmd_done` must take `file=info`. A reviewer seeing a bare `print(` in either
  function should ask about it.
- If a third command ever grows `--cd`, factor `info = sys.stderr if a.cd else
  sys.stdout` into a helper rather than repeating it a third time.
- Deferred out of this plan: the function is bash/zsh only. Fish and
  PowerShell users have no integration and no error message saying so. Adding
  `lane shellenv --fish` is a small, separate change if anyone asks.
