# Implementation Plans

Re-audited 2026-08-18 against commit `6dc6647`, after the Rust rewrite. Every finding
below was re-verified against the current tree, not carried over on trust.

Each executor: read the plan fully, honour its STOP conditions, record the baseline test
counts before starting, and update your row when done.

## Execution order & status

| Plan | Title | Priority | Effort | Depends on | Status |
|------|-------|----------|--------|------------|--------|
| 011 | Stop discarding notes whose anchor we cannot resolve | P1 | M | — | TODO |
| 003 | Stop rewriting unchanged notes, so a merge cannot destroy one | P1 | M | — | TODO |
| 006 | Make the shell integration survive failure and survive `done` | P2 | S | — | TODO |
| 007 | Let a project choose what a lane carries | P2 | M | — | TODO |
| 009 | Bound the read ledger and make its counts survive a merge | P3 | M | 003 | TODO |
| 010 | Clear the three small things that mislead | P3 | S | — | TODO |
| 012 | Make the grammar set a build-time choice | P3 | M | 011 | TODO |

Status values: TODO | IN PROGRESS | DONE | BLOCKED (reason) | REJECTED (rationale)

## Severity, if you would rather work that way

1. **011** — a note on any language outside the thirteen shipped grammars is created and
   evicted on the first audit. Verified with Swift. A regression from the Python
   implementation, whose loose regexes matched `func verify` fine.
2. **003** — a union-merged note now fails to parse, disappears from `lane why`, and is
   atticked under an eviction line with no path. Also a regression: the Python parser took
   the last duplicate key and carried on.
3. **007** — a lane without `.env` does not run the project, and the README says it will.
4. **006** — the documented shell function can `cd` into an error message, or leave you in
   a deleted directory.
5. **009, 010, 012** — debt, size, and cosmetics. No user loses work.

Both P1s are regressions introduced by the rewrite. That is what a re-audit is for.

## Dependency notes

- **012 depends on 011.** Feature-gating grammars means a trimmed build has fewer
  languages; without 011's `unverifiable` tier that silently evicts notes, turning a size
  optimisation into data loss.
- **009 depends on 003.** Same idea at a second layer — one writer per file — and its
  merge test reuses the shape 003 establishes.
- Everything else is independent. 010 is a good warm-up.

Assertion counts are expressed as deltas from whatever `cargo test` and `./test_lane.sh`
report before you start, so this ordering can change without invalidating a done criterion.

## Closed by the Rust rewrite

Verified against the tree, not assumed. The plan files were deleted; this is the record.

- **001 — portable test suites, verified reflink.** `test_lane.sh` has a `sedi` helper, so
  it runs on BSD sed. The clone layer moved into `cargo test`, where it gained the
  assertion the shell suite could not make: a 64 MiB clone must cost under 16 MiB of free
  space. Extent sharing on APFS is now verified rather than claimed.
- **002 — errors, not tracebacks.** `anyhow` renders an expected failure as one `error:`
  line; panics stay panics. Verified: `lane new <existing>`, `lane done` outside a lane,
  and a bad `lane note` path all produce a single line and no backtrace. *The
  `--allow-dirty` remnant moved into plan 010.*
- **004 — language-aware normalization.** Comments come off the parse tree, so the
  language-blind regex is gone along with the per-extension table the plan specified. A
  changed URL inside a string is drift; a markdown signature is a real hash; a brace in a
  string no longer truncates a span; a `##` in a code fence is no longer a heading. Four
  unit tests.
- **005 — note path validation and `lane why` labelling.** `lane why` groups by
  `(path, anchor)` and cannot print `None#`. `lane note` refuses a file that does not
  exist. Containment took two attempts: the first used `std::path::absolute`, which keeps
  `..`, so `../outside.txt` still escaped — fixed in `6dc6647` with canonicalization and
  lexical `..` folding, plus two tests. *The unresolvable-anchor warning moved into 011.*
- **008 — retire the superseded design.** `ctx`, `test_ctx.sh`, `post-create` and
  `pre-done` are deleted. Every assertion only `test_ctx.sh` held was ported first: init
  scaffolding, signature-versus-body, comment churn, the per-anchor budget with its attic,
  and the two-branch merge.
- **010, partially.** The dead `if kw ... { pass }` branch and the unused `ATTIC` import
  went with the file they lived in. Three items remain and are still plan 010.

## Findings considered and rejected

- **A custom merge driver for `.context/`.** Must be installed in every clone, which is
  the coordination the design refuses. Plan 003 removes the spurious diffs instead.
- **Carrying all untracked and ignored files by default.** Unbounded, and it duplicates
  secrets into a new tree unasked. `--fork` already does this and says so. Plan 007 fixes
  the docs and makes the list configurable.
- **Runtime grammar loading (`libloading`) or WASM grammars.** Would drop the binary from
  16.2 MB to ~5.8 MB, but buys a grammar build-and-distribution story, per-platform
  artifacts, and ABI skew — for a tool installed with `cargo install`. WASM avoids the
  per-platform builds but links wasmtime, which is larger than what it replaces. Plan 012
  takes the compile-time version; revisit past ~30 grammars.
- **Recounting audit tiers after review.** The pre-review numbers honestly describe what
  the hash check found and feed `--json`'s `checked` key. Plan 010 relabels the line.
- **An environment variable for the warm list.** A third way to say the same thing.
- **Keeping `ctx` as a shim.** An undocumented alias for a subset of `lane`; a second CLI
  surface to test and document forever.
- **CI.** Still a real gap — `cargo test`, `cargo clippy`, `cargo fmt --check` and
  `./test_lane.sh` are the whole gate and nothing runs them automatically. Choosing a
  provider is the maintainer's call, not an executor's.

## Not audited

- The Anthropic reviewer backend has never been run against the live API; only the `cmd`
  backend is exercised, via `tests/fake-reviewer`.
- Extent sharing is verified on APFS only. btrfs, XFS with `reflink=1`, bcachefs and ZFS
  are covered by the same assertion wherever the suite runs, but nobody has run it there.
- Anchor resolution quality *within* a supported language. Tree-sitter fixed the extent
  and code-fence bugs; nobody has swept the thirteen declaration queries for gaps.
- `syntax::walk` recurses over the parse tree with no depth guard. Untested against a
  pathologically nested file; low confidence that it matters, non-zero that it does.
