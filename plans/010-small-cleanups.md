# Plan 010: Clear the four small things that mislead a reader

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
- **Effort**: S
- **Risk**: LOW
- **Depends on**: plans/001-portable-test-suites.md
- **Category**: tech-debt
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

Four small things, each of which makes a reader believe something untrue about
the code. None is a bug a user will hit; together they are the difference
between a codebase that reads as deliberate and one that reads as drifted.
This repo's comments are unusually careful about explaining *why*, which makes
the leftovers stand out more, not less.

## Current state

### 1. A guard that does nothing

`lanelib/memory.py:171-179`, inside `resolve_anchor`:

```python
    for pat in DECL_PATTERNS:
        rx = re.compile(pat.format(name=re.escape(name)))
        if kw and kw not in pat and kw not in ("fn", "def", "class", "func", "function"):
            pass
        for i, line in enumerate(lines):
            if rx.search(line):
                if kw and not re.search(r"\b%s\b" % re.escape(kw), line):
                    continue
                return (i + 1, _find_end(lines, i) + 1)
    return None
```

The `if ... : pass` looks like a filter that skips patterns incompatible with
the anchor's keyword. It does nothing at all. The keyword is in fact enforced,
two lines below, by the `if kw and not re.search(...)` check against the
matched line — which is the correct place for it, because it tests the real
text rather than the pattern's source. The dead branch is a leftover that reads
as a guard.

### 2. An import that is never used

`lane:22-27` imports `ATTIC` from `lanelib.memory`:

```python
from lanelib.memory import (
    ATTIC, CONTEXT_DIR, PENDING, READS, TIER_BODY, TIER_FRESH, TIER_MISSING,
    TIER_RANK, TIER_SIG, Note, bump_reads, check_note, current_branch,
    evict, git, load_notes, note_dir, now_iso, promote_pending, read_counts,
    repo_root, resolve_anchor, slug, touched_paths, ulid,
)
```

`ATTIC` appears nowhere else in `lane`. Eviction goes through `evict()`, which
owns the attic path itself (`lanelib/memory.py:447-455`).

### 3. A gitignore rule that can never match

`lane:70-74`, in `cmd_init`:

```python
    gi = root / ".gitignore"
    for line in (".wt/pending.jsonl", ".lanes-*"):
        if not gi.exists() or line not in gi.read_text(encoding="utf-8"):
            with gi.open("a", encoding="utf-8") as f:
                f.write(line + "\n")
```

`.lanes-*` is written into the repository's `.gitignore`, but lanes are created
*outside* the repository — `lanes_dir` (`lanelib/worktree.py:41-42`) returns
`root.parent / (".lanes-" + root.name)`. Git never sees them, so the rule
matches nothing. It tells a reader that lanes might land inside the repo.

### 4. A summary that describes the wrong moment

`lane:279-292`, `print_audit`:

```python
def print_audit(r):
    s = r["stats"]
    print("memory: +%d new, %d fresh, %d body-drift, %d signature-changed, %d missing"
          % (len(r["created"]), s[TIER_FRESH], s[TIER_BODY], s[TIER_SIG],
             s[TIER_MISSING]))
    if r.get("reviewed"):
        print("  reviewed %d drifted note(s) via %s" % (len(r["reviewed"]),
                                                        r["reviewer"]))
```

`stats` is counted before the reviewer runs. A note the model judged `holds` is
refreshed to `fresh`, but the summary above still counts it under
`body-drift`, so the two lines appear to contradict each other:

```
memory: +2 new, 7 fresh, 1 body-drift, 0 signature-changed, 0 missing
  reviewed 1 drifted note(s) via anthropic(claude-haiku-4-5-20251001)
  holds         src/auth.rs#fn verify
```

**Considered and rejected**: recounting after review. The pre-review numbers
are the honest description of what the hash check found, which is what the line
is reporting, and they also feed `--json`'s `checked` key where a stable
meaning matters more than a tidy one. The fix is to say which moment the
numbers describe.

Repo conventions: `%`-style formatting, **no f-strings**, comments justify
decisions.

## Commands you will need

| Purpose      | Command          | Expected on success       |
|--------------|------------------|---------------------------|
| Lane suite   | `./test_lane.sh` | unchanged count, 0 failed |
| Syntax check | `python3 -c "import ast,io; [ast.parse(io.open(f).read()) for f in ['lane','lanelib/memory.py']]"` | exit 0 |

## Scope

**In scope**:
- `lanelib/memory.py` — the dead branch in `resolve_anchor`
- `lane` — the `ATTIC` import, the `.lanes-*` gitignore line, `print_audit`'s
  wording

**Out of scope** (do NOT touch, even though they look related):
- `resolve_anchor`'s actual matching behaviour. Removing the dead branch must
  change nothing; if it does, STOP.
- `DECL_PATTERNS` and pattern ordering. A bare name currently matches by
  pattern order rather than by position in the file, which is a real
  limitation — and a separate decision, not a cleanup.
- The `.wt/pending.jsonl` gitignore line. That path is live.
- The JSON shape emitted by `cmd_audit`. Only the human-readable line changes.
- `lanelib/memory.py`'s stale `"""ctx - ...` docstring and its unused
  `argparse` / `sys` imports — plan 008 owns those, to keep the two diffs from
  colliding.

## Git workflow

- Branch: `advisor/010-small-cleanups`
- One commit per numbered item is ideal — each is independently revertable and
  independently reviewable.
- Lowercase imperative subject with a body explaining why.
- Do NOT push or open a PR.

## Steps

### Step 1: Delete the dead branch, and note where the check really happens

Remove these two lines from `resolve_anchor` (`lanelib/memory.py:173-174`):

```python
        if kw and kw not in pat and kw not in ("fn", "def", "class", "func", "function"):
            pass
```

Add a comment above the surviving check so the next reader does not
re-introduce it:

```python
            if rx.search(line):
                # The keyword is enforced against the matched line, not the
                # pattern: `fn verify` must find a line that really says `fn`,
                # whichever pattern happened to match it.
                if kw and not re.search(r"\b%s\b" % re.escape(kw), line):
                    continue
```

**Verify**:
- `grep -c 'kw not in pat' lanelib/memory.py` → `0`
- `./test_lane.sh` → same pass count as before, `failed: 0`
- Anchor resolution is unchanged:

```
python3 - <<'PY'
from lanelib.memory import resolve_anchor
src = open("lanelib/memory.py").read()
assert resolve_anchor(src, "def resolve_anchor") is not None
assert resolve_anchor(src, "resolve_anchor") is not None
assert resolve_anchor(src, "class Note") is not None
assert resolve_anchor(src, "fn resolve_anchor") is None, "keyword must still filter"
print("ok")
PY
```
→ prints `ok`

### Step 2: Drop the unused import

Remove `ATTIC` from `lane`'s `from lanelib.memory import (...)` block
(`lane:23`), keeping the remaining names alphabetised and the line wrapping
tidy.

**Verify**:
- `grep -c 'ATTIC' lane` → `0`
- `./lane --help` → exits 0 and prints usage

### Step 3: Stop writing a rule that cannot match

In `cmd_init` (`lane:71`), reduce the tuple to the one live path:

```python
    for line in (".wt/pending.jsonl",):
```

Do not remove `.lanes-*` from any existing repo's `.gitignore` — appending is
safe, editing a user's file is not.

**Verify**: `./lane init` in a fresh scratch repo →
`grep -c 'lanes-' .gitignore` → `0`, and `grep -c '.wt/pending.jsonl' .gitignore` → `1`

### Step 4: Say which moment the counts describe

In `print_audit` (`lane:281`), change the summary to name the check explicitly:

```python
    print("memory: +%d new; checked %d: %d fresh, %d body-drift,"
          " %d signature-changed, %d missing"
          % (len(r["created"]), sum(s.values()), s[TIER_FRESH], s[TIER_BODY],
             s[TIER_SIG], s[TIER_MISSING]))
```

so the reviewed lines that follow read as what happened *next* rather than as
a contradiction. Add a one-line comment above it recording the decision:

```python
    # Counted before the reviewer ran: this line reports what the hash check
    # found, and the verdict lines below report what was done about it.
```

**Verify**:
- `./lane audit | head -1` matches `^memory: \+[0-9]+ new; checked [0-9]+:`
- Existing assertions that grep this line still pass — check with
  `grep -n 'memory:' test_lane.sh` before and after

## Test plan

No new assertions. Every change here is either a no-op (steps 1, 2), affects
only a fresh `lane init` (step 3), or changes one human-readable line
(step 4). The gate is that the existing suite is unchanged:

- `./test_lane.sh` → same count, `failed: 0`
- the `resolve_anchor` check in step 1, which is the only change that could
  alter behaviour, has its own inline verification

If any test greps the audit summary line, update that assertion in the same
commit as step 4 and say so in the message.

## Done criteria

ALL must hold:

- [ ] `./test_lane.sh` passes with the same assertion count as before this plan
- [ ] `grep -c 'kw not in pat' lanelib/memory.py` → `0`
- [ ] `grep -c 'ATTIC' lane` → `0`
- [ ] `grep -c 'lanes-\*' lane` → `0`
- [ ] The step 1 verification script prints `ok`
- [ ] `./lane --help` exits 0
- [ ] `git status --short` lists only `lane`, `lanelib/memory.py`, and possibly
      `test_lane.sh`
- [ ] `plans/README.md` status row for 010 updated

## STOP conditions

Stop and report back (do not improvise) if:

- Removing the dead branch changes any anchor resolution. It must not. If the
  step 1 script or any suite assertion changes result, restore the lines and
  report — that would mean the branch had a side effect nobody expected.
- An existing assertion greps the audit summary line in a way step 4 breaks
  and the fix is not obvious. Report the assertion rather than reshaping the
  output further.
- Plan 008 has already removed `lanelib/memory.py`'s unused `argparse` / `sys`
  imports and you find nothing left to do there. That is expected — those are
  008's, not this plan's. Do not duplicate the change.

## Maintenance notes

- Item 1 is worth remembering as a review habit: a bare `pass` inside an `if`
  is either dead code or an unfinished thought, and in both cases it misleads.
  Prefer deleting it and, where the intent was real, writing the comment that
  explains where the logic actually lives.
- Item 4's decision — report the pre-review counts and label them — should hold
  even if the output grows. If someone later wants post-review totals, add a
  second line rather than changing what the first one means, because `--json`'s
  `checked` key shares the number.
