# SPDX-License-Identifier: MIT
"""ctx - code-anchored memory for agent worktrees.

Design notes live in README.md. Two rules matter for reading this file:

  1. Notes are immutable, one file per note, ULID-named. Nothing ever edits a
     note in place, so concurrent branches never conflict textually.
  2. Staleness is computed against the *anchor's span*, not the file. A note
     about `#style` in a 900-line SFC does not go stale because someone
     touched `#script`.

Zero dependencies, Python 3.9+.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

CONTEXT_DIR = ".context"
ATTIC = ".attic"
READS = ".reads.jsonl"
PENDING = ".wt/pending.jsonl"

# Budget per (path, anchor). Ranking + eviction runs at audit time only.
DEFAULT_MAX_NOTES = 5
DEFAULT_MAX_CHARS = 1200

TIER_FRESH = "fresh"
TIER_BODY = "body-drift"
TIER_SIG = "signature-changed"
TIER_MISSING = "anchor-missing"

TIER_RANK = {TIER_FRESH: 0, TIER_BODY: 1, TIER_SIG: 2, TIER_MISSING: 3}


# --------------------------------------------------------------------------
# ids
# --------------------------------------------------------------------------

CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"


def ulid() -> str:
    """Crockford base32 ULID: 48-bit ms timestamp + 80 bits of randomness.

    Lexicographic sort == creation order, with no coordination between
    worktrees. That is the whole reason we use it instead of a counter.
    """
    ms = int(time.time() * 1000)
    rand = int.from_bytes(os.urandom(10), "big")
    n = (ms << 80) | rand
    out = []
    for _ in range(26):
        out.append(CROCKFORD[n & 0x1F])
        n >>= 5
    return "".join(reversed(out))


def slug(text: str, n: int = 28) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    return s[:n] or "note"


def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


# --------------------------------------------------------------------------
# git
# --------------------------------------------------------------------------


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


def repo_root() -> Path:
    return Path(git("rev-parse", "--show-toplevel"))


def current_branch() -> str:
    try:
        return git("rev-parse", "--abbrev-ref", "HEAD")
    except RuntimeError:
        return "unknown"


def touched_paths(base: str) -> set:
    """Paths this branch touched relative to base. Used to bias retention."""
    try:
        out = git("diff", "--name-only", "%s...HEAD" % base, check=False)
    except RuntimeError:
        return set()
    return {p for p in out.splitlines() if p}


# --------------------------------------------------------------------------
# anchor resolution
#
# v0 is deliberately regex-based and language-loose. Everything below this
# comment is the part you replace with tree-sitter later; the rest of the
# tool only consumes (start_line, end_line) and never learns how it got them.
# --------------------------------------------------------------------------

DECL_PATTERNS = [
    # rust / go / c-like
    r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+{name}\b",
    r"^\s*(?:pub\s+)?(?:struct|enum|trait|impl|type|mod)\s+{name}\b",
    r"^\s*func\s+(?:\([^)]*\)\s*)?{name}\b",
    # python
    r"^\s*(?:async\s+)?def\s+{name}\b",
    r"^\s*class\s+{name}\b",
    # ts / js
    r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+{name}\b",
    r"^\s*(?:export\s+)?(?:const|let|var)\s+{name}\b",
    r"^\s*(?:export\s+)?(?:interface|type|class|enum)\s+{name}\b",
    # generic fallback: a method-ish line
    r"^\s*{name}\s*[:=]\s*(?:async\s*)?\(",
]

SFC_BLOCKS = {"script", "style", "template"}


def resolve_anchor(text: str, anchor: str):
    """Return (start_line, end_line) 1-indexed inclusive, or None.

    Anchor grammar, loosest first, because agents write loose references:
      @file        whole file
      #script      SFC / html top-level block
      ## Heading   markdown section
      fn verify    declaration keyword + name
      verify       bare name, tried against every declaration pattern
    """
    lines = text.splitlines()
    if not lines:
        return None
    a = anchor.strip()

    if a in ("@file", "*", ""):
        return (1, len(lines))

    if a.startswith("#") and not a.startswith("##"):
        block = a[1:].strip().lower()
        if block in SFC_BLOCKS:
            return _resolve_sfc_block(lines, block)

    if a.startswith("##") or a.startswith("# "):
        return _resolve_heading(lines, a)

    parts = a.split()
    name = parts[-1]
    kw = parts[0] if len(parts) > 1 else None

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


def _resolve_sfc_block(lines, block):
    open_rx = re.compile(r"^\s*<%s[\s>]" % re.escape(block))
    close_rx = re.compile(r"^\s*</%s\s*>" % re.escape(block))
    start = None
    for i, line in enumerate(lines):
        if start is None and open_rx.search(line):
            start = i
        elif start is not None and close_rx.search(line):
            return (start + 1, i + 1)
    return (start + 1, len(lines)) if start is not None else None


def _resolve_heading(lines, anchor):
    want = anchor.strip()
    level = len(want) - len(want.lstrip("#"))
    for i, line in enumerate(lines):
        if line.strip() == want:
            for j in range(i + 1, len(lines)):
                s = lines[j].lstrip()
                if s.startswith("#"):
                    lvl = len(s) - len(s.lstrip("#"))
                    if lvl <= level:
                        return (i + 1, j)
            return (i + 1, len(lines))
    return None


def _find_end(lines, start_idx):
    """End of a declaration: brace balance, else indentation, else EOF."""
    first = lines[start_idx]
    if "{" in first:
        depth = 0
        for i in range(start_idx, len(lines)):
            depth += lines[i].count("{") - lines[i].count("}")
            if depth <= 0 and i > start_idx or (depth == 0 and "{" in lines[i]):
                return i
        return len(lines) - 1

    base = len(first) - len(first.lstrip())
    for i in range(start_idx + 1, len(lines)):
        s = lines[i]
        if not s.strip():
            continue
        indent = len(s) - len(s.lstrip())
        if indent <= base:
            return i - 1
    return len(lines) - 1


# --------------------------------------------------------------------------
# span hashing
# --------------------------------------------------------------------------

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


def sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()[:16]


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


# --------------------------------------------------------------------------
# note storage
# --------------------------------------------------------------------------


class Note:
    def __init__(self, meta: dict, body: str, file: Path = None):
        self.meta = meta
        self.body = body
        self.file = file

    @property
    def id(self):
        return self.meta.get("id", "")

    @property
    def path(self):
        return self.meta.get("path", "")

    @property
    def anchor(self):
        return self.meta.get("anchor", "@file")

    @property
    def pinned(self):
        return str(self.meta.get("pinned", "")).lower() == "true"

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


FM_RX = re.compile(r"^---\n(.*?)\n---\n?(.*)$", re.S)


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


def note_dir(root: Path, path: str) -> Path:
    return root / CONTEXT_DIR / path


def load_notes(root: Path, path_filter: str = None):
    base = root / CONTEXT_DIR
    if not base.exists():
        return []
    notes = []
    for p in sorted(base.rglob("*.md")):
        rel = p.relative_to(base)
        if rel.parts and rel.parts[0] == ATTIC:
            continue
        n = parse_note(p)
        if path_filter and n.path != path_filter:
            continue
        notes.append(n)
    return notes


# --------------------------------------------------------------------------
# reads ledger (append-only, union-merged)
# --------------------------------------------------------------------------


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


# --------------------------------------------------------------------------
# staleness
# --------------------------------------------------------------------------


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


# --------------------------------------------------------------------------
# promotion + eviction (shared by `lane audit` and `lane done`)
# --------------------------------------------------------------------------


def promote_pending(root: Path):
    """Pending notes are resolved and hashed here, never at write time.

    This is what makes rebase safe: spans are fingerprinted against the tree
    as it exists at audit time, so nothing is bound to a rewritten commit.
    """
    pending = root / PENDING
    if not pending.exists():
        return []
    created = []
    for line in pending.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        rec = json.loads(line)
        target = root / rec["path"]
        sig = body = lines_str = ""
        status = TIER_MISSING
        if target.exists():
            text = target.read_text(encoding="utf-8", errors="replace")
            span = resolve_anchor(text, rec["anchor"])
            if span:
                sig, body = span_hashes(text, span)
                lines_str = "%d-%d" % span
                status = TIER_FRESH
        nid = ulid()
        note = Note({
            "id": nid, "path": rec["path"], "anchor": rec["anchor"],
            "created": rec["at"], "branch": rec["branch"], "sig": sig,
            "body_hash": body, "lines": lines_str, "status": status,
            "checked": now_iso(),
        }, rec["text"])
        d = note_dir(root, rec["path"])
        d.mkdir(parents=True, exist_ok=True)
        f = d / ("%s-%s.md" % (nid, slug(rec["text"])))
        f.write_text(note.render(), encoding="utf-8")
        note.file = f
        created.append(note)
    pending.unlink()
    return created


def evict(root: Path, note: Note, reason: str):
    """Never delete: the audit is the one writer here without a reviewer, so
    its decisions have to stay inspectable."""
    rel = note.file.relative_to(root / CONTEXT_DIR)
    dest = root / CONTEXT_DIR / ATTIC / rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    note.meta["evicted"] = "%s (%s)" % (now_iso(), reason)
    dest.write_text(note.render(), encoding="utf-8")
    note.file.unlink()
