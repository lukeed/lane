# Plan 005: Make `lane note` reject paths it cannot serve, and `lane why` name the file it is showing

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

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: plans/001-portable-test-suites.md, plans/002-fail-with-a-message.md
- **Category**: bug
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

`lane note` accepts any string as a path. It does not check that the file
exists, and it does not check that the path is inside the repo. Two concrete
failures follow.

**A typo silently throws the note away.** `lane note -p src/ath.rs ...`
succeeds and prints `noted -> src/ath.rs#@file`. At the next audit the file
does not exist, so the note is promoted with `status: anchor-missing` and
evicted to the attic in the same run. The user recorded a finding and the tool
discarded it, with the only signal buried in an `evict` line.

**A path outside the repo escapes `.context/`.** Verified: `-p ../outside.txt`
resolves to a relative path of `../outside.txt`, and `note_dir` builds
`root/.context/../outside.txt`, so the note file was written to
`<repo>/outside.txt/01M0...md` — outside the memory store, where `lane why`
will never look and `lane done` will never commit it. A deeper `../../` writes
outside the repository entirely.

Separately, `lane why` with no path argument prints the string `None` where
the filename belongs:

```
$ lane why

None#fn verify
    n1
      01M0C2APXK · main · 2026-08-19
```

`lane why` with no argument is a reasonable thing to want — show me everything
this repo knows — and the output is currently unusable for it, because each
note is labelled with the file it is not about.

## Current state

Files:

- `lane` — `cmd_note` at `lane:111-121`, `cmd_why` at `lane:297-322`
- `lanelib/memory.py` — `note_dir` at `lanelib/memory.py:327-328`,
  `load_notes` at `lanelib/memory.py:331-344`

`lane:111-121`:

```python
def cmd_note(a):
    root = repo_root()
    rel = os.path.relpath(os.path.abspath(a.path), str(root))
    pending = root / PENDING
    pending.parent.mkdir(parents=True, exist_ok=True)
    rec = {"text": a.text, "path": rel, "anchor": a.anchor or "@file",
           "branch": current_branch(), "at": now_iso()}
    with pending.open("a", encoding="utf-8") as f:
        f.write(json.dumps(rec) + "\n")
    print("noted -> %s#%s" % (rel, rec["anchor"]))
    return 0
```

`lane:297-311`, the head of `cmd_why` — `rel` is `None` when no path is given,
and is then interpolated into the header line at `lane:311`:

```python
def cmd_why(a):
    root = repo_root()
    rel = os.path.relpath(os.path.abspath(a.path), str(root)) if a.path else None
    notes = load_notes(root, rel)
    if a.anchor:
        notes = [n for n in notes if n.anchor == a.anchor]
    if not notes:
        print("no context for %s" % (rel or "repo"))
        return 0
    groups = {}
    for n in notes:
        groups.setdefault(n.anchor, []).append(n)
    shown = []
    for anchor in sorted(groups):
        print("\n%s#%s" % (rel, anchor))
```

Note that `load_notes(root, None)` correctly returns every note — the filter is
skipped when `path_filter` is falsy (`lanelib/memory.py:341-342`). Only the
display is wrong. Grouping by anchor alone is also wrong for the no-path case:
two different files can both have an `@file` anchor and would be merged under
one header.

`lanelib/memory.py:327-328`:

```python
def note_dir(root: Path, path: str) -> Path:
    return root / CONTEXT_DIR / path
```

Repo conventions to match:

- `%`-style formatting; **no f-strings anywhere in this repo**
- Command functions print `error: ...` to stderr and return a non-zero int —
  see `lane:348-350`
- After plan 002, an expected failure raises `LaneError` from
  `lanelib.memory`, which `main()` renders as a single `error:` line. Either
  style is available; use `LaneError` for the validation in step 1 so the
  message is uniform with the rest of the CLI.
- Comments justify decisions, not mechanics

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
- `lane` — `cmd_note` and `cmd_why`
- `lanelib/memory.py` — one containment helper
- `test_lane.sh` — new section

**Out of scope** (do NOT touch, even though they look related):
- `promote_pending` (`lanelib/memory.py:405`). It must keep tolerating a note
  whose file has since been deleted — that is a legitimate state, not the bug
  here. Validation belongs at write time, where the user is present to be told.
- The attic and eviction machinery. Notes whose anchors genuinely vanish
  should still be evicted.
- `ctx` — superseded script, plan 008.
- Adding a `--force` escape to `lane note`. If the executor feels one is
  needed, that is a design question for the maintainer, not a step here.

## Git workflow

- Branch: `advisor/005-validate-note-paths`
- Commit per step; lowercase imperative subject with a body explaining why.
- Do NOT push or open a PR.

## Steps

### Step 1: Reject paths that are outside the repo or do not exist

Add a helper to `lanelib/memory.py`, directly below `note_dir`
(`lanelib/memory.py:328`):

```python
def rel_to_repo(root: Path, path: str) -> str:
    """Repo-relative path, or raise. Every note directory is built by joining
    this onto .context/, so a `..` here writes memory outside the store —
    somewhere `lane why` will never look and `lane done` will never commit."""
    target = Path(os.path.abspath(path))
    try:
        rel = target.relative_to(Path(os.path.abspath(str(root))))
    except ValueError:
        raise LaneError("%s is outside the repository" % path)
    return str(rel)
```

`LaneError` comes from plan 002 and lives in this same module. If plan 002 has
not landed, STOP — this plan depends on it.

Then rewrite `cmd_note` (`lane:111-121`):

```python
def cmd_note(a):
    root = repo_root()
    rel = rel_to_repo(root, a.path)
    if not (root / rel).exists():
        # A note about a file that is not there is promoted, found missing,
        # and atticked in the same audit. Say so now, while the typo is still
        # on screen.
        raise LaneError("%s does not exist; note not recorded" % rel)
    pending = root / PENDING
    pending.parent.mkdir(parents=True, exist_ok=True)
    rec = {"text": a.text, "path": rel, "anchor": a.anchor or "@file",
           "branch": current_branch(), "at": now_iso()}
    with pending.open("a", encoding="utf-8") as f:
        f.write(json.dumps(rec) + "\n")
    print("noted -> %s#%s" % (rel, rec["anchor"]))
    return 0
```

Import `rel_to_repo` and `LaneError` in `lane`'s
`from lanelib.memory import (...)` block, keeping the list alphabetised.

**Verify**: in a scratch repo,
- `./lane note -p ../outside.txt -a "@file" x` → exit 1, prints
  `error: ../outside.txt is outside the repository`
- `./lane note -p src/nope.rs -a "@file" x` → exit 1, prints
  `error: src/nope.rs does not exist; note not recorded`
- `./lane note -p src/auth.rs -a "fn verify" x` → exit 0, prints `noted -> ...`

### Step 2: Warn when the anchor does not resolve, without refusing the note

An anchor that does not resolve is not the same mistake as a missing file. The
file may be about to gain the function the note describes, and `@file` is
always valid. Warn and record:

```python
    text = (root / rel).read_text(encoding="utf-8", errors="replace")
    if resolve_anchor(text, a.anchor or "@file") is None:
        print("warning: anchor %s not found in %s; recording it anyway"
              % (a.anchor, rel), file=sys.stderr)
```

Place this after the existence check and before the pending write. Import
`resolve_anchor` — it is already imported in `lane` at `lane:26`.

**Verify**: `./lane note -p src/auth.rs -a "fn nonexistent" x` → exit 0, prints
a `warning:` line on stderr and a `noted ->` line on stdout.

### Step 3: Label every note with the file it is about

Group by `(path, anchor)` rather than by anchor alone, so the header is always
correct and the no-path case works. Replace `lane:306-311`:

```python
    groups = {}
    for n in notes:
        groups.setdefault(n.anchor, []).append(n)
    shown = []
    for anchor in sorted(groups):
        print("\n%s#%s" % (rel, anchor))
```

with:

```python
    groups = {}
    for n in notes:
        groups.setdefault((n.path, n.anchor), []).append(n)
    shown = []
    for path, anchor in sorted(groups):
        print("\n%s#%s" % (path, anchor))
```

and change the loop body's `groups[anchor]` to `groups[(path, anchor)]`.

The note's own `path` is authoritative and is set at promotion time, so this
is correct whether or not the user passed an argument.

**Verify**: `./lane why` with no argument prints headers naming real files and
`grep -c 'None#'` of its output → `0`.

### Step 4: Confirm the suites still pass

**Verify**:
- `./test_lane.sh` → `failed: 0` at the baseline count (unchanged; step 5 adds more)
- `./test_ctx.sh` → `passed: 14   failed: 0`

## Test plan

Add a section to `test_lane.sh` before the final summary, modelled on section
11. Five assertions:

```bash
echo "== 15. note validates its path, why names its file =="
setup
"$LANE" note -p ../escape.txt -a "@file" "should not land" > /tmp/esc.out 2>&1
is "note outside the repo is refused" "$?" "1"
is "nothing was written outside .context" \
   "$(find "$TMP" -maxdepth 2 -name 'escape.txt' -type d | wc -l | tr -d ' ')" "0"
"$LANE" note -p src/typo.rs -a "@file" "should not land" > /tmp/typo.out 2>&1
is "note on a missing file is refused" "$?" "1"
"$LANE" note -p src/auth.rs -a "fn verify" "real note" > /dev/null
"$LANE" audit > /dev/null
is "why with no path names the file" \
   "$("$LANE" why | grep -c '^src/auth.rs#fn verify')" "1"
is "why with no path prints no None header" \
   "$("$LANE" why | grep -c 'None#')" "0"
```

Structural pattern: section 11 of `test_lane.sh`.

Confirm the two refusal assertions fail against the current code before
changing it — today both `lane note` calls exit 0.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 5

## Done criteria

ALL must hold:

- [ ] `./test_lane.sh` reports `failed: 0` with baseline + 5 assertions
- [ ] `./test_ctx.sh` prints `passed: 14   failed: 0`
- [ ] `grep -c 'def rel_to_repo' lanelib/memory.py` → `1`
- [ ] `grep -c 'os.path.relpath' lane` → `1` (only `cmd_why`'s remains, and
      only for the filter argument)
- [ ] `./lane why` in a repo with notes prints no line containing `None#`
- [ ] `./lane note -p ../x -a "@file" y` exits 1 with an `error:` line and no
      `Traceback`
- [ ] `git status --short` lists only `lane`, `lanelib/memory.py`, `test_lane.sh`
- [ ] `plans/README.md` status row for 005 updated

## STOP conditions

Stop and report back (do not improvise) if:

- Plan 002 has not landed, so `LaneError` does not exist in
  `lanelib/memory.py`. This plan raises it in two places and depends on
  `main()` rendering it.
- `Path.relative_to` rejects a path that is legitimately inside the repo —
  most likely via a symlinked repo root, where `os.path.abspath` does not
  resolve symlinks but `repo_root()` returns git's resolved answer. If you hit
  this, report it; the fix is `os.path.realpath` on both sides, but confirm
  before changing behaviour for everyone.
- Grouping by `(path, anchor)` breaks the read-count bump at `lane:321`. It
  should not — `shown` is a flat list of ids — but check the assertion in
  `test_ctx.sh` section 8, which depends on `why` bumping reads.
- Refusing a missing file breaks an existing suite assertion. Section 7 of
  `test_lane.sh` deliberately notes an anchor that is about to disappear; it
  notes an existing *file*, so it should be unaffected. If it is not, report
  before relaxing the check.

## Maintenance notes

- `rel_to_repo` is the single choke point for turning user input into a
  `.context/` path. Any future command that accepts a path — a `lane forget`,
  a `lane pin` — should go through it rather than repeat `os.path.relpath`.
- The asymmetry is deliberate and worth preserving in review: a **missing
  file** is refused, a **missing anchor** is warned about. The file is a fact;
  the anchor is a guess that the next audit will resolve.
- Deferred out of this plan: `lane path <name>` still prints a path for a lane
  that does not exist, and the `cd` branch of the shell function will then cd
  into nothing. Plan 006 covers the shell integration.
