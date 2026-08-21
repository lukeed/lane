# Plan 031: Take the model out of the binary, and make the verdicts verbs

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat 961aae2..HEAD -- crates/lane/src/review.rs crates/lane/src/audit.rs crates/lane/src/cli.rs crates/lane/src/note.rs`
> and `git diff --stat e9f5435..HEAD -- www/src`

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: design
- **Planned at**: commit `961aae2`, 2026-08-21. Line numbers in `crates/` read there and
  not run; the `www/src` ones re-verified at `e9f5435`, after the site landed.

## Why this matters

`lane audit` sends drifted notes to a language model and applies its verdicts:
`holds` refreshes the fingerprint, `superseded` **writes a new note with the model's
own text** and attics yours, `contradicted` evicts. Off unless `ANTHROPIC_API_KEY` or
`LANE_REVIEW_CMD` is set, so nothing is broken today — this is a design correction, not
a defect report. Four reasons to make it.

**1. It contradicts the rule the rest of the design is built on.** `explainer.md` argues
a note is worth keeping precisely because a person decided to write that sentence: *"the
mechanism has no input except the sentence you chose to write"*, and a note nobody chose
to write *"would be the git log again, which is the thing this design refuses."* Review
is the one place lane breaks that rule on itself.

**2. A model-written note is unmarked.** `Meta` (`note.rs:12`) is `id, anchor, created,
branch, norm, sig, body_hash, raw_hash, lines, supersedes, pinned`. There is no author
field. A note written by `superseded` is byte-identical in shape to one you typed: same
branch name, same date, same rendering under `lane why`. The only trace is a
`kind: "verdict"` record in `log.jsonl`, which nobody reads six months later. So the
load-bearing sentence about `fn verify` may or may not have been written by a person and
the store cannot tell you which.

**3. The verdicts are not verbs, which is why the model ended up inside the binary.**
`refresh_holds` (`audit.rs:66`) is private and called only from the review path.
`lane note` (`cli.rs:74`) takes `{text, path, anchor}` and cannot supersede anything.
`lane check --json` (`cli.rs:796`) emits `{id, path, anchor, tier}` — pointers, not work
items. So today:

| resolution | who can do it |
|---|---|
| **holds** — looked, still true, stop flagging | nobody |
| **superseded** — write a replacement, attic the old | nobody |
| **contradicted** — it is wrong, drop it | you, by deleting the file by hand |

The reviewer is not *one* way to resolve drift; it is the only way to resolve it into
anything but deletion. `USAGE.md` claims drift *"keeps getting reported until a reviewer
resolves it or you rewrite it"*, but "you rewrite it" means deleting a file and writing a
fresh note that loses the `supersedes` link. The model became the API because it was the
only caller.

**4. The caller usually is a model already.** Lane's audience is agents. An agent in the
lane has the diff, the surrounding code and the commit it just wrote; the review prompt
gets one span and one sentence, on a model lane picked, using a key it found in the
environment. Shipping an HTTP client and a TLS stack inside a git tool duplicates the
caller with strictly less context. `plans/README.md` already records that the Anthropic
backend *"has never been run against the live API"*.

Removing it also makes `lane done` fully local and deterministic again: no network, no
rate limit, no 120s timeout between you and a landing.

**What replaces it**: the three resolutions become commands, `lane check --json` becomes
a complete work item, and `crates/lane/assets/skill.md` teaches the loop. An agent then
resolves drift by typing the same commands you would — so the note is authored by
whoever ran them and stamped with `branch` like every other note, and reason 2 dissolves
without adding an `author:` field.

**The honest cost**: `lane done` stops quietly tidying drift. Resolution now happens only
when an agent or a person does it. That is the same trade the trailer design already
makes, but it is a real behaviour change, and it puts weight on `lane check` being loud
enough that the pile does not grow. Accept it or stop here.

## Current state

Verified at `961aae2`:

```
crates/lane/src/review.rs                 319 lines — Reviewer trait, Null, Cmd, Anthropic,
                                          prompt, parse_response, build()
crates/lane/src/review.rs:249             build(mode, cmd) — the precedence table, untested
crates/lane/src/cli.rs:6                  use crate::review;
crates/lane/src/cli.rs:35-42              struct ReviewArgs { review, review_cmd, review_max }
crates/lane/src/cli.rs:111,129            ReviewArgs on Audit and Done
crates/lane/src/cli.rs:188-197            threaded into audit_cmd and done
crates/lane/src/audit.rs:66               fn refresh_holds — private, review-only
crates/lane/src/audit.rs:230-320          the verdict applier
crates/lane/src/audit.rs:355-374          the "reviewed N drifted note(s) via X" report block
crates/lane/Cargo.toml                    ureq = { features = ["json", "rustls"] }
tests/fake-reviewer                       the only backend exercised
```

Callers of the flags outside `crates/lane`:

```
crates/lane-tour/src/scenes.rs:112,258,263,264,295,309   --review none
test_lane.sh:159,185,580,589                             --review cmd --review-cmd "$FAKE"
test_lane.sh:574,577,597,606,622 (and others)            --review none
```

`--review anthropic` and `LANE_REVIEW` appear nowhere outside `review.rs` and the docs.
In every `--review cmd --review-cmd X` call site the `cmd` is a no-op: `--review-cmd X`
alone already resolves to `Cmd` through the `auto` branch.

Conventions: one-line comments, and only where the reason is not obvious; `anyhow::Result`;
tests in `#[cfg(test)] mod tests` at file end. Commit subjects are Conventional Commits,
`type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 4, see Step 8 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 5 |
| Linux gates | `./scripts/check-linux.sh` | exit 0 |
| Site typecheck | `cd www && bun run build` | 0 errors (Step 7 only) |

Record both Rust baselines before starting. This plan was written without running them.

Capture every exit code without a pipe (`cmd > /tmp/out 2>&1; echo $?`). Piping to `tail`
reports `tail`'s status and has already masked a failure in this project.

## Scope

**In scope**: `crates/lane/src/{review.rs,cli.rs,audit.rs,note.rs,store.rs}`,
`crates/lane/Cargo.toml`, `crates/lane/assets/skill.md`, `crates/lane-tour/src/scenes.rs`,
`tests/fake-reviewer`, `test_lane.sh`, `README.md`, `USAGE.md`, `AGENTS.md`, and the site
files listed in Step 7.

**Out of scope**:
- The tiers themselves, the hashing, `syntax.rs`, the attic, the budget numbers.
- Eviction ranking. `holds` refreshes a fingerprint; it does not touch `eviction_key`.
- Adding an `author:` field to `Meta`. Steps 1–2 make it unnecessary; if you find you
  still want it, that is a separate plan.
- Any replacement reviewer, in-binary or spawned. See "Rejected" below.
- A `lane resolve` interactive TUI. Tempting, and a different decision.

## Steps

### Step 1: `lane holds <id>` — re-vouch for a note

The most common resolution has no command. Add one.

Promote `refresh_holds` out of `audit.rs`'s private section, and add a subcommand that
takes a note id. It re-checks the note, writes the current `sig`/`body_hash`/`raw_hash`/
`norm` onto its state entry, sets the tier to `FRESH`, and appends a `kind: "holds"`
record to `log.jsonl` carrying the id, path, anchor and branch — the same shape the
verdict record used, so history stays uniform across the change.

**An id argument means any unambiguous prefix**, resolved through one helper that
`--supersedes` shares. `lane why` prints ten characters of a ULID, not twenty-six, so
exact matching would mean the id a reader can see is not the id the verb accepts. An
ambiguous prefix is an error that lists what it matched; `lane holds` echoes the whole
resolved id, and `lane note --supersedes` stores the whole id on the pending record so
promotion is never handed a prefix that has since grown ambiguous.

**It must refuse when the anchor does not resolve.** Vouching for a span that is not there
is a lie the store would then carry as fact. Exit non-zero with a message naming the tier.

**Verify**: in a scratch repo, edit a noted function body, `lane check` → `body-drift 1`
and a row naming the note; paste the ten characters it printed into `lane holds`;
`lane check` → `fresh` for that note and `body-drift 0`. Then delete the function and
`lane holds <id>` → non-zero, and `lane check` still reports `anchor-missing 1`. Also
check that a prefix matching two notes is refused and names both.

### Step 2: `lane note --supersedes <id>` — write a replacement

Add the flag to `Note` in `cli.rs`. Carry `supersedes` on the pending record so
promotion, not `note` itself, applies it: `lane note` stays a pure queue write, and the
transition lands where the reviewer's did.

At promotion, do what `audit.rs:270-298` does today — write the new note with
`Meta.supersedes` set to the old id, `git mv` the old one to `.lane/attic/` with reason
`superseded by <new id>`, move the state entry across, drop the old one. Reuse that code
rather than writing a second copy; Step 4 deletes only its caller.

Refuse an id that does not name a live note, before anything is queued.

**Verify**: `lane note -p src/auth.rs -a "fn verify" --supersedes <old> "new text"`, then
`lane audit` → `.lane/memory/src/auth.rs/` holds one note whose front matter has
`supersedes: <old>`, `.lane/attic/src/auth.rs/` holds the old one, and `lane why
src/auth.rs` shows only the new text.

### Step 3: Make `lane check --json` a work item, not a pointer

Add `note` (the body text) to every record, and `span` (the current text of the resolved
span) to non-`fresh` records only. Fresh notes are the bulk and nobody acts on them; the
drifted ones are the working set, and a caller that has to open four files to see what
moved will not bother.

Keep the existing keys.

**The human output grows a list too.** Counts alone say something drifted and not which
note, which is a dead end when the next thing you type needs an id. After the tier
counts, print one row per non-fresh note — mark, ten characters of id, `path#anchor` —
so `lane check` hands you what `lane holds` wants. `--json` must not be the only way a
person can get an id.

**Verify**: `lane check --json` on a repo with one drifted and several fresh notes →
every record has `note`, exactly the non-fresh ones have `span`, and the span text matches
the file.

### Step 4: Delete the reviewer

Now that the transitions are reachable, the model has no privileged position. Remove:

- `crates/lane/src/review.rs` entirely, and its `mod` line.
- `ReviewArgs` (`cli.rs:35-42`) and both uses; `use crate::review;`.
- The verdict applier (`audit.rs:230-320`) and the `reviewer` parameter threaded to it.
  Keep `refresh_holds` — Step 1 owns it now — and keep the supersede block Step 2 calls.
- The `reviewed N drifted note(s) via X` report block (`audit.rs:355-374`) and the
  `reviewer` / `reviewed` fields on the audit outcome, including from `--json`.
- `ureq` from `crates/lane/Cargo.toml`, and `cargo update` so the lockfile drops rustls
  and its tree.
- `tests/fake-reviewer`.
- Every `--review*` flag from `crates/lane-tour/src/scenes.rs` and `test_lane.sh`. They
  are no-ops once the flags are gone, not renames.

`ANTHROPIC_API_KEY`, `LANE_REVIEW`, `LANE_REVIEW_CMD` and `LANE_REVIEW_MODEL` stop being
read. Grep for each and confirm zero hits outside this plan.

**Verify**: `cargo build` with no network feature reachable; `cargo tree | grep -c rustls`
→ `0`; `lane audit --help` shows no review flag; `./test_lane.sh` → `failed: 0`.

### Step 5: Teach the skill the loop

`crates/lane/assets/skill.md` is where this behaviour now lives. Add a section, after the
freshness marks it already explains around line 31, that says: before `lane done`, run
`lane check`; for each `~` or `!`, read the span and the note, then take exactly one of

- `lane holds <id>` — the note is still true,
- `lane note -p <path> -a <anchor> --supersedes <id> "<rewrite>"` — still the right
  subject, wrong sentence,
- delete the note file and commit — the constraint is gone.

Say plainly that a `?` is not resolvable (no grammar) and an `x` means the symbol is gone,
so the note goes to the attic rather than being vouched for.

Keep it to the length of the sections around it. The skill is loaded into an agent's
context; it is not a manual.

### Step 6: The repository's own docs

`USAGE.md` and `explainer.md` are no longer at the root — `e9f5435` moved them under
`www/`, where Step 7 owns them. Only two files are left here.

- `README.md:88` — the `signature-changed` row says `review`. It is now `resolve`.
- `README.md:95` — "until a reviewer resolves it or a human rewrites it" → name the three
  commands.
- `README.md:205-211` — replace the whole `## Review` section with `## Resolving drift`,
  describing the three verbs and saying lane never calls a model.
- `AGENTS.md` — check the four-line protocol still reads true; it likely needs nothing.

### Step 7: The site

`www/src/pages/usage.md` is the old `USAGE.md` and `www/src/pages/memory.md` the old
`explainer.md`; both moved in `e9f5435`. Edit them there and nowhere else.

| file | change |
|---|---|
| `www/src/data/commands.ts` | drop `--review`, `--review-cmd`, `--review-max` from `audit` and `done`; fix both `usage` strings; add a `holds` entry after `why`; add `--supersedes` to `note`; drop the `reviewed`/`superseded` lines from the `done` example |
| `www/src/pages/usage.md` | `### Review` → `### Resolving drift`, rewritten around the three verbs; drop the three env rows; fix the reference table; fix the `lane done` transcript |
| `www/src/pages/index.astro` | the two `tiers` entries reading "Flagged, and sent for review at lane done." (lines 22–23); "until a reviewer resolves it" (line 217); the "Review is off until you turn it on. Set `ANTHROPIC_API_KEY`…" paragraph in Installation (line 314) |
| `www/src/pages/memory.md` | "same budget, same review" (line 127) → drop the last clause |
| `www/src/scripts/acts.ts` | act 5, see below |

**Act 5 of the workflow animation is the interesting one.** Today it prints
`reviewed 1 drift via anthropic(haiku)` and `superseded src/auth.rs#fn verify`, which is
exactly the behaviour being removed. Do not delete the beat — change who acts:

```
$ lane holds src/auth.rs#fn verify        # or the supersede, one line
$ lane done
rebased onto main
memory: +1 new; checked 8
  ...
```

The panel beside it needs no change at all. Its `land` step already moves note chip 0 to
the attic and lights chip 4 — that choreography now illustrates *you* superseding a note
instead of a model doing it, which is the point of the whole plan. Keep every line at 44
columns or fewer; that is what fits a 360px screen, and `www/src/scripts/acts.ts` is
checked against it.

**Verify**: `cd www && bun run build` → 0 errors; `rg -n 'review|ANTHROPIC|haiku' www/src`
→ no hits outside `app.css`'s `/* superseded, not deleted */` comment.

### Step 8: Cover it

`cargo test`, in `audit.rs` and `note.rs`:

1. `holds` refreshes all three hashes and sets `FRESH`.
2. `holds` on an unresolvable anchor returns an error and leaves state untouched.
3. A pending record carrying `supersedes` promotes to a note with the link set, and attics
   its predecessor.
4. `check --json` carries `span` for a drifted note and omits it for a fresh one.

`test_lane.sh`, before the summary, numbered one past the last:

5. End to end: drift a note, `lane holds`, confirm `lane check` reports it fresh and the
   change survives `lane done` onto trunk.

Then delete the assertions that only existed to exercise the reviewer
(`test_lane.sh:159,185` and their fixtures). Removing a feature should lower the count;
say so in the commit body rather than padding it back.

## Done criteria

- `rg -n 'review|ANTHROPIC|ureq' crates/ tests/ test_lane.sh` → no hits.
- `cargo tree | grep -c rustls` → `0`.
- `cargo test` at baseline + 4, `./test_lane.sh` at `failed: 0` and baseline + 5 net of
  the reviewer assertions removed.
- `lane holds`, `lane note --supersedes` and `lane check --json` documented in `README.md`,
  `USAGE.md` (or `www/src/pages/usage.md`), `www/src/data/commands.ts` and `skill.md`.
- A drifted note can be taken to fresh, superseded, or the attic without a network call.

## STOP conditions

- **Step 2 turns out to need `lane note` to write immediately** rather than queue. That
  changes what `lane note` means and needs a decision, not an executor's judgment.
- **`refresh_holds` is not sufficient** to clear a flag on its own — if the tier is
  recomputed somewhere you did not find and comes back drifted after Step 1, stop; the
  state model is more tangled than this plan assumes.
- **Any step wants to add an `author:` field to `Meta`.** That is out of scope by design.
  Report it; do not add it.
- **`USAGE.md` or `explainer.md` exists at the repository root.** You are on a tree from
  before `e9f5435` and Step 7's paths will not resolve.

## Rejected

- **Keeping `--review-cmd` as a provider-neutral hook.** It is the cheapest thing to keep
  — one process spawn, no secret handling, no vendor name, no model default to age out —
  and it was the first proposal. Rejected because it keeps lane in the business of
  choosing when a reviewer runs, which is the thing being moved to the skill, and because
  the surviving verdict applier would be code with exactly one caller again. If drift
  resolution turns out not to happen without it, that is evidence about `lane check`'s
  loudness, not an argument to put the spawn back.
- **Keeping only the `holds` verdict.** Cheapest and most reversible of the three, but it
  is still a model quietly deciding a flagged note is fine, which is the failure this
  plan is about.
- **Marking model-written notes with `author:` and keeping review.** Fixes the provenance
  symptom and none of the rest, and adds a field every future note carries.
