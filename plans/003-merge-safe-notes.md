# Plan 003: Stop rewriting unchanged notes, and make merged frontmatter unambiguous

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c2f4ed4..HEAD -- lane lanelib/memory.py test_lane.sh`
> If any of those changed since this plan was written, compare the "Current
> state" excerpts against the live code before proceeding; on a mismatch,
> treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/001-portable-test-suites.md
- **Category**: bug
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

`lane`'s central claim is that parallel lanes can write memory without
coordinating: one file per note, ULID-named, `merge=union` in
`.gitattributes`, nothing to lock. That holds for *creating* notes. It does
not hold for *maintaining* them.

Every `lane audit` stamps `checked: <now>` into every note and rewrites the
file, whether or not anything about the note actually changed. So any two
branches that both audit produce a competing one-line change to the same line
of the same file. Union merge does what union merge does — it keeps both:

```
---
id: 01M0C29WQMYMQWTV6Y9YD1H0MT
...
status: fresh
checked: 2026-08-19T20:00:00Z
checked: 2026-08-19T10:00:00Z
---
```

Verified by merging two branches that had each run an audit. `parse_note`
(`lanelib/memory.py:314-324`) builds its dict by assignment, so the last line
wins — merge order, not recency.

For `checked:` that is cosmetic. The same mechanism applies to `sig:`,
`body_hash:` and `status:`, which are the fingerprint that drives the whole
staleness model. A stale `sig` winning a merge means a note that genuinely
drifted reports `fresh` and is never reviewed. That is the failure mode this
tool exists to prevent.

`lane done` currently dodges it, because it rebases *then* audits, so a
lane's memory commit is always built on top of trunk's. Nothing else dodges
it. `USAGE.md` tells users to run `git pull --rebase` on trunk when it has
diverged, and any real merge of memory from a remote or a colleague hits this
directly.

Two changes fix it, and both are needed:

1. **A no-op audit should produce no diff.** If nothing about a note changed,
   do not rewrite it. Then two branches auditing a stable store touch nothing
   and there is nothing to merge.
2. **When a duplicate key does arrive, resolve it deterministically and
   safely.** Never let a merge decide which fingerprint is true; re-derive it
   from the file the note points at, which is the ground truth anyway.

## Current state

Files:

- `lane` — `run_audit` at `lane:194`, the only place notes are re-stamped
- `lanelib/memory.py` — `Note` and `render()` at `lanelib/memory.py:274-308`,
  `parse_note` at `lanelib/memory.py:314`, `check_note` at
  `lanelib/memory.py:384`

`lane:204-217`, the loop that causes the churn:

```python
    for n in notes:
        res = check_note(root, n)
        tier = res["tier"]
        stats[tier] += 1
        n.meta["status"] = tier
        n.meta["checked"] = now_iso()
        if tier in (TIER_BODY, TIER_SIG):
            n.meta["sig"] = res.get("sig", n.meta.get("sig", ""))
            n.meta["body_hash"] = res.get("body_hash", n.meta.get("body_hash", ""))
            if res.get("span"):
                n.meta["lines"] = "%d-%d" % res["span"]
            review.append(n)
        if n.file:
            n.file.write_text(n.render(), encoding="utf-8")
```

`lanelib/memory.py:314-324`, the parser that lets the last duplicate win:

```python
def parse_note(p: Path) -> Note:
    raw = p.read_text(encoding="utf-8")
    m = FM_RX.match(raw)
    if not m:
        return Note({"id": p.stem}, raw, p)
    meta = {}
    for line in m.group(1).splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            meta[k.strip()] = v.strip()
    return Note(meta, m.group(2), p)
```

`lanelib/memory.py:296-308`, `render()` — note it emits a fixed whitelist of
keys in a fixed order, so a re-render of a parsed note is normalised. That is
what makes "compare rendered output to decide whether to write" reliable, and
it means writing a merged note also heals its duplicates.

```python
    def render(self) -> str:
        keys = ["id", "path", "anchor", "created", "branch", "sig", "body_hash",
                "lines", "status", "checked", "reviewed", "verdict",
                "supersedes", "pinned", "evicted"]
        lines = ["---"]
        for k in keys:
            if k in self.meta and self.meta[k] not in (None, ""):
                lines.append("%s: %s" % (k, self.meta[k]))
        lines.append("---")
        lines.append("")
        lines.append(self.body.strip())
        lines.append("")
        return "\n".join(lines)
```

`lanelib/memory.py:384-397`, `check_note` — the ground truth. Given a note it
resolves the anchor in the real file and recomputes both hashes. An empty
stored `sig` therefore yields `TIER_SIG`, i.e. "look at this", never "fresh":

```python
def check_note(root: Path, note: Note) -> dict:
    target = root / note.path
    if not target.exists():
        return {"tier": TIER_MISSING, "reason": "file gone"}
    text = target.read_text(encoding="utf-8", errors="replace")
    span = resolve_anchor(text, note.anchor)
    if span is None:
        return {"tier": TIER_MISSING, "reason": "anchor not found"}
    sig, body = span_hashes(text, span)
    if sig != note.meta.get("sig", ""):
        return {"tier": TIER_SIG, "sig": sig, "body_hash": body, "span": span}
    if body != note.meta.get("body_hash", ""):
        return {"tier": TIER_BODY, "sig": sig, "body_hash": body, "span": span}
    return {"tier": TIER_FRESH, "sig": sig, "body_hash": body, "span": span}
```

**Already checked, do not re-investigate**: nothing in the codebase *reads*
`meta["checked"]`. `grep -rn '"checked"' lane lanelib/` returns only writes
(`lane:174`, `lane:209`, `lanelib/memory.py:435`) plus the render whitelist and
an unrelated JSON key at `lane:265`. It is a write-only informational field,
which is why redefining it in step 1 is safe.

`TIER_RANK` (`lanelib/memory.py:42`) already gives a total order over tiers,
worst-last:

```python
TIER_RANK = {TIER_FRESH: 0, TIER_BODY: 1, TIER_SIG: 2, TIER_MISSING: 3}
```

Repo conventions to match:

- `%`-style formatting; **no f-strings anywhere in this repo**
- Comments justify decisions. `lanelib/memory.py:447-449` and `lane:136-141`
  are the house voice — copy that register
- Module-level regexes are UPPER_SNAKE and defined above their user
- Functions are small and return plain dicts/tuples, not custom types

## Commands you will need

| Purpose      | Command          | Expected on success       |
|--------------|------------------|---------------------------|
| Lane suite   | `./test_lane.sh` | `failed: 0`, baseline + 5 |
| Ctx suite    | `./test_ctx.sh`  | `passed: 14   failed: 0`  |
| Syntax check | `python3 -c "import ast,io; [ast.parse(io.open(f).read()) for f in ['lane','lanelib/memory.py']]"` | exit 0 |

(`./test_ctx.sh` is deleted by plan 008. If that has already landed, skip its row.)


**Record the baseline first.** Run `./test_lane.sh` before you change
anything and write down the number it prints. Plans in this directory land in
whatever order the maintainer chooses, so the only stable expectation is a
*delta*: this plan must leave the suite passing with **5 more assertions**
than that baseline. Any absolute total below is illustrative.

## Scope

**In scope**:
- `lane` — `run_audit` only
- `lanelib/memory.py` — `parse_note`, and a new duplicate-key helper
- `test_lane.sh` — new section

**Out of scope** (do NOT touch, even though they look related):
- `.gitattributes` generation in `cmd_init` (`lane:45-54`). `merge=union` is
  the deliberate design and this plan makes it correct rather than replacing
  it. Do not introduce a custom merge driver — it would need installing on
  every clone, which is exactly the coordination the design refuses.
- `promote_pending` (`lanelib/memory.py:405`). New notes are written once with
  a fresh fingerprint; they are not part of the churn.
- `evict` (`lanelib/memory.py:447`) and `apply_review` (`lane:135`). Both write
  notes, but only when something genuinely happened.
- `ctx` — superseded script, plan 008.
- The ordering of `lane done` (rebase, then audit). It is correct and is the
  reason this bug is survivable today.

## Git workflow

- Branch: `advisor/003-merge-safe-notes`
- Commit per step; lowercase imperative subject with a body explaining why.
  Example from this repo: `lane rm: stop discarding unlanded commits`
- Do NOT push or open a PR.

## Steps

### Step 1: Only write a note when its content actually changed

Redefine `checked:` from "when this note was last audited" to "when this note
last changed". Nothing reads it, so the meaning is free; what it buys is that
a stable store produces a byte-identical render on every audit.

Give `Note` the raw text it was parsed from so the comparison is against what
is really on disk. In `lanelib/memory.py`, extend `Note.__init__`
(`lanelib/memory.py:275`) with a `raw` parameter defaulting to `None`:

```python
    def __init__(self, meta: dict, body: str, file: Path = None, raw: str = None):
        self.meta = meta
        self.body = body
        self.file = file
        self.raw = raw      # bytes as parsed, so audit can skip a no-op write
```

Pass it from `parse_note`'s two return sites: `Note({"id": p.stem}, raw, p, raw)`
and `Note(meta, m.group(2), p, raw)`.

Then replace `lane:204-217` with:

```python
    for n in notes:
        res = check_note(root, n)
        tier = res["tier"]
        stats[tier] += 1
        before = n.render()
        n.meta["status"] = tier
        if tier in (TIER_BODY, TIER_SIG):
            n.meta["sig"] = res.get("sig", n.meta.get("sig", ""))
            n.meta["body_hash"] = res.get("body_hash", n.meta.get("body_hash", ""))
            if res.get("span"):
                n.meta["lines"] = "%d-%d" % res["span"]
            review.append(n)
        # An audit that learned nothing must leave no trace. Stamping every
        # note on every run gave two branches a competing edit to the same
        # line, and union merge keeps both.
        if n.render() != before:
            n.meta["checked"] = now_iso()
        if n.file and n.render() != n.raw:
            n.file.write_text(n.render(), encoding="utf-8")
```

The second condition also rewrites a note whose on-disk text is not what
`render()` produces — which is exactly a note carrying duplicate keys from a
past merge. Writing it heals the file.

**Verify**: in a scratch repo with at least one note, run `lane audit` twice
and confirm `git status --porcelain -- .context` is empty after the second
run. Step 5's test automates this.

### Step 2: Collect duplicate frontmatter keys instead of overwriting them

In `lanelib/memory.py`, change `parse_note` to gather every value per key:

```python
def parse_note(p: Path) -> Note:
    raw = p.read_text(encoding="utf-8")
    m = FM_RX.match(raw)
    if not m:
        return Note({"id": p.stem}, raw, p, raw)
    seen = {}
    for line in m.group(1).splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            seen.setdefault(k.strip(), []).append(v.strip())
    return Note(resolve_dupes(seen), m.group(2), p, raw)
```

**Verify**: `python3 -c "import ast,io; ast.parse(io.open('lanelib/memory.py').read())"` → exit 0

### Step 3: Resolve duplicates deterministically, and never in favour of "fresh"

Add `resolve_dupes` directly above `parse_note` in `lanelib/memory.py`:

```python
# Union merge concatenates both sides of a changed line, so a note that two
# branches audited arrives with repeated keys. Which line came first is an
# artifact of merge order and carries no meaning, so nothing here may depend
# on it: every rule below is order-independent, and every ambiguous
# fingerprint resolves toward re-checking rather than toward "fresh".
def resolve_dupes(seen: dict) -> dict:
    meta = {}
    for k, vals in seen.items():
        uniq = list(dict.fromkeys(vals))
        if len(uniq) == 1:
            meta[k] = uniq[0]
        elif k in ("sig", "body_hash", "lines"):
            # A fingerprint we cannot arbitrate is a fingerprint we do not
            # have. Dropping it makes check_note re-derive it from the file,
            # which is the only real source of truth.
            meta[k] = ""
        elif k == "status":
            meta[k] = max(uniq, key=lambda t: TIER_RANK.get(t, 0))
        else:
            meta[k] = max(uniq)
    return meta
```

`max(uniq)` for the remaining keys is a string maximum: right for the ISO
timestamps in `checked`, `created` and `reviewed`, and harmless for the rest,
which never legitimately differ between branches (`id`, `path`, `anchor`).

An empty `sig` makes `check_note` return `TIER_SIG`, so the note is flagged
for review and the next audit writes a real fingerprint back. The cost of a
merge collision is one review; the cost of the current behaviour is a silently
wrong `fresh`.

**Verify**:

```
python3 - <<'PY'
from lanelib.memory import resolve_dupes
a = {"sig": ["aaa", "bbb"], "status": ["fresh", "body-drift"], "checked": ["2026-01-02T00:00:00Z", "2026-01-01T00:00:00Z"]}
b = {"sig": ["bbb", "aaa"], "status": ["body-drift", "fresh"], "checked": ["2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"]}
assert resolve_dupes(a) == resolve_dupes(b), "must not depend on merge order"
assert resolve_dupes(a)["sig"] == ""
assert resolve_dupes(a)["status"] == "body-drift"
assert resolve_dupes(a)["checked"] == "2026-01-02T00:00:00Z"
print("ok")
PY
```
→ prints `ok`

### Step 4: Confirm the existing suites still pass

Both suites exercise audit heavily. A drop here means step 1's write condition
is wrong.

**Verify**:
- `./test_lane.sh` → `failed: 0` at the baseline count (unchanged; step 5 adds more)
- `./test_ctx.sh` → `passed: 14   failed: 0`

Note `test_ctx.sh` drives the standalone `ctx` script, which has its own copy
of this logic and is **out of scope**. It should be unaffected. If it changes,
you edited the wrong file.

### Step 5: Test it end to end

Add a new section to `test_lane.sh` before the final summary block, modelled
on section 11 (the newest, same helpers, same shape). Five assertions:

```bash
echo "== 13. audit is idempotent and memory survives a real merge =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
"$LANE" audit > /dev/null
is "a no-op audit writes nothing" \
   "$(git status --porcelain -- .context | wc -l | tr -d ' ')" "0"

git checkout -qb branch-a
"$LANE" note -p src/auth.rs -a "fn verify" "a: alpha" > /dev/null
"$LANE" audit > /dev/null && git add -A && git commit -qm a
git checkout -q main && git checkout -qb branch-b
"$LANE" note -p src/auth.rs -a "fn verify" "b: beta" > /dev/null
"$LANE" audit > /dev/null && git add -A && git commit -qm b
git merge -q --no-edit branch-a > /tmp/m.out 2>&1
is "parallel memory merges without conflict" "$?" "0"
is "both notes survived" \
   "$(grep -rl 'a: alpha\|b: beta' .context --include='*.md' | wc -l | tr -d ' ')" "2"
is "no note has a duplicated key" \
   "$(for f in $(find .context -name '*.md' -not -path '*/.attic/*'); do
        awk '/^---$/{n++; next} n==1 {sub(/:.*/,""); print}' "$f" | sort | uniq -d
      done | wc -l | tr -d ' ')" "0"
"$LANE" check --json > /tmp/k.json
is "no note reports a fingerprint it cannot justify" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/k.json"));print(sum(1 for x in d if x["tier"]=="anchor-missing"))')" "0"
```

The duplicate-key assertion is the regression gate: it reads the frontmatter
of every live note and reports any key appearing twice.

To confirm the test actually catches the bug, temporarily revert step 1's
write condition to the unconditional `n.file.write_text(...)`, re-run, and
check that this section fails. Then restore step 1.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 5

## Test plan

Summarised from step 5 — five new assertions in `test_lane.sh` section 13:

1. a second consecutive `lane audit` leaves `.context` with no git diff
2. two branches that each added a note merge without conflict
3. both notes survive the merge
4. no live note carries a duplicated frontmatter key
5. the merged store reports no bogus tiers

Structural pattern: section 11 of `test_lane.sh`. Use `is` for every
assertion; do not add a new helper.

The negative control described in step 5 (revert, watch it fail, restore) is
required, not optional. A regression test that passes against the old code is
not a regression test.

## Done criteria

ALL must hold:

- [ ] `./test_lane.sh` reports `failed: 0` with baseline + 5 assertions
- [ ] `./test_ctx.sh` prints `passed: 14   failed: 0`
- [ ] `grep -c 'def resolve_dupes' lanelib/memory.py` → `1`
- [ ] `grep -c 'n.meta\["checked"\] = now_iso()' lane` → `1`, and it is inside
      the `if n.render() != before:` branch
- [ ] The step 3 verification script prints `ok`
- [ ] Running `lane audit` twice in a repo with notes leaves
      `git status --porcelain -- .context` empty
- [ ] `git status --short` lists only `lane`, `lanelib/memory.py`,
      `test_lane.sh`
- [ ] `plans/README.md` status row for 003 updated

## STOP conditions

Stop and report back (do not improvise) if:

- Making writes conditional breaks an existing assertion in `test_lane.sh`
  sections 1–12. Those cover promotion, review verdicts and eviction, all of
  which write notes for real reasons and must keep working.
- You find code that *reads* `meta["checked"]` and depends on it meaning "last
  audited". The plan assumes it is write-only; that was verified at
  `c2f4ed4`, but verify again with
  `grep -rn 'checked' lane lanelib/` before step 1.
- The duplicate-key assertion in step 5 passes *before* you make any change.
  That means the test is not reproducing the bug — report it rather than
  proceeding, because the rest of the plan then has no gate.
- Resolving duplicates makes notes churn on every audit (each run flagging
  everything as `signature-changed`). That would mean `render()` and the
  on-disk text disagree for a reason other than merge damage. Report the
  diff rather than loosening the comparison.

## Maintenance notes

- The invariant to protect in review: **an audit that learned nothing writes
  nothing.** Any future field that gets stamped unconditionally — a counter, a
  "last seen" timestamp, a schema version — reintroduces this bug. If a field
  must change every run, it does not belong in the note file; put it in the
  ledger alongside `.reads.jsonl`.
- `resolve_dupes` is the only place that knows union merge exists. If notes
  ever move to a format with real escaping, that function is where the
  ambiguity rules live and should move with it.
- Deferred out of this plan: `.reads.jsonl` has the same class of problem —
  unbounded growth and merge-order-dependent counts. Plan 009 covers it.
- Deferred out of this plan: `reason` strings from the model reviewer are
  written into `verdict:` unescaped (`lane:160-162`), so a newline in a model
  response corrupts the frontmatter directly, without any merge involved.
  That is a real hole in the same file format; it is small, but it needs its
  own change and its own test. Raise it if you want it folded in.
