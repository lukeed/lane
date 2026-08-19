# Plan 004: Stop hiding real changes behind a language-blind comment stripper

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat c2f4ed4..HEAD -- lanelib/memory.py test_lane.sh`
> If either changed since this plan was written, compare the "Current state"
> excerpts against the live code before proceeding; on a mismatch, treat it as
> a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: plans/001-portable-test-suites.md, plans/003-merge-safe-notes.md
- **Category**: bug
- **Planned at**: commit `c2f4ed4`, 2026-08-18

## Why this matters

Staleness detection is the product. A note is `fresh` when the normalized text
of its anchor's span hashes to the same value as last time. `normalize()`
strips comments before hashing so that a formatter run or a reworded comment
does not read as drift — a good idea, implemented with one regex applied to
every file in every language:

```python
COMMENT_RX = re.compile(r"(//[^\n]*|#[^\n]*|--[^\n]*)")
```

That regex does not know what language it is looking at, and it does not know
what a string literal is. Both gaps produce **false negatives** — real changes
that report as `fresh`:

```python
>>> normalize('let url = "https://api.example.com/v1/charge";')
'let url = "https:'
>>> normalize('let url = "https://api.example.com/v1/refund";')
'let url = "https:'
```

Changing a payment endpoint from charge to refund is invisible to the drift
checker. Anything after `//` in a string is.

The `#` rule is worse in markdown, where `#` is not a comment at all. Every
line of a heading normalizes to nothing, so:

- `sig` for every `## Heading` anchor is `sha("")` — verified, it is
  `e3b0c44298fc1c14` for all of them — which makes the `signature-changed`
  tier unreachable for markdown;
- restructuring the `###` sub-headings inside a documented section is not
  drift, because every one of those lines was erased before hashing.

A false positive in this system costs one review. A false negative costs the
thing the tool promises to prevent: a note that is quietly wrong. The fix is
to strip only what is certainly a comment in the file at hand, and to keep
everything else.

## Current state

Files:

- `lanelib/memory.py` — `COMMENT_RX`/`BLOCK_COMMENT_RX` at
  `lanelib/memory.py:236-237`, `normalize` at `lanelib/memory.py:240-249`,
  `span_hashes` at `lanelib/memory.py:256-266`, and its two callers
  `check_note` (`lanelib/memory.py:392`) and `promote_pending`
  (`lanelib/memory.py:427`).

`lanelib/memory.py:236-249`:

```python
COMMENT_RX = re.compile(r"(//[^\n]*|#[^\n]*|--[^\n]*)")
BLOCK_COMMENT_RX = re.compile(r"/\*.*?\*/", re.S)


def normalize(span: str) -> str:
    """Lossy on purpose: comment and whitespace churn must not read as drift."""
    s = BLOCK_COMMENT_RX.sub("", span)
    out = []
    for line in s.splitlines():
        line = COMMENT_RX.sub("", line)
        line = re.sub(r"\s+", " ", line).strip()
        if line:
            out.append(line)
    return "\n".join(out)
```

`lanelib/memory.py:256-266`:

```python
def span_hashes(text: str, span):
    """(signature_hash, body_hash). Splitting these is what buys the
    signature-changed vs body-drift distinction for ~six lines of work."""
    lines = text.splitlines()
    start, end = span
    chunk = lines[start - 1 : end]
    if not chunk:
        return ("", "")
    sig = normalize(chunk[0])
    body = normalize("\n".join(chunk[1:]))
    return (sha(sig), sha(body))
```

Both call sites already have the note's path and anchor in scope:

- `lanelib/memory.py:388-392` inside `check_note`, which has `note.path` and
  `note.anchor`
- `lanelib/memory.py:424-428` inside `promote_pending`, which has
  `rec["path"]` and `rec["anchor"]`

The anchor grammar (`lanelib/memory.py:141-180`) already distinguishes the
top-level blocks of a single-file component: `#script`, `#style`, `#template`.
That is exactly the information needed to pick a comment syntax inside a
`.svelte` or `.vue` file, so it should be used rather than guessed.

`SFC_BLOCKS` is already defined at `lanelib/memory.py:138`:

```python
SFC_BLOCKS = {"script", "style", "template"}
```

Repo conventions to match:

- `%`-style formatting; **no f-strings anywhere in this repo**
- Module-level regexes and tables are UPPER_SNAKE, defined above their user
- Comments justify the decision, not the mechanics — see
  `lanelib/memory.py:115-120` for the register
- Zero third-party dependencies, Python 3.9+. Do **not** add tree-sitter or
  any parser library. The README already names tree-sitter as the eventual
  replacement for *anchor resolution*, which is a different problem and a
  different plan.

## Commands you will need

| Purpose      | Command          | Expected on success       |
|--------------|------------------|---------------------------|
| Lane suite   | `./test_lane.sh` | `failed: 0`, baseline + 6 |
| Ctx suite    | `./test_ctx.sh`  | `passed: 14   failed: 0`  |
| Syntax check | `python3 -c "import ast,io; ast.parse(io.open('lanelib/memory.py').read())"` | exit 0 |

(`./test_ctx.sh` is deleted by plan 008. If that has already landed, skip its row.)


**Record the baseline first.** Run `./test_lane.sh` before you change
anything and write down the number it prints. Plans in this directory land in
whatever order the maintainer chooses, so the only stable expectation is a
*delta*: this plan must leave the suite passing with **6 more assertions**
than that baseline. Any absolute total below is illustrative.

## Scope

**In scope**:
- `lanelib/memory.py`
- `test_lane.sh`

**Out of scope** (do NOT touch, even though they look related):
- `resolve_anchor` and `_find_end` (`lanelib/memory.py:141-229`). Anchor
  resolution is a separate, documented v0 limitation. This plan changes only
  what happens to a span *after* it has been resolved.
- `ctx` — it carries its own stale copy of `normalize`. Plan 008 decides its
  fate; leaving it unchanged is what keeps `test_ctx.sh` a useful control
  during this work.
- `lanelib/review.py` — the model reviewer sees raw span text, not normalized
  text, and should keep doing so.
- Adding any dependency.

## Git workflow

- Branch: `advisor/004-language-aware-normalization`
- Commit per step; lowercase imperative subject with a body explaining why.
- Do NOT push or open a PR.

## Steps

### Step 1: Add a comment-syntax table keyed by extension

Replace `COMMENT_RX`/`BLOCK_COMMENT_RX` (`lanelib/memory.py:236-237`) with a
per-language table. Put it directly above `normalize`.

```python
# Comment syntax per file type: (line tokens, (block open, block close) pairs).
# Anything not listed normalizes whitespace only. That default is deliberate:
# a comment we fail to strip costs one spurious review, while a string we
# mistake for a comment hides a real change, which is the failure this tool
# exists to prevent. When unsure, keep the text.
SLASH = (("//",), (("/*", "*/"),))
HASH = (("#",), ())
SYNTAX = {
    ".c": SLASH, ".cc": SLASH, ".cpp": SLASH, ".cs": SLASH, ".dart": SLASH,
    ".go": SLASH, ".h": SLASH, ".hpp": SLASH, ".java": SLASH, ".js": SLASH,
    ".jsx": SLASH, ".kt": SLASH, ".mjs": SLASH, ".php": SLASH, ".rs": SLASH,
    ".scala": SLASH, ".swift": SLASH, ".ts": SLASH, ".tsx": SLASH,
    ".zig": SLASH,
    ".bash": HASH, ".nix": HASH, ".pl": HASH, ".py": HASH, ".rb": HASH,
    ".sh": HASH, ".tf": HASH, ".toml": HASH, ".yaml": HASH, ".yml": HASH,
    ".zsh": HASH,
    ".css": ((), (("/*", "*/"),)),
    ".scss": SLASH,
    ".sql": (("--",), (("/*", "*/"),)),
    ".lua": (("--",), (("--[[", "]]"),)),
    ".html": ((), (("<!--", "-->"),)),
    ".xml": ((), (("<!--", "-->"),)),
    ".md": ((), (("<!--", "-->"),)),
}
SFC_EXTS = (".svelte", ".vue")
```

Note `.md` strips only HTML comments. `#` in markdown is a heading, not a
comment, and erasing headings is what makes `signature-changed` unreachable
there today.

**Verify**: `python3 -c "from lanelib.memory import SYNTAX; print(SYNTAX['.md'], SYNTAX['.rs'])"`
→ `((), (('<!--', '-->'),)) (('//',), (('/*', '*/'),))`

### Step 2: Pick the syntax from the path, and from the anchor inside an SFC

Add below the table:

```python
def comment_syntax(path: str, anchor: str = "@file"):
    """A .svelte or .vue file is three languages in one, and the anchor says
    which one we are looking at. Outside those, the extension decides."""
    ext = os.path.splitext(path)[1].lower()
    if ext in SFC_EXTS:
        block = anchor.strip().lstrip("#").strip().lower()
        if block == "script":
            return SLASH
        if block == "style":
            return ((), (("/*", "*/"),))
        return ((), (("<!--", "-->"),))
    return SYNTAX.get(ext, ((), ()))
```

`os` is already imported at `lanelib/memory.py:20`.

**Verify**:

```
python3 - <<'PY'
from lanelib.memory import comment_syntax
assert comment_syntax("a.rs") == (("//",), (("/*", "*/"),))
assert comment_syntax("a.md") == ((), (("<!--", "-->"),))
assert comment_syntax("a.svelte", "#script") == (("//",), (("/*", "*/"),))
assert comment_syntax("a.svelte", "#style") == ((), (("/*", "*/"),))
assert comment_syntax("a.unknownext") == ((), ())
print("ok")
PY
```
→ prints `ok`

### Step 3: Strip comments with a string-aware scanner

A regex cannot tell `// comment` from `"https://..."`. Replace the regex
substitution with a single left-to-right pass that tracks quote state. Add
above `normalize`:

```python
QUOTES = "\"'`"


def strip_comments(text: str, line_tokens, block_pairs) -> str:
    """Remove comments without touching string literals.

    One pass, tracking quote state, because the regex this replaces truncated
    every line containing a `//` inside a URL and called the result unchanged.
    """
    out = []
    i, n = 0, len(text)
    quote = None
    block_close = None
    while i < n:
        ch = text[i]
        if block_close:
            if text.startswith(block_close, i):
                i += len(block_close)
                block_close = None
            else:
                if ch == "\n":
                    out.append(ch)
                i += 1
            continue
        if quote:
            out.append(ch)
            if ch == "\\" and i + 1 < n:
                out.append(text[i + 1])
                i += 2
                continue
            if ch == quote:
                quote = None
            i += 1
            continue
        if ch in QUOTES:
            quote = ch
            out.append(ch)
            i += 1
            continue
        hit = next((o for o, c in block_pairs if text.startswith(o, i)), None)
        if hit is not None:
            block_close = dict(block_pairs)[hit]
            i += len(hit)
            continue
        if any(text.startswith(t, i) for t in line_tokens):
            while i < n and text[i] != "\n":
                i += 1
            continue
        out.append(ch)
        i += 1
    return "".join(out)
```

An unterminated quote or block simply runs to the end of the span; that is
acceptable, because both sides of a comparison are normalized the same way.

**Verify**:

```
python3 - <<'PY'
from lanelib.memory import strip_comments, comment_syntax
rs = comment_syntax("a.rs")
a = strip_comments('let url = "https://x.com/charge"; // note', *rs)
b = strip_comments('let url = "https://x.com/refund"; // note', *rs)
assert a != b, "a URL change must survive normalization"
assert "note" not in a, "a real comment must still be stripped"
assert strip_comments("x = 1 /* c */ + 2", *rs).replace("  ", " ") == "x = 1 + 2 "
print("ok")
PY
```
→ prints `ok`

### Step 4: Thread path and anchor through `normalize` and `span_hashes`

Rewrite `normalize` (`lanelib/memory.py:240-249`) to take the syntax it should
use, and `span_hashes` to take the path and anchor:

```python
def normalize(span: str, path: str = "", anchor: str = "@file") -> str:
    """Lossy on purpose: comment and whitespace churn must not read as drift.

    Lossy only where we are sure, though. The comment syntax comes from the
    file's own type, so a `#` in a markdown heading and a `//` in a URL both
    survive to be hashed.
    """
    line_tokens, block_pairs = comment_syntax(path, anchor)
    s = strip_comments(span, line_tokens, block_pairs)
    out = []
    for line in s.splitlines():
        line = re.sub(r"\s+", " ", line).strip()
        if line:
            out.append(line)
    return "\n".join(out)


def span_hashes(text: str, span, path: str = "", anchor: str = "@file"):
    """(signature_hash, body_hash). Splitting these is what buys the
    signature-changed vs body-drift distinction for ~six lines of work."""
    lines = text.splitlines()
    start, end = span
    chunk = lines[start - 1 : end]
    if not chunk:
        return ("", "")
    sig = normalize(chunk[0], path, anchor)
    body = normalize("\n".join(chunk[1:]), path, anchor)
    return (sha(sig), sha(body))
```

Update both call sites to pass them:

- `lanelib/memory.py:392` in `check_note` →
  `sig, body = span_hashes(text, span, note.path, note.anchor)`
- `lanelib/memory.py:427` in `promote_pending` →
  `sig, body = span_hashes(text, span, rec["path"], rec["anchor"])`

**Verify**:
- `grep -c 'span_hashes(text, span)' lanelib/memory.py` → `0`
- `python3 -c "import ast,io; ast.parse(io.open('lanelib/memory.py').read())"` → exit 0

### Step 5: Re-baseline existing notes instead of flagging the whole store

Changing the hash function changes every hash. Without this step, the first
audit after the upgrade reports every note in the store as
`signature-changed` and — if a reviewer is configured — sends all of them to a
model. That is a bill and a wall of noise for a change no user made.

Add a version marker near the tier constants (`lanelib/memory.py:37-42`):

```python
# Bump when normalize() changes shape. A note fingerprinted by an older
# version is re-baselined silently rather than reported as drift: the hashes
# moved because we changed, not because the code did.
NORM_VERSION = "2"
```

Add `"norm"` to the `render()` key whitelist (`lanelib/memory.py:297-299`),
after `"body_hash"`.

Set it where fingerprints are written — in `promote_pending`'s `Note({...})`
literal (`lanelib/memory.py:431-436`), add `"norm": NORM_VERSION`.

In `check_note` (`lanelib/memory.py:384-397`), return a re-baseline instead of
a drift verdict when the marker is missing or old. Insert immediately after
`sig, body = span_hashes(...)`:

```python
    if note.meta.get("norm", "") != NORM_VERSION:
        # Fingerprinted by an older normalize(); the hashes cannot be
        # compared. Adopt the new ones without calling it drift.
        return {"tier": TIER_FRESH, "sig": sig, "body_hash": body,
                "span": span, "rebaselined": True}
```

Finally, in `lane`'s `run_audit` (`lane:204-217`, as left by plan 003), the
fresh branch must persist the new fingerprint. After `n.meta["status"] = tier`
add:

```python
        if res.get("rebaselined"):
            n.meta["sig"] = res["sig"]
            n.meta["body_hash"] = res["body_hash"]
            n.meta["norm"] = NORM_VERSION
```

Import `NORM_VERSION` in `lane`'s `from lanelib.memory import (...)` block,
keeping the list alphabetised.

**Verify**: in a scratch repo, create a note, run `lane audit`, then
`grep -c 'norm: 2' .context/**/*.md` → at least `1`; and a second
`lane audit` reports `0 signature-changed`.

### Step 6: Test the false negatives that motivated this plan

Add a section to `test_lane.sh` before the final summary, modelled on section
11. Six assertions:

```bash
echo "== 14. normalization strips comments, not code =="
setup
cat > src/net.rs <<'EOF'
pub fn charge(id: &str) -> Result<()> {
    let url = "https://api.example.com/v1/charge";
    post(url, id)  // fire and forget
}
EOF
cat > docs/guide.md <<'EOF'
## Rate limiting

### Buckets
one bucket per key.
EOF
git add -A && git commit -qm net
"$LANE" note -p src/net.rs -a "fn charge" "endpoint is not idempotent" > /dev/null
"$LANE" note -p docs/guide.md -a "## Rate limiting" "buckets are per-key, not per-ip" > /dev/null
"$LANE" audit > /dev/null

sedi 's|// fire and forget|// fire, forget, and move on|' src/net.rs
is "reworded comment is not drift" \
   "$("$LANE" check --json | python3 -c 'import json,sys;d=json.load(sys.stdin);print([x["tier"] for x in d if x["path"]=="src/net.rs"][0])')" \
   "fresh"

sedi 's|/v1/charge|/v1/refund|' src/net.rs
is "url change inside a string IS drift" \
   "$("$LANE" check --json | python3 -c 'import json,sys;d=json.load(sys.stdin);print([x["tier"] for x in d if x["path"]=="src/net.rs"][0])')" \
   "body-drift"

is "markdown signature is a real hash" \
   "$(grep -h '^sig:' .context/docs/guide.md/*.md | grep -c 'e3b0c44298fc1c14')" "0"

sedi 's|### Buckets|### Token buckets|' docs/guide.md
is "markdown sub-heading change IS drift" \
   "$("$LANE" check --json | python3 -c 'import json,sys;d=json.load(sys.stdin);print([x["tier"] for x in d if x["path"]=="docs/guide.md"][0])')" \
   "body-drift"
```

Add two more assertions of your choosing that cover the SFC case: editing
`#script` in a `.svelte` file where the script contains a `//` inside a string,
and confirming a `/* */` change in `#style` is not drift.

`sedi` is the portable in-place edit helper added by plan 001. `docs/` does not
exist in the suite's fixture repo — create it in the same `cat` block, or add
`mkdir -p docs` before writing the file.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 6

## Test plan

Six new assertions in `test_lane.sh` section 14, covering both directions:

- **must stay fresh**: reworded comment (Rust), changed CSS comment in an SFC
  `#style` block
- **must read as drift**: URL changed inside a string literal, markdown
  sub-heading renamed, `#script` body changed in an SFC
- **must not be `sha("")`**: the `sig` of a markdown-anchored note

Structural pattern: section 5 of `test_lane.sh`, which already drives
`lane check --json` through Python to read one note's tier.

The false-negative assertions are the point of this plan. Confirm they fail
against the current code before you change it: stash your work, run just that
section, and check the URL assertion reports `fresh`. Then unstash.

## Done criteria

ALL must hold:

- [ ] `./test_lane.sh` reports `failed: 0` with baseline + 6 assertions
- [ ] `./test_ctx.sh` prints `passed: 14   failed: 0`
- [ ] `grep -c 'COMMENT_RX' lanelib/memory.py` → `0`
- [ ] `grep -c 'def strip_comments' lanelib/memory.py` → `1`
- [ ] `grep -c 'def comment_syntax' lanelib/memory.py` → `1`
- [ ] The step 2 and step 3 verification scripts both print `ok`
- [ ] `python3 -c "import lanelib.memory"` exits 0 with no output
- [ ] `git status --short` lists only `lanelib/memory.py`, `test_lane.sh`
- [ ] `plans/README.md` status row for 004 updated

## STOP conditions

Stop and report back (do not improvise) if:

- `test_ctx.sh` changes result. It drives the standalone `ctx` script, which
  has its own copy of `normalize` and is out of scope. A change there means
  you edited the wrong file.
- `strip_comments` needs to become recursive, or needs to know about nested
  block comments, raw strings, heredocs or template-literal interpolation. It
  does not, for the languages in the table. If a test seems to require it,
  report rather than growing the scanner — the honest answer may be to drop
  that language from `SYNTAX` and let it normalize whitespace only.
- Any existing assertion in sections 1–13 changes result. Section 5's SFC
  granularity test and `test_ctx.sh`'s comment-churn test are the two that
  most directly constrain this change.
- Step 5's re-baseline turns out to mask a real drift in the suite (a note
  that should have been flagged reports `fresh`). Report it: the tradeoff was
  chosen deliberately for the one-time upgrade, but a permanent hole is not
  acceptable.

## Maintenance notes

- The rule for future edits to `SYNTAX`: adding a language is cheap and safe;
  removing one is also safe (it falls back to whitespace-only). What is not
  safe is adding a token that is ambiguous in that language — `#` for a
  language where `#` also starts a directive, for instance. Prefer a false
  positive.
- `NORM_VERSION` must be bumped by any future change to `normalize`,
  `strip_comments` or `SYNTAX` that can alter an existing hash. A reviewer
  should treat a diff to those three without a version bump as a defect.
- Anchor resolution is still regex-based and still the larger source of
  wrongness — a `{` inside a string literal can still end a span early in
  `_find_end` (`lanelib/memory.py:210-229`). The string-aware scanner written
  here is reusable there if someone takes that on before tree-sitter lands.
- Deferred out of this plan: `#template` blocks in SFCs are treated as HTML,
  which is right for the markup but wrong for any expression inside `{...}`.
  Notes anchored to `#template` therefore lean toward false positives, which
  is the safe direction.
