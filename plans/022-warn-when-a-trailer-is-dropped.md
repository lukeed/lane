# Plan 022: Say something when a `Why:` trailer cannot be captured

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat 34e74cc..HEAD -- crates/lane/src/cli.rs scripts/test.sh`

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `34e74cc`, 2026-08-19

## Why this matters

The `post-commit` hook lane installs is:

```sh
# lane: capture Why trailers
command -v lane >/dev/null 2>&1 && lane capture HEAD || true
```

When `lane` is not on `PATH`, that is a silent no-op. A user who writes

```
make verify constant-time

Why: src/auth.rs#fn verify | early return leaks token length
```

gets no note, no warning, and no way to notice — the commit succeeds and the thought is
gone. This was hit for real while working on this repository: the binary was being run by
absolute path during development, `command -v lane` failed, and several trailers were
discarded before anyone noticed.

The `|| true` is correct and must stay. A commit hook that can fail a commit is worse than
one that misses a note. But *silence* is the wrong failure: the fix is to warn, on stderr,
only when there was something to capture.

## The design

Warn only when both are true: `lane` is not runnable, **and** the commit message actually
contains a `Why:` trailer. A user who does not use trailers must never see this.

```sh
# lane: capture Why trailers
if command -v lane >/dev/null 2>&1; then
  lane capture HEAD || true
elif git log -1 --format=%B | grep -qi '^Why:'; then
  echo "lane: not on PATH, so the Why trailer in this commit was not captured" >&2
  echo "lane: run 'lane capture HEAD' once lane is installed to record it" >&2
fi
```

Two properties worth keeping deliberately:

- The message names the recovery. `lane capture HEAD` re-reads the commit, so the note is
  not lost — it just has not been collected yet. Verified: running it by hand after an
  uncaptured commit records the trailer correctly.
- `grep -qi` matches the trailer key case-insensitively, because `capture.rs` compares the
  key with `eq_ignore_ascii_case("why")`. A hook that is stricter than the parser would
  stay silent on exactly the trailers the parser would have accepted.

This changes an installed hook's content. `hooks_install` compares against
`POST_COMMIT_MARKER`, not the whole block, so an already-installed hook keeps its old body
and prints `already installed`. That is acceptable and in keeping with how the skill and
protocol behave, but it must be **stated in the output**, not left for the user to discover.

## Current state

`crates/lane/src/cli.rs`:

```rust
const POST_COMMIT_MARKER: &str = "# lane: capture Why trailers";
const POST_COMMIT_BLOCK: &str = "# lane: capture Why trailers\n\
command -v lane >/dev/null 2>&1 && lane capture HEAD || true\n";
```

`hooks_install()` in the same file writes `format!("#!/bin/sh\n{}", spec.block)` when the
file is absent, prints `already installed` when it exists and contains the marker, and
refuses with exit 1 when it exists without the marker. `hooks_uninstall()` removes the block
by exact string match — `existing.replace(spec.block, "")` — so the uninstall path depends
on the block text matching byte for byte.

`crates/lane/src/capture.rs` — `parse_trailers` runs `git interpret-trailers --parse` and
matches keys with `eq_ignore_ascii_case("why")`.

`scripts/test.sh` section 22 covers capture. The harness gives you `setup`, `is`, and `sedi`
(use `sedi`, never `sed -i`, so the suite stays portable to BSD sed).

Conventions: one-line comments, and only where the reason is not obvious; `anyhow::Result`;
tests in the existing `#[cfg(test)] mod tests` at the end of `cli.rs`. Commit subjects are
Conventional Commits, `type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline unchanged |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./scripts/test.sh` | `failed: 0`, baseline + 3 |

Record both baselines before starting; at `34e74cc` they are 65 and 101.

`./scripts/check-linux.sh` cannot run from a lane — it copies the repo into a container and
a linked worktree's `.git` is a file pointing outside the mount. Do not modify it; it is out
of scope. Skip it and say so.

## Scope

**In scope**: `crates/lane/src/cli.rs`, `scripts/test.sh`.

**Out of scope**:
- `crates/lane/src/capture.rs`. The parser is correct; this is about the hook that calls it.
- The `prepare-commit-msg` hook and `PREPARE_BLOCK`.
- Upgrading an already-installed hook in place. Say it, do not do it — see Step 3.
- `crates/lane/assets/skill.md`, `AGENTS.md`, `README.md`, `USAGE.md`.

## Steps

### Step 1: Rewrite the block

Replace `POST_COMMIT_BLOCK` with the shell from "The design". Leave `POST_COMMIT_MARKER`
exactly as it is — it is what identifies lane's block in a user's existing hook, and
changing it would orphan every hook already installed.

The block is embedded in a Rust string literal, so mind the escaping and keep the trailing
newline; `hooks_uninstall` matches this text exactly.

**Verify**: `lane install hooks` in a scratch repo, then `sh -n .git/hooks/post-commit`
exits 0 — the generated script must be syntactically valid before anything else is tested.

### Step 2: Prove all three behaviours

The interesting case needs `lane` off `PATH` while the hook runs. `env -i` or a `PATH`
override for one command is enough; do not uninstall anything.

Three behaviours to confirm by hand before writing the assertions:

1. lane on PATH, commit with a `Why:` trailer → captured, as today, no warning
2. lane off PATH, commit with a `Why:` trailer → commit succeeds, exit 0, warning on stderr
3. lane off PATH, commit with **no** trailer → commit succeeds, complete silence

Then confirm recovery: after case 2, put lane back on `PATH`, run `lane capture HEAD`, and
check the note is recorded.

**Verify**: all four by hand, and paste the actual terminal output into your report.

### Step 3: Say that an existing hook is not upgraded

`hooks_install` leaves an already-installed hook alone. Add one line to that path so the
user is told the body is not refreshed and how to refresh it — removing the file and
re-running, or `lane uninstall hooks && lane install hooks`.

Keep it to one line. This is a note, not a migration.

**Verify**: `lane install hooks` twice; the second run mentions that an existing hook is
kept as-is, and exits 0.

### Step 4: Cover it

Add to `scripts/test.sh`, in or immediately after section 22:

```bash
git commit -q --allow-empty -m "silent commit

Why: src/auth.rs#fn verify | a trailer that should warn when lane is missing" 2>/tmp/nolane.err
# re-run the hook with lane off PATH, to exercise the branch
( PATH=/usr/bin:/bin sh .git/hooks/post-commit ) 2>/tmp/nolane2.err
is "a dropped trailer warns" "$(grep -c 'not on PATH' /tmp/nolane2.err)" "1"
is "and names the recovery" "$(grep -c 'lane capture HEAD' /tmp/nolane2.err)" "1"
git commit -q --allow-empty -m "no trailer here"
( PATH=/usr/bin:/bin sh .git/hooks/post-commit ) 2>/tmp/nolane3.err
is "a commit without a trailer stays silent" "$(wc -c < /tmp/nolane3.err | tr -d ' ')" "0"
```

Confirm all three fail against the pre-Step-1 binary.

**Verify**: `./scripts/test.sh` → `failed: 0`, baseline + 3.

## Done criteria

- [ ] `cargo test` unchanged at baseline; `./scripts/test.sh` passes, baseline + 3
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `sh -n` on the generated `post-commit` exits 0
- [ ] With lane off `PATH`: a trailer warns, no trailer is silent, and the commit succeeds
      in both cases
- [ ] `lane capture HEAD` afterwards still records the trailer that was missed
- [ ] `lane uninstall hooks` removes the new block cleanly, leaving no `# lane:` remnant
- [ ] `git diff --stat -- crates/lane/src/capture.rs crates/lane/assets/skill.md AGENTS.md` → empty

## STOP conditions

- The hook can make `git commit` fail, exit non-zero, or block, in any of the three cases.
  That is strictly worse than the silence being fixed.
- `lane uninstall hooks` no longer removes the block — it matches the block text exactly, so
  a mismatch between `POST_COMMIT_BLOCK` and what was written strands it in the user's hook.
- The warning fires when the commit message has no `Why:` trailer. Noise on every commit
  would get the hook uninstalled, which loses the feature entirely.

## Maintenance notes

- `POST_COMMIT_BLOCK` is matched verbatim by `hooks_uninstall`. Any future edit to the block
  strands the old text in hooks already installed. If the block changes often, that argues
  for marker-delimited replacement like plan 020 gave `AGENTS.md` — the same problem, the
  same shape of answer.
- The hook is now stricter than a no-op and looser than the parser: it greps for `^Why:`
  while `capture.rs` uses `git interpret-trailers`, which understands folded and indented
  trailers. A trailer that the parser accepts but the grep misses would be dropped silently
  again. That gap is acceptable for a warning, and would not be for the capture itself.
