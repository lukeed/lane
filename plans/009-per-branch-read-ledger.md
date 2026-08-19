# Plan 009: Bound the read ledger and make its counts survive a merge

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c2f4ed4..HEAD -- lane lanelib/memory.py test_lane.sh`
> If any changed since this plan was written, compare the "Current state"
> excerpts against the live code before proceeding; on a mismatch, treat it as
> a STOP condition.

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/001-portable-test-suites.md, plans/003-merge-safe-notes.md
- **Category**: bug
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

`lane why` bumps a read counter, and that counter is the second key in the
retention ranking — `pinned > times read > touched by this lane > freshness >
age`. It decides which notes survive the budget and which go to the attic. It
is stored badly in two ways.

**It grows without bound and is committed.** One line per note shown, per
invocation. Verified: 20 runs of `lane why` over 3 notes produced 60 lines and
4 KB, and `lane done` commits `.context/` wholesale, so every one of those
lines is in git history forever. An agent that reads context before each edit —
which is exactly what `AGENTS.md` tells it to do — writes to this file all day.

**The counts do not survive a merge.** The file is `merge=union`, and union
merge keeps distinct lines and collapses identical ones. Records are
`{"id": ..., "at": "<second-granularity ISO>"}`, so two reads of the same note
in the same second are byte-identical and merge into one. Verified: those 60
lines contain only 9 distinct ones. After any merge the ranking is computed
from a number that depends on how fast the user typed.

The fix follows the pattern the note store already uses successfully: **one
file per writer.** Notes never conflict because each note is its own file. Give
each branch its own counts file and the same property falls out — no union
merge, no lost events, no unbounded log.

## Current state

Files:

- `lanelib/memory.py` — `READS` at `lanelib/memory.py:30`, `bump_reads` at
  `lanelib/memory.py:352-359`, `read_counts` at `lanelib/memory.py:362-376`
- `lane` — `cmd_init` writes the merge rule at `lane:45-54`; `cmd_why` calls
  `bump_reads` at `lane:321`; `run_audit` calls `read_counts` at `lane:197`

`lanelib/memory.py:30`:

```python
READS = ".reads.jsonl"
```

`lanelib/memory.py:352-376`:

```python
def bump_reads(root: Path, ids):
    if not ids:
        return
    f = root / CONTEXT_DIR / READS
    f.parent.mkdir(parents=True, exist_ok=True)
    with f.open("a", encoding="utf-8") as fh:
        for i in ids:
            fh.write(json.dumps({"id": i, "at": now_iso()}) + "\n")


def read_counts(root: Path) -> dict:
    f = root / CONTEXT_DIR / READS
    counts = {}
    if not f.exists():
        return counts
    for line in f.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        counts[rec.get("id", "")] = counts.get(rec.get("id", ""), 0) + 1
    return counts
```

`lane:45-54`, where the merge rule is written:

```python
    ga = root / ".gitattributes"
    rules = ["%s/**/*.md merge=union" % CONTEXT_DIR,
             "%s/%s merge=union" % (CONTEXT_DIR, READS)]
    existing = ga.read_text(encoding="utf-8") if ga.exists() else ""
    add = [r for r in rules if r not in existing]
    if add:
        with ga.open("a", encoding="utf-8") as f:
            if existing and not existing.endswith("\n"):
                f.write("\n")
            f.write("\n".join(add) + "\n")
```

Consumers of `read_counts` — there is exactly one, `lane:197` inside
`run_audit`, feeding the sort at `lane:236-242`:

```python
        group.sort(key=lambda n: (
            0 if n.pinned else 1,
            -counts.get(n.id, 0),
            0 if n.path in touched else 1,
            TIER_RANK.get(n.meta.get("status", TIER_FRESH), 0),
            n.id,
        ))
```

(That excerpt is from `c2f4ed4` — read the live version, plan 003 touches the
surrounding loop.)

Helpers already available in `lanelib/memory.py`: `current_branch()`
(`lanelib/memory.py:98-102`), `slug()` (`lanelib/memory.py:68-70`),
`load_notes()` (`lanelib/memory.py:331`).

Repo conventions: `%`-style formatting, **no f-strings**, comments justify
decisions, zero dependencies.

## Commands you will need

| Purpose      | Command          | Expected on success       |
|--------------|------------------|---------------------------|
| Lane suite   | `./test_lane.sh` | previous count + 4        |
| Syntax check | `python3 -c "import ast,io; [ast.parse(io.open(f).read()) for f in ['lane','lanelib/memory.py']]"` | exit 0 |

## Scope

**In scope**:
- `lanelib/memory.py` — `READS`, `bump_reads`, `read_counts`, one migration
- `lane` — the `.gitattributes` rules in `cmd_init`
- `test_lane.sh` — new section
- `USAGE.md` — the Layout section, which names `.reads.jsonl`

**Out of scope** (do NOT touch, even though they look related):
- The ranking itself (`lane:236-242`). This plan changes where the number comes
  from, not what it means or how it is weighted.
- The `merge=union` rule for `*.md` notes. That one is correct and load-bearing.
- Making reads a signal in anything other than retention. If someone wants
  `lane why` to sort by popularity, that is a separate change.
- `ctx` (if still present) — plan 008.

## Git workflow

- Branch: `advisor/009-per-branch-read-ledger`
- Commit per step; lowercase imperative subject with a body explaining why.
- Do NOT push or open a PR.

## Steps

### Step 1: Give each branch its own counts file

Replace `READS` (`lanelib/memory.py:30`) with a directory and a helper:

```python
READS = ".reads"          # directory: one counts file per branch
```

Add near `bump_reads`:

```python
def reads_file(root: Path) -> Path:
    """One file per branch, so two lanes never write the same bytes.

    Same trick as the note store: conflicts are avoided by never sharing a
    path, not by merging cleverly. The old single .reads.jsonl was union-merged
    instead, which silently collapsed same-second reads into one.
    """
    name = slug(current_branch(), 60) or "detached"
    return root / CONTEXT_DIR / READS / ("%s.json" % name)
```

**Verify**: `python3 -c "from lanelib.memory import READS; print(READS)"` → `.reads`

### Step 2: Store counts, not events

A count per note is bounded by the number of notes, which the budget already
caps. An event log is bounded by nothing.

```python
def bump_reads(root: Path, ids):
    if not ids:
        return
    f = reads_file(root)
    f.parent.mkdir(parents=True, exist_ok=True)
    counts = {}
    if f.exists():
        try:
            counts = json.loads(f.read_text(encoding="utf-8"))
        except ValueError:
            counts = {}
    for i in ids:
        counts[i] = counts.get(i, 0) + 1
    # Drop ids the store no longer has, so a branch's file stays proportional
    # to the live note count rather than to everything it has ever seen.
    live = {n.id for n in load_notes(root)}
    counts = {k: v for k, v in counts.items() if k in live}
    f.write_text(json.dumps(counts, indent=0, sort_keys=True) + "\n",
                 encoding="utf-8")


def read_counts(root: Path) -> dict:
    """Sum across every branch's file. Order-independent by construction."""
    d = root / CONTEXT_DIR / READS
    counts = {}
    if not d.exists():
        return counts
    for f in sorted(d.glob("*.json")):
        try:
            part = json.loads(f.read_text(encoding="utf-8"))
        except (ValueError, OSError):
            continue
        for k, v in part.items():
            try:
                counts[k] = counts.get(k, 0) + int(v)
            except (TypeError, ValueError):
                continue
    return counts
```

`sort_keys=True` keeps the file's diff stable, which matters because it is
committed.

**Verify**:

```
python3 - <<'PY'
import json, tempfile, os
from pathlib import Path
from lanelib.memory import read_counts, CONTEXT_DIR, READS
r = Path(tempfile.mkdtemp())
d = r / CONTEXT_DIR / READS
d.mkdir(parents=True)
(d / "main.json").write_text(json.dumps({"a": 2, "b": 1}))
(d / "lane-x.json").write_text(json.dumps({"a": 3}))
assert read_counts(r) == {"a": 5, "b": 1}, read_counts(r)
print("ok")
PY
```
→ prints `ok`

### Step 3: Migrate an existing ledger once

Add to `bump_reads`, before it writes, and to `read_counts`, before it reads —
or factor into one helper called by both:

```python
def _migrate_legacy_reads(root: Path):
    """Fold a pre-per-branch .reads.jsonl into this branch's file, once."""
    legacy = root / CONTEXT_DIR / ".reads.jsonl"
    if not legacy.exists():
        return
    counts = {}
    for line in legacy.read_text(encoding="utf-8").splitlines():
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        i = rec.get("id", "")
        if i:
            counts[i] = counts.get(i, 0) + 1
    f = reads_file(root)
    f.parent.mkdir(parents=True, exist_ok=True)
    if f.exists():
        try:
            existing = json.loads(f.read_text(encoding="utf-8"))
        except ValueError:
            existing = {}
        for k, v in existing.items():
            counts[k] = counts.get(k, 0) + int(v)
    f.write_text(json.dumps(counts, indent=0, sort_keys=True) + "\n",
                 encoding="utf-8")
    legacy.unlink()
```

**Verify**: create a repo with a hand-written `.context/.reads.jsonl`
containing three lines for the same id; run `lane why <path>`; confirm
`.context/.reads.jsonl` is gone and `.context/.reads/<branch>.json` contains
that id with a count of at least 3.

### Step 4: Stop declaring a union merge for the ledger

Per-branch files never collide, so the union rule is not just unnecessary — it
is harmful, because union-merging two JSON objects produces invalid JSON. In
`cmd_init` (`lane:46-47`) reduce `rules` to the notes rule alone:

```python
    rules = ["%s/**/*.md merge=union" % CONTEXT_DIR]
```

Existing repos keep a stale `.context/.reads.jsonl merge=union` line in their
`.gitattributes`; it is inert once the file is gone. Do not attempt to rewrite
users' `.gitattributes` — appending is safe, editing is not.

**Verify**: `./lane init` in a fresh repo → `grep -c 'merge=union' .gitattributes` → `1`

Note this changes an assertion ported from `test_ctx.sh` (or still living
there) which expects `2`. Update it to `1` in the same commit and say why in
the message.

### Step 5: Update the documented layout

`USAGE.md`'s Layout section shows:

```
    .reads.jsonl                  append-only, union-merged
```

Replace with the directory and a one-line reason:

```
    .reads/<branch>.json          read counts, one file per branch
```

**Verify**: `grep -c 'reads.jsonl' USAGE.md README.md` → `0` for both.

## Test plan

Add a section to `test_lane.sh` before the final summary. Four assertions:

```bash
echo "== N. read counts are bounded and merge-stable =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "n1" > /dev/null
"$LANE" note -p src/auth.rs -a "fn verify" "n2" > /dev/null
"$LANE" audit > /dev/null
for i in $(seq 1 20); do "$LANE" why src/auth.rs > /dev/null; done
is "the ledger holds one entry per note, not per read" \
   "$(python3 -c 'import json,glob;print(sum(len(json.load(open(f))) for f in glob.glob(".context/.reads/*.json")))')" "2"
is "counts reflect every read" \
   "$(python3 -c 'import json,glob;d=[json.load(open(f)) for f in glob.glob(".context/.reads/*.json")];print(min(min(x.values()) for x in d))')" "20"
git add -A && git commit -qm reads

git checkout -qb reader-a
"$LANE" why src/auth.rs > /dev/null
git add -A && git commit -qm a
git checkout -q main && git checkout -qb reader-b
"$LANE" why src/auth.rs > /dev/null
git add -A && git commit -qm b
git merge -q --no-edit reader-a > /tmp/rm.out 2>&1
is "two branches of reads merge cleanly" "$?" "0"
is "every ledger file is still valid json" \
   "$(python3 -c 'import json,glob,sys
bad=0
for f in glob.glob(".context/.reads/*.json"):
    try: json.load(open(f))
    except Exception: bad+=1
print(bad)')" "0"
```

The last two are the regression gate: against the current code the merge
produces a union-merged JSONL whose counts silently collapse. Confirm the
"every ledger file is still valid json" assertion is meaningful by checking
that the old code, with two branches writing the same `.reads.jsonl`, yields
duplicated records.

Structural pattern: section 13 from plan 003, which also drives a real
`git merge`.

## Done criteria

ALL must hold:

- [ ] `./test_lane.sh` passes with 4 more assertions than before
- [ ] `grep -c 'reads.jsonl' lanelib/memory.py lane USAGE.md README.md` → `0` for all
- [ ] The step 2 verification script prints `ok`
- [ ] After 20 `lane why` runs over 2 notes, `.context/.reads/` contains one
      file holding exactly 2 keys
- [ ] `./lane init` in a fresh repo writes exactly one `merge=union` rule
- [ ] `git status --short` lists only `lane`, `lanelib/memory.py`,
      `test_lane.sh`, `USAGE.md`
- [ ] `plans/README.md` status row for 009 updated

## STOP conditions

Stop and report back (do not improvise) if:

- Two branches produce the *same* ledger filename, so the per-branch guarantee
  does not hold. `slug()` maps `feature/x` and `feature-x` to the same name.
  Report it; the fix is to append a short hash of the full branch name, but
  changing the naming scheme mid-plan invalidates the migration.
- Pruning ids in `bump_reads` calls `load_notes` often enough to be slow on a
  large store. It runs once per `lane why`, which is interactive, so measure
  before assuming. If it is slow, report rather than dropping the prune — an
  unbounded file is the bug being fixed.
- The budget assertion ported from `test_ctx.sh` section 8 ("evicts least-read
  first") changes result. It depends on `why` bumping reads and on `audit`
  reading them back, which is exactly the seam this plan moves. If it fails,
  the counts are not reaching the ranking — report the actual counts dict.
- Removing the union rule breaks the note merge assertions. It should not; the
  `*.md` rule is untouched.

## Maintenance notes

- The invariant: **one writer per file.** `.context/` now has two families that
  hold it — one file per note, one file per branch — and it is what lets the
  whole store be committed without a lock. Any new state that is per-repo
  rather than per-writer will reintroduce merge conflicts; put it in a
  per-branch file instead.
- Dead branches leave their counts file behind forever. That is deliberate:
  deleting a file that another branch may have modified is a delete/modify
  conflict, which is worse than a few kilobytes. If the directory ever gets
  genuinely large, garbage-collect it in a single explicit command
  (`lane gc`), on trunk, not as a side effect of audit.
- The one-time migration in step 3 deletes `.context/.reads.jsonl`. During the
  transition window, a branch that migrates and a branch that still appends can
  produce a delete/modify conflict on that path. It resolves by taking the
  deletion. Worth a line in the release notes if this is ever released.
