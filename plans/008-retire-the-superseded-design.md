# Plan 008: Delete `ctx` and the `.wt` hooks, after moving the coverage they hold

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c2f4ed4..HEAD -- ctx test_ctx.sh post-create pre-done test_lane.sh lanelib/memory.py`
> If any changed since this plan was written, compare the "Current state"
> excerpts against the live code before proceeding; on a mismatch, treat it as
> a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/001-portable-test-suites.md
- **Category**: tech-debt
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

The repo carries two implementations of the same idea and one abandoned
integration design:

- **`ctx`** (697 lines) is the standalone predecessor. Roughly 460 of those
  lines are a byte-for-byte copy of `lanelib/memory.py`; the rest is its own
  CLI. It is not mentioned anywhere in `README.md` or `USAGE.md`, and nothing
  in `lane` or `lanelib/` imports or invokes it.
- **`post-create`** and **`pre-done`** are hooks from the design that `lane`
  replaced. `post-create` shells out to `cp --reflink` — precisely what
  `lanelib/cow.py`'s docstring argues against, because `cp` will not tell you
  whether you got real sharing. `pre-done` invokes a `ctx` binary the README
  no longer documents and writes to a fixed `/tmp/ctx-audit.json`.

The copies have already diverged in a way that loses data. `ctx`'s
`Note.render` whitelist omits `reviewed`, `verdict` and `supersedes`:

```
$ diff <(sed -n '1,460p' ctx) lanelib/memory.py
299c298,299
<                 "lines", "status", "checked", "pinned", "evicted"]
---
>                 "lines", "status", "checked", "reviewed", "verdict",
>                 "supersedes", "pinned", "evicted"]
```

So running `ctx audit` over a store that `lane` has reviewed silently strips
every review verdict from every note it rewrites. Two tools, one directory
format, one of them lossy. That is a trap, not a fallback.

Delete all three. **But not before moving the test coverage `test_ctx.sh`
holds**, which is the real content of this plan: `test_ctx.sh` asserts several
behaviours `test_lane.sh` does not, and deleting it blind would quietly reduce
the suite.

## Current state

Files:

- `ctx` — standalone script, 697 lines, executable, tracked
- `test_ctx.sh` — 14 assertions driving `ctx`
- `post-create`, `pre-done` — shell hooks, executable, tracked, unreferenced
- `test_lane.sh` — 42 assertions at `c2f4ed4`, more as earlier plans land
- `lanelib/memory.py` — its module docstring still opens
  `"""ctx - code-anchored memory for agent worktrees.` (`lanelib/memory.py:2`)
  and it still imports `argparse` (`lanelib/memory.py:17`) and `sys`
  (`lanelib/memory.py:23`), neither of which it uses — all three are leftovers
  from when this file *was* `ctx`.

**Verified at `c2f4ed4`**: nothing references the three files.
`grep -rn 'post-create\|pre-done\|\bctx\b' README.md USAGE.md lane lanelib/ test_lane.sh`
returns only `lanelib/memory.py`'s stale docstring. Re-run it before deleting.

**Coverage held only by `test_ctx.sh` at `c2f4ed4`** — this is the list to
work from, but confirm each one against the live `test_lane.sh`, because plans
003 and 004 add assertions that may already cover some:

| test_ctx.sh section | asserts | in test_lane.sh at c2f4ed4? |
|---|---|---|
| 1 | `init` writes two `merge=union` rules to `.gitattributes` | no |
| 1 | `init` writes the `Context memory` protocol to `AGENTS.md` | no |
| 3 | a comment-only edit does not read as drift | no — plan 004 adds a Rust case |
| 5 | a changed declaration line reports `signature-changed` | no |
| 7 | two branches merged with real `git merge` produce no conflict | no — plan 003 adds this |
| 8 | `--max-notes` evicts, least-read first | no |
| 8 | the eviction reason is recorded as `budget` | no |

`test_ctx.sh:47-50`, the init assertions to port:

```bash
"$CTX" init > /dev/null
is "gitattributes has union rule" "$(grep -c 'merge=union' .gitattributes)" "2"
is "AGENTS.md has protocol" "$(grep -c 'Context memory' AGENTS.md)" "1"
```

`test_ctx.sh:79-83`, the signature assertion:

```bash
sed -i 's|pub fn verify(token: &str) -> bool {|pub fn verify(token: \&str, now: u64) -> bool {|' src/auth.rs
is "sig change detected" \
   "$("$CTX" check --json | python3 -c 'import json,sys;d=json.load(sys.stdin);print([x["tier"] for x in d if x["anchor"]=="fn verify"][0])')" \
   "signature-changed"
```

`test_ctx.sh:110-121`, the budget assertions:

```bash
for i in 1 2 3 4; do
  "$CTX" note -p src/auth.rs -a "fn verify" "filler note number $i about verify" > /dev/null
done
"$CTX" audit > /dev/null
"$CTX" why src/auth.rs -a "fn verify" > /dev/null   # bumps reads on survivors
"$CTX" audit --max-notes 2 --json > /tmp/a.json
is "budget capped to 2" \
   "$(find .context/src/auth.rs -name '*.md' | wc -l | tr -d ' ')" "2"
is "eviction reason recorded" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/a.json"));print(d["evicted"][0]["reason"])')" \
   "budget"
```

`lane` exposes the same surface: `lane init`, `lane check --json`,
`lane audit --max-notes N --json`, `lane why <path> -a <anchor>`. The JSON
shapes match too — `lane`'s `cmd_audit` (`lane:258-276`) emits the same
`created` / `checked` / `evicted` keys plus `reviewer` and `verdicts`.

`test_lane.sh` conventions: `setup` builds a fresh fixture repo and leaves the
shell in `$TMP/repo`; assertions use `is <name> <actual> <expected>`; sections
are numbered and announced with `echo "== N. title =="`.

Repo conventions: `%`-style formatting, no f-strings, SPDX header and docstring
on every module.

## Commands you will need

| Purpose      | Command          | Expected on success       |
|--------------|------------------|---------------------------|
| Lane suite   | `./test_lane.sh` | see step 5                |
| Ctx suite    | `./test_ctx.sh`  | `passed: 14   failed: 0` (until step 4) |
| Syntax check | `python3 -c "import ast,io; ast.parse(io.open('lanelib/memory.py').read())"` | exit 0 |

## Scope

**In scope**:
- `test_lane.sh` — gains the ported assertions
- `ctx`, `test_ctx.sh`, `post-create`, `pre-done` — deleted
- `lanelib/memory.py` — docstring and unused imports only

**Out of scope** (do NOT touch, even though they look related):
- Any behavioural change to `lanelib/memory.py` beyond the docstring and the
  two unused imports. If a ported assertion fails against `lane`, that is a
  **finding**, not a licence to change the implementation — see STOP conditions.
- `PENDING = ".wt/pending.jsonl"` (`lanelib/memory.py:31`) and the `.gitignore`
  line `lane init` writes for it (`lane:71`). The `.wt/` directory is still the
  live location for pending notes; only the *hooks* are dead.
- Rewriting `ctx` as a shim over `lanelib`. Considered and rejected: it is an
  undocumented alias for a subset of `lane`, keeping it means keeping a second
  CLI surface tested and documented forever, and nothing depends on it.

## Git workflow

- Branch: `advisor/008-retire-the-superseded-design`
- Commit the port and the deletion **separately**, so the deletion is a diff a
  reviewer can read in one screen and the port is reviewable on its own.
- Use `git rm` for the deletions so the mode bits and history are handled.
- Lowercase imperative subject with a body explaining why.
- Do NOT push or open a PR.

## Steps

### Step 1: Re-derive the coverage gap against the live suite

Do not trust the table above; plans 003 and 004 may have landed since. For each
row, grep `test_lane.sh` for an equivalent assertion:

```
grep -n 'merge=union\|Context memory' test_lane.sh
grep -n 'comment' test_lane.sh
grep -n 'signature-changed' test_lane.sh
grep -n 'git merge' test_lane.sh
grep -n 'max-notes\|budget' test_lane.sh
```

Write down which are already covered. Port only the rest.

**Verify**: you have an explicit list of assertions to port, and it is a subset
of the seven in the table.

### Step 2: Port the missing assertions into `test_lane.sh`

Add one new section before the final summary block, numbered after the highest
existing section. Translate each ported assertion from `ctx` to `lane`:

- `"$CTX" init` → `"$LANE" init`
- `"$CTX" check --json` → `"$LANE" check --json`
- `"$CTX" audit --max-notes 2 --json` → `"$LANE" audit --max-notes 2 --json`
- `sed -i '...' f` → `sedi '...' f` (the portable helper from plan 001)

The init assertions need care: `test_lane.sh`'s `setup` already runs
`lane init` and commits, so assert against the resulting files rather than
running `init` again:

```bash
echo "== N. init scaffolding and budget behaviour =="
setup
is "gitattributes has both union rules" "$(grep -c 'merge=union' .gitattributes)" "2"
is "AGENTS.md has the protocol" "$(grep -c 'Context memory' AGENTS.md)" "1"

sedi 's|pub fn verify(token: &str) -> bool {|pub fn verify(token: \&str, now: u64) -> bool {|' src/auth.rs
"$LANE" note -p src/auth.rs -a "fn verify" "verify is called on the hot path" > /dev/null
"$LANE" audit > /dev/null
sedi 's|pub fn verify(token: &str, now: u64) -> bool {|pub fn verify(token: \&str, now: u64, tz: i32) -> bool {|' src/auth.rs
is "a changed declaration is signature-changed" \
   "$("$LANE" check --json | python3 -c 'import json,sys;d=json.load(sys.stdin);print([x["tier"] for x in d if x["anchor"]=="fn verify"][0])')" \
   "signature-changed"

for i in 1 2 3 4; do
  "$LANE" note -p src/auth.rs -a "fn verify" "filler note number $i about verify" > /dev/null
done
"$LANE" audit > /dev/null
"$LANE" why src/auth.rs -a "fn verify" > /dev/null
"$LANE" audit --max-notes 2 --json > /tmp/budget.json
is "budget caps the anchor at 2 notes" \
   "$(find .context/src/auth.rs -name '*.md' | wc -l | tr -d ' ')" "2"
is "eviction reason is recorded" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/budget.json"));print(d["evicted"][0]["reason"])')" \
   "budget"
```

Adjust to whatever step 1 said is actually missing. The signature assertion
needs the note to be fingerprinted *before* the declaration changes, which is
why it edits, notes, audits, then edits again.

**Verify**: `./test_lane.sh` → all pass, with the count up by the number of
assertions you ported.

### Step 3: Confirm the ported assertions genuinely exercise `lane`

Each ported assertion must fail if the behaviour it covers regresses. Spot
check the budget one, which is the most valuable and the most intricate: change
`--max-notes 2` to `--max-notes 5` temporarily and confirm the
"budget caps the anchor at 2 notes" assertion fails. Then restore it.

**Verify**: the assertion failed when perturbed, and passes when restored.

### Step 4: Delete the superseded files

Re-run the reference check first:

```
grep -rn 'post-create\|pre-done\|\bctx\b' README.md USAGE.md lane lanelib/ test_lane.sh
```

The only hit should be `lanelib/memory.py`'s docstring, which step 5 fixes. If
anything else matches, STOP.

```
git rm ctx test_ctx.sh post-create pre-done
```

**Verify**:
- `ls ctx test_ctx.sh post-create pre-done 2>&1 | grep -c 'No such file'` → `4`
- `git ls-files | grep -c 'ctx\|post-create\|pre-done'` → `0`

### Step 5: Clean the leftovers in `lanelib/memory.py`

- Line 2: change the docstring's opening line from
  `"""ctx - code-anchored memory for agent worktrees.` to
  `"""Code-anchored memory: notes, anchors, staleness, promotion, eviction.`
  Leave the rest of the docstring alone — the two rules it states are still
  the two rules that matter.
- Delete `import argparse` (`lanelib/memory.py:17`) and `import sys`
  (`lanelib/memory.py:23`). Both are unused; they were needed when this file
  carried a CLI.

**Verify**:
- `python3 -c "import ast,io; t=ast.parse(io.open('lanelib/memory.py').read()); print(sorted({a.name for n in ast.walk(t) if isinstance(n, ast.Import) for a in n.names}))"`
  → does not contain `argparse` or `sys`
- `python3 -c "import lanelib.memory"` → exit 0, no output
- `grep -c '"""ctx - ' lanelib/memory.py` → `0`

### Step 6: Update the README's test line

`README.md`'s `## Tests` section names only `./test_lane.sh`, so it needs only
its assertion count refreshed to the new total. Check whether any other line
mentions two suites.

**Verify**: `grep -c 'test_ctx' README.md USAGE.md` → `0` for both.

## Test plan

No new behaviour, so no new coverage — the point is that coverage does not
*drop*. The gate is arithmetic:

- Before: `test_lane.sh` at N assertions, `test_ctx.sh` at 14.
- After: `test_lane.sh` at N + (number ported), `test_ctx.sh` gone.
- Every row of the coverage table above is either already covered in
  `test_lane.sh` or ported by step 2. Write the mapping into the commit body so
  a reviewer can check it without re-deriving it.

The perturbation check in step 3 is the real test of the port. A ported
assertion that cannot fail is not coverage.

## Done criteria

ALL must hold:

- [ ] `./test_lane.sh` passes, with a count equal to the previous count plus
      the number of assertions ported in step 2
- [ ] `git ls-files | grep -c 'ctx\|post-create\|pre-done'` → `0`
- [ ] `grep -rn 'post-create\|pre-done\|\bctx\b' README.md USAGE.md lane lanelib/ test_lane.sh`
      → no matches
- [ ] `python3 -c "import lanelib.memory"` exits 0
- [ ] The commit body lists each ported assertion and where it now lives
- [ ] `plans/README.md` status row for 008 updated

## STOP conditions

Stop and report back (do not improvise) if:

- A ported assertion fails against `lane`. That means `lane` and `ctx` have
  diverged in behaviour, not just in the render whitelist, and the difference
  needs a decision before anything is deleted. Report which assertion, what
  `ctx` produced and what `lane` produced. **Do not change `lanelib/` to make
  it pass.**
- The reference check in step 4 finds a live reference to `ctx`, `post-create`
  or `pre-done` anywhere outside `lanelib/memory.py`'s docstring.
- `test_ctx.sh` is currently failing before you start. It should pass at 14/14
  after plan 001. A failing baseline means you cannot tell a port from a
  regression — fix or report that first.
- Deleting `ctx` turns out to break `test_lane.sh`. It should not; nothing
  references it. If it does, something in the suite is shelling out to a name
  on `PATH`.

## Maintenance notes

- After this lands there is exactly one implementation of the note format. The
  rule to hold in review: `.context/` has one writer, and it is `lanelib`. Any
  future tool that wants to write notes should import `lanelib.memory`, not
  reimplement `render()` — the divergence documented above is what happens
  otherwise.
- `post-create` did contain one idea worth remembering: it verified sharing
  with `filefrag -v` after copying. Plan 001 moves the equivalent check into
  the test suite, which is where it belongs.
- Deferred out of this plan: `README.md` still describes the `.context/`
  layout and the agent protocol accurately, so nothing there needs to change
  beyond the test count.
