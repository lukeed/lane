# Implementation Plans

> **STALE — pending re-audit.** These plans were written against the Python
> implementation at commit `c2f4ed4`. The tool has since been rewritten in Rust
> (`crates/lane`), which closed some of these findings outright and moved every
> file path. Do not execute them as written; they are kept as the record of what
> was found and what was decided. See "After the Rust rewrite" below.

Written 2026-08-18 against commit `c2f4ed4`, from an audit of the whole repo
(13 files, ~2,900 lines). Every finding raised in that audit is planned here
except the two that were already fixed and committed — see "Already fixed"
below.

Each executor: read the plan fully before starting, honour its STOP
conditions, and update your row when done.

## Execution order & status

| Plan | Title | Priority | Effort | Depends on | Status |
|------|-------|----------|--------|------------|--------|
| 001 | Make both suites run on macOS/BSD; prove reflink shares extents | P1 | S | — | TODO |
| 002 | Fail with a message, not a Python traceback | P1 | S | 001 | TODO |
| 003 | Stop rewriting unchanged notes; make merged frontmatter unambiguous | P1 | M | 001 | TODO |
| 004 | Stop hiding real changes behind a language-blind comment stripper | P1 | M | 001, 003 | TODO |
| 005 | Validate `lane note` paths; make `lane why` name its file | P2 | S | 001, 002 | TODO |
| 006 | Make the shell integration survive failure and survive `done` | P2 | S | 001, 002 | TODO |
| 007 | Make the warm list configurable; describe what actually happens | P2 | M | 001 | TODO |
| 008 | Delete `ctx` and the `.wt` hooks, after moving their coverage | P2 | M | 001 | TODO |
| 009 | Bound the read ledger; make its counts survive a merge | P3 | M | 001, 003 | TODO |
| 010 | Clear the four small things that mislead a reader | P3 | S | 001 | TODO |

Status values: TODO | IN PROGRESS | DONE | BLOCKED (with one-line reason) |
REJECTED (with one-line rationale)

## By severity, if you would rather work that way

The order above is a *dependency* order, not a severity order. Ranked by what
goes wrong if it is left alone:

1. **003** — silent memory corruption. A merged note can carry a stale
   fingerprint that wins by merge order, so a note that genuinely drifted
   reports `fresh` and is never reviewed. This is the failure the tool exists
   to prevent.
2. **004** — silent false negatives in drift detection, for the same reason at
   a different layer: a change inside a string literal, or to a markdown
   sub-heading, does not register.
3. **007** — a lane that does not carry `.env` does not run the project, and
   the README says it will.
4. **002, 005, 006** — the tool works but presents badly, refuses nothing, and
   can put your shell in a directory that no longer exists.
5. **009, 008, 010** — debt. Real, bounded, no user is losing work over it.

001 is first in the execution order because it is the gate everything else is
verified against, not because it is the most severe.

## Dependency notes

- **Everything depends on 001.** At `c2f4ed4` neither test suite runs on
  macOS: both call GNU-style `sed -i`, which BSD sed rejects. Every other plan
  uses `./test_lane.sh` as its verification gate, so that gate has to work
  first. On a Linux box with GNU sed you can technically skip ahead — don't;
  the reflink assertion 001 adds is also the only automated check that
  `clone_file` is really sharing extents.
- **004 depends on 003.** 004 changes what `normalize()` hashes, which changes
  every stored fingerprint. It handles that with a `NORM_VERSION` re-baseline,
  and that re-baseline only stays quiet if 003's "an audit that learned nothing
  writes nothing" rule is already in place. Landing 004 first produces a
  store-wide rewrite on the first audit.
- **009 depends on 003** for the same reason: it is the second half of the same
  idea (one writer per file), and its merge test reuses the shape 003
  establishes.
- **005 and 006 depend on 002.** Both raise `LaneError` and both assume
  `main()` renders it as a single `error:` line rather than a traceback.
- **008 should land after 003 and 004 if you want the smaller diff.** It ports
  coverage out of `test_ctx.sh` before deleting it, and 003 and 004 each add an
  assertion that covers one of the rows it would otherwise have to port. It is
  safe to run earlier; it just ports more.
- **010 is independent** of everything but 001 and can be taken at any point,
  including as a warm-up.

Assertion counts in each plan are expressed as a **delta** from whatever
`./test_lane.sh` reports before you start, precisely so this ordering can
change without invalidating a done criterion. Plan 001 is the exception and
uses absolute numbers, because it runs from the known state at `c2f4ed4`
(42 assertions).

## Already fixed — do not re-plan

Both were found in the same audit and committed before these plans were
written:

- **`lane` could not start.** The `import` commit flattened the tree, so
  `from lanelib import cow, ...` had no package to import and
  `worktree.py`'s `from . import cow` had nothing to be relative to. Fixed in
  `e35df8a`: `lanelib/` restored with an `__init__.py`, `fake-reviewer` moved
  back under `tests/`, `test_lane.sh`'s hardcoded `/home/claude/lane` replaced
  with a derived root, scripts marked executable.
- **`lane rm` discarded unlanded commits.** It ran `git branch -D`
  unconditionally, so removing a clean lane holding unmerged work deleted the
  only reference to it and exited 0. Fixed in `c2f4ed4`: deletion uses `-d`,
  the kept branch is reported with recovery instructions, `--force` now also
  means "drop those commits", and six assertions cover it.

## Findings considered and rejected

- **Replace `merge=union` with a custom merge driver for `.context/`.** A
  custom driver must be installed in every clone's git config, which is exactly
  the coordination the design refuses. Plan 003 makes union correct instead, by
  removing the spurious diffs it was being asked to resolve.
- **Make `lane new` carry all untracked and ignored files by default**, which
  is what the README currently claims. Unbounded — editor state, OS junk,
  coverage output, caches nobody wants twice — and it would duplicate secrets
  into a new tree without being asked. `--fork` already does this and says so.
  Plan 007 fixes the docs and makes the list configurable instead.
- **Keep `ctx` as a thin shim over `lanelib`.** It is an undocumented alias for
  a subset of `lane`; keeping it means a second CLI surface to test and
  document forever. Plan 008 deletes it, after porting the coverage
  `test_ctx.sh` uniquely holds.
- **Recount audit tiers after the reviewer runs**, so a `holds` verdict stops
  showing as `body-drift` in the summary. The pre-review numbers are the honest
  description of what the hash check found, and they feed `--json`'s `checked`
  key where a stable meaning matters more than a tidy one. Plan 010 relabels
  the line instead.
- **Add an environment variable for the warm list**, alongside git config and
  `--warm`. A third way to say the same thing with no new capability.
- **Add CI.** Real gap — the repo has none, and the two suites are its whole
  verification surface — but choosing a provider is the maintainer's call, not
  an executor's. Once CI exists, `./test_lane.sh` is the entire gate. Noted in
  plan 001's maintenance notes.

## Not audited

- The Anthropic API path in `lanelib/review.py` was read but never executed
  against the live API; only the `cmd` backend was exercised, via
  `tests/fake-reviewer`.
- Extent sharing was verified on APFS only. btrfs, XFS with `reflink=1`,
  bcachefs and ZFS remain unverified by the maintainer — plan 001 adds an
  assertion that covers them automatically wherever the suite runs.
- Anchor resolution is regex-based and known-loose; the audit treated that as
  the documented v0 limitation the README already declares, not as a finding.
  Plan 004's string-aware scanner is reusable there when someone takes it on.

## After the Rust rewrite

The rewrite carried across the findings whose fix was inherent to the new
implementation or too small to leave broken, and deliberately preserved the rest
so the plans stay meaningful.

**Closed by the rewrite:**

- **001** — the suite runs on macOS (`sedi` helper), and extent sharing is now a
  real assertion in `cargo test` rather than an unverified README claim.
- **002** — `anyhow` renders an expected failure as one `error:` line; panics
  stay panics. `--allow-dirty` still exists and still cannot work.
- **004** — comments come from the parse tree, so the language-blind stripper is
  gone. A changed URL inside a string is drift; a markdown signature is a real
  hash; the per-extension `SYNTAX` table the plan specified was unnecessary.
- **005** — `lane note` refuses a path outside the repo or one that does not
  exist, and `lane why` groups by `(path, anchor)` so it can no longer print
  `None#`.
- **008** — `ctx`, `test_ctx.sh`, `post-create` and `pre-done` are deleted. Every
  assertion that only `test_ctx.sh` held was ported first: init scaffolding, the
  signature-vs-body distinction, comment churn, the per-anchor budget, and the
  two-branch merge.
- **010** — the dead `pass` branch and the unused import are gone with the file
  they lived in. `.lanes-*` is still written to `.gitignore`, and the audit
  summary still reports pre-review tier counts.

**Still open, unchanged by the rewrite:**

- **003** — every audit still stamps `checked:` and rewrites every note, so
  union merge can still duplicate frontmatter keys. Frontmatter now goes through
  a real YAML serializer, which fixes the *escaping* half of the problem (a
  newline in a model's `reason` can no longer corrupt a note) but not the
  duplicate-key half.
- **006** — `lane shellenv` emits the same shell function, `tail -1` and all.
- **007** — the warm list is still the ten hardcoded entries in
  `worktree::WARM_DEFAULT`, and the README table still overstates it.
- **009** — `.reads.jsonl` is still one append-only file, union-merged.

**New, found during the rewrite:**

- A file whose language has no tree-sitter grammar resolves `@file` and nothing
  else; named anchors there report `anchor-missing` and are evicted, rather than
  being reported as unverifiable. The Python regexes were language-loose and
  would sometimes match. This wants a tier or a guard.
