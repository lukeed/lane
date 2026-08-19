# SPDX-License-Identifier: MIT
"""Model-in-the-loop review of drifted notes.

A hash tells you a span changed. It cannot tell you whether the note about
that span is still true. That judgment is the one part of this design that
genuinely needs a model, and it runs at exactly one moment: `lane done`,
after the rebase, when both the note and the current code are in hand.

Verdicts:
  holds         the note is still accurate; refresh its fingerprint
  superseded    still relevant but needs rewording; a new note replaces it
  contradicted  the code now does the opposite; quarantine it
  unsure        leave flagged for a human

Only notes that actually drifted are reviewed, so a clean audit costs nothing.

Backends:
  none        default; flag and re-fingerprint, no judgment (previous behaviour)
  cmd         pipe JSON to any command, read JSON back. Works with
              `claude -p`, `codex exec`, `llm`, or a local model.
  anthropic   direct API call, ANTHROPIC_API_KEY
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import urllib.request

VERDICTS = ("holds", "superseded", "contradicted", "unsure")

SYSTEM = (
    "You audit code annotations against the code they describe. For each item "
    "you receive a note and the current text of the span it is anchored to. "
    "Decide whether the note is still accurate.\n\n"
    "Reply with ONLY a JSON object, no prose and no markdown fences:\n"
    '{"verdicts":[{"id":"<id>","verdict":"holds|superseded|contradicted|unsure",'
    '"rewrite":"<new note text, only when superseded>","reason":"<one short clause>"}]}\n\n'
    "holds: still true, even if the implementation moved.\n"
    "superseded: the underlying point survives but the wording is now wrong or "
    "misleading; supply a rewrite that is one or two sentences, concrete, and "
    "says why rather than what.\n"
    "contradicted: the code now does the opposite of what the note claims.\n"
    "unsure: the span alone is not enough to judge. Prefer this over guessing."
)


def build_payload(items):
    return {"reviews": [
        {"id": i["id"], "path": i["path"], "anchor": i["anchor"],
         "note": i["note"], "span": i["span"]} for i in items]}


def parse_response(text: str) -> dict:
    """Tolerant of fences and of a bare array, because models produce both."""
    t = re.sub(r"^\s*```(?:json)?|```\s*$", "", text.strip(), flags=re.M).strip()
    try:
        data = json.loads(t)
    except ValueError:
        m = re.search(r"[\[{].*[\]}]", t, re.S)
        if not m:
            return {}
        try:
            data = json.loads(m.group(0))
        except ValueError:
            return {}
    if isinstance(data, list):
        data = {"verdicts": data}
    out = {}
    for v in data.get("verdicts", []):
        if not isinstance(v, dict) or "id" not in v:
            continue
        verdict = str(v.get("verdict", "")).lower().strip()
        if verdict not in VERDICTS:
            verdict = "unsure"
        out[v["id"]] = {
            "verdict": verdict,
            "rewrite": (v.get("rewrite") or "").strip(),
            "reason": (v.get("reason") or "").strip()[:200],
        }
    return out


class NullReviewer:
    name = "none"

    def review(self, items):
        return {}


class CmdReviewer:
    """Pipes the payload to a command on stdin and parses stdout.

    e.g. LANE_REVIEW_CMD='claude -p --output-format text'
    """

    def __init__(self, cmd: str, timeout: int = 120):
        self.cmd = cmd
        self.timeout = timeout
        self.name = "cmd(%s)" % cmd.split()[0]

    def review(self, items):
        payload = json.dumps(build_payload(items))
        stdin = SYSTEM + "\n\n" + payload
        try:
            p = subprocess.run(self.cmd, shell=True, input=stdin, text=True,
                               capture_output=True, timeout=self.timeout)
        except subprocess.TimeoutExpired:
            return {}
        if p.returncode != 0:
            return {}
        return parse_response(p.stdout)


class AnthropicReviewer:
    def __init__(self, api_key: str, model: str = None, timeout: int = 120):
        self.api_key = api_key
        self.model = model or os.environ.get("LANE_REVIEW_MODEL",
                                             "claude-haiku-4-5-20251001")
        self.timeout = timeout
        self.name = "anthropic(%s)" % self.model

    def review(self, items):
        body = json.dumps({
            "model": self.model,
            "max_tokens": 2000,
            "system": SYSTEM,
            "messages": [{"role": "user",
                          "content": json.dumps(build_payload(items))}],
        }).encode()
        req = urllib.request.Request(
            "https://api.anthropic.com/v1/messages", data=body,
            headers={"content-type": "application/json",
                     "x-api-key": self.api_key,
                     "anthropic-version": "2023-06-01"})
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as r:
                data = json.loads(r.read().decode())
        except Exception:
            return {}
        text = "".join(b.get("text", "") for b in data.get("content", [])
                       if b.get("type") == "text")
        return parse_response(text)


def build_reviewer(mode: str = None, cmd: str = None):
    """Resolution order: explicit flag, LANE_REVIEW_CMD, ANTHROPIC_API_KEY, off.

    Defaulting to off matters: `lane done` must stay usable on a plane, and
    must never silently start spending money.
    """
    mode = mode or os.environ.get("LANE_REVIEW", "auto")
    cmd = cmd or os.environ.get("LANE_REVIEW_CMD", "")

    if mode == "none":
        return NullReviewer()
    if mode == "cmd" or (mode == "auto" and cmd):
        return CmdReviewer(cmd) if cmd else NullReviewer()
    if mode == "anthropic" or mode == "auto":
        key = os.environ.get("ANTHROPIC_API_KEY", "")
        if key:
            return AnthropicReviewer(key)
    return NullReviewer()
