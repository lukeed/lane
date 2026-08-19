# Plan 015: Capture decisions from commit trailers, without importing the git log

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 6856a10..HEAD -- crates/lane/src/`

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `6856a10`, 2026-08-19

## Why this matters

`lane note` has to be remembered, at the moment you understand something, in a separate
command. The README already admits the gap: *"No session distillation. `lane note` still
needs calling."*

Meanwhile the reason you just made a change is being typed anyway, into the commit message,
and then thrown away as far as the memory store is concerned.

**The thing to avoid is turning the git log into the note store.** A commit message says
*what changed, once*. A note says *what must stay true*. `made verify constant-time` is a
step; `must stay constant-time — early return leaks token length` is an invariant that
outlives every step. Importing the first kind wholesale would bury the second and make
`lane why` useless, which is worse than not having this feature.

So the design is not "read commits". It is "offer one narrow, explicit place to record a
decision, shaped so a commit summary cannot fit in it".

## The design

A git trailer on the commit that made the decision:

```
make verify constant-time

Rewrites the early-return path so both branches do the same work.

Why: src/auth.rs#fn verify | early return leaks token length
Why: src/sync.rs#fn reconnect | the caller holds the session lock now
```

`lane capture` reads them at commit time and appends to `.wt/pending.jsonl`; the next audit
promotes them like any other note. Nothing else changes.

### Six filters against volume, strongest first

1. **Opt-in per commit.** No trailer, no note, no cost. This is the main answer, and it is
   structural rather than a heuristic.
2. **A target is mandatory.** `Why:` without `path#anchor` is refused. You cannot record a
   note about *this commit* — only about a durable thing in the tree. A commit summary has
   no target, so it cannot be expressed in this syntax at all.
3. **The key is `Why:`, not `Note:`.** Names shape use. A field called `Why` asks for a
   reason; a field called `Note` invites a summary.
4. **Restating the subject is refused.** Normalized word overlap between the trailer text
   and the commit subject above 0.6 means the subject was pasted. Refuse that trailer,
   print why, never block the commit.
5. **Identical text on one anchor is one note.** Amend, cherry-pick and re-running the hook
   cannot produce duplicates.
6. **The existing budget.** 5 notes / 1200 chars per `(path, anchor)`, already enforced.

### Syntax, verified against git

Every claim below was checked with `git interpret-trailers --parse`:

| form | result |
|---|---|
| `Why[src/auth.rs#fn verify]: text` | **not a trailer** — brackets in the key are rejected |
| one malformed line in the block | **voids the whole block**, including valid trailers |
| `Why: src/auth.rs#fn verify \| text` | parses |
| repeated `Why:` keys | all returned, in order |
| value containing `:` and `\|\|` | preserved; split on the **first** ` \| ` |
| `Why: docs/g.md### Rate limiting \| text` | parses; split path from anchor on the **first** `#` |
| `why:` lowercase | parses; match the key case-insensitively |
| a trailer-shaped line outside the last paragraph | correctly ignored by git |

So: **key `Why`, value `<path>[#<anchor>] | <text>`**, anchor defaulting to `@file`.
Use `git interpret-trailers --parse` rather than a regex — it already knows what a trailer
block is.

### Hook behaviour, verified

- `post-commit` fires on a normal commit and on `--amend`, and **not** during `rebase`.
  Filter 5 covers the amend case, so no `post-rewrite` hook is needed.
- `git rev-parse --git-path hooks` resolves correctly from inside a lane worktree and
  honours `core.hooksPath`.
- A hook installed once fires inside every lane, because worktrees share the common dir.
- Comment lines are **only** stripped from editor commits. With `-m` and `-F` the default
  cleanup is `whitespace`, so a `#` line survives into the stored message.
- `prepare-commit-msg` receives the message source as `$2`: `message` for `-m`/`-F`,
  `commit` for `--amend`, `merge`, `squash`, `template`, or empty when an editor will open.
  Gating on empty-or-`template` is what makes a hint line safe.

## Current state

- `crates/lane/src/store.rs` — `PendingNote { text, path, anchor, branch, at }`,
  `append_pending`, `rel_to_repo`, `promote_pending`.
- `crates/lane/src/cli.rs` — `note()` validates the path and warns; `init()` scaffolds.
- `crates/lane/src/git.rs` — `git`, `try_git`, `git_ok`, `current_branch`.
- `.wt/pending.jsonl` is gitignored by `lane init`, so appending to it never dirties the
  tree. That is what makes a `post-commit` hook safe here and unsafe for `lane audit`.

Conventions: one-line comments, `anyhow::Result`, `#[cfg(test)] mod tests` at file end.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 6 |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 6 |

At `6856a10` the baselines are 34 and 64.

## Scope

**In scope**: `crates/lane/src/capture.rs` (new), `cli.rs`, `store.rs`, `lib.rs`,
`test_lane.sh`, `README.md`, `USAGE.md`.

**Out of scope**:
- Inferring notes from a diff, a commit body, or an agent session. This plan reads one
  explicit field and nothing else. Anything that guesses belongs in its own plan and needs
  its own argument.
- Blocking or rewriting commits. The hook never fails a commit.
- `pre-commit`, `post-merge`, `post-rewrite`. Two hooks, and neither can fail a commit.
- Running `lane audit` from a hook — it writes tracked files and would leave every commit
  with a dirty tree.

## Steps

### Step 1: Parse trailers into pending notes

New `crates/lane/src/capture.rs`:

```rust
pub struct Captured {
    pub path: String,
    pub anchor: String,
    pub text: String,
}

/// Parse `Why: <path>[#<anchor>] | <text>` trailers from a commit message.
pub fn parse_trailers(message: &str) -> Vec<Result<Captured, String>>
```

Feed the message to `git interpret-trailers --parse` on stdin, keep lines whose key is
`Why` case-insensitively, then for each value:

- split on the first ` | `; no separator is an error naming the expected form
- split the left side on the first `#`; anchor defaults to `@file`
- empty path or empty text is an error

Return per-trailer results so one bad line reports itself without discarding the others.

**Verify**: unit tests for each row of the syntax table above, including the markdown
anchor and a value containing `:` and `||`.

### Step 2: Refuse a pasted subject

```rust
/// Word overlap between the trailer and the commit subject. A pasted subject scores 1.0.
fn restates(subject: &str, text: &str) -> bool
```

Lowercase both, keep alphanumeric words of 3+ characters, and compare Jaccard similarity
against a threshold of 0.6. Reject at or above it with a message that says what the field
is for.

Measured against the worked example — subject `make verify constant-time`, note
`must stay constant-time; early return leaks token length` — the score is about 0.18, so
real notes are nowhere near the threshold.

**Verify**: unit tests — an exact paste is refused, the worked example is accepted.

### Step 3: `lane capture <rev>`

A hidden subcommand (`#[command(hide = true)]`), because it is machinery rather than an
everyday verb:

- read the message with `git log -1 --format=%B <rev>` and the subject with `%s`
- parse trailers; for each success, validate the path with `store::rel_to_repo` and check
  it exists, exactly as `note()` does
- append to `.wt/pending.jsonl` via `store::append_pending`
- print one line per captured note and one `warning:` line per rejected trailer, all on
  stderr
- **always exit 0.** A hook that fails a commit over a malformed note is worse than the
  note being lost.

**Verify**: `lane capture HEAD` after a commit carrying two trailers appends two lines to
`.wt/pending.jsonl` and exits 0; a commit with a malformed trailer prints a warning and
still exits 0.

### Step 4: Deduplicate at promotion

In `store::promote_pending`, skip a pending record whose `(path, anchor, trimmed text)`
already matches a live note. This covers `--amend`, a cherry-pick, running the hook twice,
and a human who also typed `lane note`.

**Verify**: a unit test — promoting the same pending record twice yields one note.

### Step 5: Put the syntax in front of the user, not in their memory

The one thing this feature asks of a user is a line in a shape they have to know. Do not
make them remember it. `lane hooks install` also writes `prepare-commit-msg`:

```sh
#!/bin/sh
# lane: offer the Why form when an editor will open
case "$2" in
  ""|template) printf '\n# Why: <path>#<anchor> | what must stay true (optional, lane note)\n' >> "$1" ;;
esac
```

The `case` is load-bearing, not decoration: git strips `#` lines from editor commits but
**not** from `-m`, so an ungated hint would end up inside every message committed that way.

**Verify**: `git commit -m x` stores a message with no `#` line; an editor commit shows the
hint and stores a message without it.

### Step 6: `lane hooks install`

Writes `post-commit` into `git rev-parse --git-path hooks`:

```sh
#!/bin/sh
# lane: capture Why trailers
command -v lane >/dev/null 2>&1 && lane capture HEAD || true
```

- if no hook exists, create it executable
- if a hook exists and already contains the marker comment, say so and do nothing
- if a hook exists without the marker, **do not modify it** — print the two lines to add
  and exit 1

Add `lane hooks uninstall` that removes only the marked block, and mention both in
`lane init`'s output so the feature is discoverable without reading the docs.

**Verify**: `lane hooks install` twice is idempotent; against a foreign hook it refuses and
prints the snippet.

### Step 7: Cover it end to end

Add a section to `test_lane.sh` before the summary. Six assertions:

```bash
echo "== N. decisions are captured from commit trailers =="
setup
"$LANE" hooks install > /dev/null
git commit -q --allow-empty -m "make verify constant-time

Why: src/auth.rs#fn verify | early return leaks token length"
is "the trailer became a pending note" \
   "$(grep -c 'early return leaks' .wt/pending.jsonl)" "1"
"$LANE" audit > /dev/null
is "and promotes like any other note" \
   "$("$LANE" why src/auth.rs | grep -c 'early return leaks')" "1"

git commit -q --allow-empty -m "tidy imports"
is "a commit with no trailer records nothing" \
   "$([ -f .wt/pending.jsonl ] && echo yes || echo no)" "no"

git commit -q --allow-empty -m "refactor the parser

Why: refactor the parser" 2> /tmp/cap.out
is "a pasted subject is refused" "$(grep -c 'warning:' /tmp/cap.out)" "1"
is "and records nothing" "$([ -f .wt/pending.jsonl ] && echo yes || echo no)" "no"

git commit -q --allow-empty -m "note it twice

Why: src/auth.rs#fn verify | early return leaks token length"
"$LANE" audit > /dev/null
is "an identical note is not duplicated" \
   "$(grep -rl 'early return leaks' .context/- --include='*.md' | wc -l | tr -d ' ')" "1"
```

Confirm each fails against the current code before implementing.

### Step 8: Document it

A short `USAGE.md` section under "Leave notes while you work": the syntax, that a target is
required, and the one sentence that matters — **record why it must stay true, not what you
did**. Add `lane hooks install` / `uninstall` to the Reference table, and replace the
README's "No session distillation" bullet with what now exists and what still does not.

**Verify**: `grep -c 'Why:' USAGE.md` → at least `1`.

## Done criteria

- [ ] `cargo test` passes, baseline + 6; `./test_lane.sh` passes, baseline + 6
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] A commit with no `Why:` trailer creates no pending file
- [ ] A malformed or subject-restating trailer warns, records nothing, and exits 0
- [ ] `lane hooks install` is idempotent and refuses to touch a foreign hook
- [ ] `git commit -m` stores no `#` hint line; an editor commit shows one and stores none
- [ ] The same trailer on two commits yields one note
- [ ] `plans/README.md` row updated

## STOP conditions

- `git interpret-trailers --parse` behaves differently on the installed git than the table
  above records. Report the version and the output; the syntax was verified on this
  machine, not on every git.
- The similarity check in step 2 refuses a note you would want to keep. Report the pair
  rather than raising the threshold on your own — if real notes score near 0.6 the measure
  is wrong, not the number.
- You find yourself wanting to read the diff, the body, or anything other than the `Why:`
  trailer to decide what to record. That is the line this plan exists to hold. Stop and
  make the case separately.
- Installing the hook makes any existing assertion fail. `setup` builds a fresh repo per
  section, so the hook must not leak between sections.

## Maintenance notes

- The rule to defend in review: **this feature reads one explicit field and never guesses.**
  Every proposal to infer notes from commits should be weighed against what it does to
  `lane why` output, which is the only thing that matters here.
- Filters 1 and 2 are structural; 4 is a heuristic. If the heuristic ever gets in the way,
  delete it — the structural ones carry the design.
- The hint in `prepare-commit-msg` must stay gated on `$2`. Ungating it puts a `#` line
  into every `-m` commit message in the repository.
- `post-commit` is safe only because `.wt/pending.jsonl` is gitignored. Anything a future
  hook writes into a tracked path leaves every commit dirty and breaks `lane done`'s
  refusal to land a dirty lane.
- Deferred: capturing from an agent session at `done` time, which is the other half of the
  README's session-distillation gap. Same producer, same `pending.jsonl`, different source,
  and a much harder argument about what deserves recording.
