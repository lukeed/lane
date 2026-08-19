# Plan 001: Make both test suites run on macOS/BSD, and prove reflink actually shares extents

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c2f4ed4..HEAD -- test_lane.sh test_ctx.sh`
> If either file changed since this plan was written, compare the "Current
> state" excerpts against the live code before proceeding; on a mismatch,
> treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

`lane` sells copy-on-write worktrees, and APFS is one of its two headline
filesystems. Yet neither test suite runs on macOS: both call GNU-style
`sed -i 's|a|b|' file`, and BSD sed requires an argument to `-i`
(`sed -i '' 's|a|b|' file`). A maintainer on a Mac gets
`sed: 1: "s|...": invalid command code` and no test signal at all.

Every other plan in this directory uses `./test_lane.sh` as its verification
gate. That gate has to work on the machine the work is being done on, so this
plan goes first.

Second, the README currently says extent sharing is unverified. It is now
verified by hand on APFS — a 256 MiB clone consumed 0 MiB of free space — but
nothing in the suite checks it, so a regression in `clone_file` that silently
fell back to a byte copy would still show 42 green assertions. This plan turns
that manual check into an assertion.

## Current state

Files:

- `test_lane.sh` — 42 assertions against real git repos in a tmpdir; the
  primary suite. Uses `sed -i` at 6 sites.
- `test_ctx.sh` — 14 assertions for the standalone `ctx` script. Uses
  `sed -i` at 4 sites.

Both scripts are standalone: no shared helper file, no test framework. They
define their own assertion helpers at the top.

`test_lane.sh:5-13` — the harness both suites follow:

```bash
set -uo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
LANE="$ROOT/lane"
FAKE="$ROOT/tests/fake-reviewer"
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   - $1"; }
bad() { fail=$((fail+1)); echo "  FAIL - $1"; }
is()  { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (want '$3', got '$2')"; fi; }
```

(`test_ctx.sh:4-14` is the same shape but uses `set -euo pipefail` and
`CTX="$ROOT/ctx"`-style variables. Read it before editing.)

The 10 `sed -i` sites, verbatim:

```
test_lane.sh:92   sed -i 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
test_lane.sh:119  sed -i 's|  let undoStack = \[\];|  let undoStack = [];\n  let cursor = 0;|' src/Editor.svelte
test_lane.sh:143  sed -i 's|pub fn refresh|pub fn rotate_token|' src/auth.rs
test_lane.sh:167  sed -i 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
test_lane.sh:168  sed -i 's|    let lock = session.lock()?;|    let lock = session.assume_locked();|' src/sync.rs
test_lane.sh:169  sed -i 's|    transmit(msg)|    backoff_retry(\|\| transmit(msg))|' src/sync.rs
test_ctx.sh:64    sed -i 's|    let parsed = parse(token);|    // NOTE: see RFC\n    let parsed = parse(token);|' src/auth.rs
test_ctx.sh:70    sed -i 's|  let undoStack = \[\];|  let undoStack = [];\n  let cursor = 0;|' src/Editor.svelte
test_ctx.sh:80    sed -i 's|pub fn verify(token: &str) -> bool {|pub fn verify(token: \&str, now: u64) -> bool {|' src/auth.rs
test_ctx.sh:87    sed -i 's|pub fn refresh|pub fn rotate_token|' src/auth.rs
```

Note `test_lane.sh:169` contains escaped `\|` inside a `|`-delimited
expression. Preserve its quoting exactly.

**Already checked, do not re-investigate**: `\n` in the *replacement* text is
expanded by both GNU sed and the BSD sed shipped with current macOS. Only the
`-i` flag differs. Do not rewrite the expressions themselves.

`test_lane.sh:35-62` — section 1, the cow layer. Note that it prints its own
`ok` lines from Python and the shell then adds a fixed count:

```bash
echo "== 1. cow layer =="
PYTHONPATH="$ROOT" python3 - <<'PY'
from lanelib import cow
ok, detail = cow.probe("/tmp")
print("  ok   - probe returns a verdict: %s (%s)" % (ok, detail))
calls = []
real = cow.clone_file
def spy(s, d):
    calls.append(s)
    return real(s, d)
cow.clone_file = spy
import tempfile, os, filecmp
src = tempfile.mkdtemp(); dst = tempfile.mkdtemp() + "/out"
os.makedirs(os.path.join(src, "sub"))
open(os.path.join(src, "a.bin"), "wb").write(os.urandom(4096))
open(os.path.join(src, "sub", "b.bin"), "wb").write(os.urandom(4096))
os.symlink("a.bin", os.path.join(src, "link"))
st = cow.clone_tree(src, dst)
assert len(calls) == 2, "clone_file must be attempted per regular file, got %d" % len(calls)
print("  ok   - clone_file attempted before any fallback (2 files)")
assert filecmp.cmp(os.path.join(src,"a.bin"), os.path.join(dst,"a.bin"), shallow=False)
assert filecmp.cmp(os.path.join(src,"sub","b.bin"), os.path.join(dst,"sub","b.bin"), shallow=False)
print("  ok   - fallback tree is byte-identical")
assert os.path.islink(os.path.join(dst, "link")) and st.links == 1
print("  ok   - symlinks recreated, not dereferenced")
PY
pass=$((pass+4))
```

`lanelib/cow.py` exposes exactly what the new assertion needs:
`probe(path) -> (bool, str)` and `clone_file(src, dst)` which raises
`cow.CloneUnsupported` when the filesystem cannot share.

Repo conventions to match:

- Shell: two-space indent inside functions, `$(...)` not backticks, lowercase
  helper names, a comment above anything non-obvious explaining *why*.
- Assertions read as sentences: `is "warm dir present in lane" "$(...)" "yes"`.
- Comments in this repo justify decisions rather than restate code. Keep that.

## Commands you will need

| Purpose        | Command          | Expected on success            |
|----------------|------------------|--------------------------------|
| Lane suite     | `./test_lane.sh` | `passed: 43   failed: 0`       |
| Ctx suite      | `./test_ctx.sh`  | `passed: 14   failed: 0`       |
| Shell syntax   | `bash -n test_lane.sh && bash -n test_ctx.sh` | exit 0 |

There is no build, no linter, no typechecker and no CI in this repo. The two
suites are the entire verification surface.

## Scope

**In scope** (the only files you should modify):
- `test_lane.sh`
- `test_ctx.sh`
- `README.md` (one section only — see step 4)

**Out of scope** (do NOT touch, even though they look related):
- `lanelib/cow.py` — the clone code itself is correct and verified; this plan
  only observes it.
- Any `.py` file or the `lane` script. If a test fails for a reason other than
  `sed`, that is a real defect and belongs to another plan — report it, do not
  fix it here.
- `test_ctx.sh`'s eventual deletion — plan 008 handles that. Fix it here so it
  runs; do not remove it.

## Git workflow

- Branch: `advisor/001-portable-test-suites`
- One commit per step is fine, or one for the sed change and one for the
  reflink assertion. Message style is lowercase imperative subject with an
  optional body explaining why, e.g.
  `tests: use a portable in-place edit helper`.
- Do NOT push or open a PR.

## Steps

### Step 1: Add a portable in-place edit helper to `test_lane.sh`

Insert directly after the `is()` helper definition (`test_lane.sh:13`):

```bash
# BSD sed wants an argument to -i, GNU sed refuses one. Neither form is
# portable, so do the rename ourselves and keep the expressions untouched.
sedi() {
  local expr="$1"; shift
  for f in "$@"; do
    sed "$expr" "$f" > "$f.sedi" && mv "$f.sedi" "$f"
  done
}
```

**Verify**: `bash -n test_lane.sh` → exit 0, no output.

### Step 2: Replace all 6 `sed -i` calls in `test_lane.sh` with `sedi`

Change only the command name. Every expression, its quoting, and the filename
stay byte-identical. Example, `test_lane.sh:92`:

```bash
# before
sed -i 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
# after
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
```

**Verify**:
- `grep -c 'sed -i' test_lane.sh` → `0`
- `grep -c '^sedi ' test_lane.sh` → `6`
- `./test_lane.sh` → `passed: 42   failed: 0`

### Step 3: Do the same in `test_ctx.sh`

Add the identical `sedi` helper after its `is()` definition
(`test_ctx.sh:14`), then convert its 4 call sites. `test_ctx.sh` runs under
`set -euo pipefail`, so a failing `sed` will abort the script — that is the
behaviour we want.

**Verify**:
- `grep -c 'sed -i' test_ctx.sh` → `0`
- `./test_ctx.sh` → `passed: 14   failed: 0`

### Step 4: Assert that reflink shares extents, not just that it returns 0

`clone_file` succeeding proves the syscall was accepted. It does not prove the
filesystem shared the bytes. Add that check to section 1 of `test_lane.sh`,
inside the existing Python heredoc, immediately before the closing `PY`:

```python
# A successful clone_file only proves the syscall was accepted. Confirm the
# filesystem did not spend the bytes. Skipped where reflink is unavailable,
# because the fallback copy is supposed to cost full price.
supported, _ = cow.probe(src)
if supported:
    import subprocess
    big = os.path.join(src, "big.bin")
    with open(big, "wb") as f:
        f.write(os.urandom(64 * 1024 * 1024))
    subprocess.run(["sync"])
    def free():
        s = os.statvfs(src)
        return s.f_bavail * s.f_frsize
    before = free()
    real(big, big + ".clone")
    subprocess.run(["sync"])
    spent = before - free()
    assert spent < 16 * 1024 * 1024, \
        "64 MiB clone spent %d bytes; extents were not shared" % spent
    print("  ok   - clone shares extents (64 MiB cost %.1f MiB)"
          % (spent / 1048576.0))
else:
    print("  ok   - extent sharing skipped, no reflink on this filesystem")
```

Note it calls `real`, the pre-spy reference captured earlier in the block, so
the clone does not perturb the `calls` assertion above it.

Then bump the counter on `test_lane.sh:62` from `pass=$((pass+4))` to
`pass=$((pass+5))`.

**Verify**:
- `./test_lane.sh` → `passed: 43   failed: 0`
- On APFS or btrfs the output line reads `clone shares extents (64 MiB cost
  0.0 MiB)` or similar, well under the 16 MiB threshold.

### Step 5: Correct the README's "Not verified here" section

`README.md` contains a section headed `## Not verified here` that begins "The
container this was built in runs ext4 on a kernel with no btrfs or XFS
modules, so **extent sharing itself is untested**." That is now false on
macOS. Rewrite the section so it:

- states that extent sharing is verified on APFS, by the suite, and how (a
  64 MiB clone must cost under 16 MiB of free space);
- keeps the honest caveat that btrfs, XFS with `reflink=1`, bcachefs and ZFS
  remain unverified by the maintainer, and that the same assertion covers them
  automatically when the suite runs there;
- keeps the `filefrag -v` tip for inspecting shared extents on Linux.

Also update the assertion count in the `## Tests` section from `42` to `43`.

**Verify**:
- `grep -c 'extent sharing itself is untested' README.md` → `0`
- `grep -c '43 assertions' README.md` → `1`

## Test plan

No new test file. The changes are to the suites themselves:

- Existing coverage must be preserved exactly: 42 lane assertions and 14 ctx
  assertions still pass after the `sedi` conversion. A drop means an
  expression was altered during conversion.
- One new assertion (lane section 1): extent sharing is real on a
  reflink-capable filesystem, skipped with an explicit line otherwise.
- Structural pattern to follow: the existing Python heredoc in section 1 of
  `test_lane.sh`, which asserts with bare `assert` and prints its own
  `  ok   - ` lines.

Run both suites on macOS. If you also have access to a Linux box with GNU sed,
run them there too — the point of `sedi` is that neither platform is special.

## Done criteria

ALL must hold:

- [ ] `grep -c 'sed -i' test_lane.sh test_ctx.sh` reports `0` for both files
- [ ] `bash -n test_lane.sh && bash -n test_ctx.sh` exits 0
- [ ] `./test_lane.sh` prints `passed: 43   failed: 0`
- [ ] `./test_ctx.sh` prints `passed: 14   failed: 0`
- [ ] `grep -c 'extent sharing itself is untested' README.md` returns `0`
- [ ] `git status --short` lists only `test_lane.sh`, `test_ctx.sh`, `README.md`
- [ ] `plans/README.md` status row for 001 updated

## STOP conditions

Stop and report back (do not improvise) if:

- A suite fails after conversion with an error that is not about `sed`. That is
  a real defect in `lane` and belongs to a different plan.
- The `sedi` helper needs different quoting per call site. It should not — if
  one site resists, you have changed an expression; revert it and retry.
- The extent-sharing assertion fails on a filesystem where `cow.probe` returns
  `True`. That is a genuine finding about `clone_file`, not a test bug. Report
  the measured `spent` value.
- The extent-sharing assertion is flaky across runs. Report the spread of
  `spent` values rather than raising the threshold on your own.

## Maintenance notes

- `sedi` writes `<file>.sedi` next to the target and renames over it. Every
  current call site targets a file inside the suite's own `$TMP` repo, so the
  stray file is never visible to git. If a future test edits a file in a
  directory whose `git status` is being asserted, the temp name would show up
  as untracked — use a different scratch location there.
- The 16 MiB threshold on a 64 MiB clone is deliberately loose: free space
  moves under you on a live machine. Measured cost on APFS is 0 bytes. If this
  ever goes flaky, the fix is a larger file, not a larger threshold.
- Deferred out of this plan: adding CI. The repo has none, and choosing a CI
  provider is the maintainer's call, not an executor's. Once CI exists, these
  two commands are the whole gate.
