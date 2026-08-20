# Implementation Plans

Re-audited 2026-08-18 against commit `43e404f`, after the Rust rewrite; 013 and 014
added 2026-08-19 against `43e404f`. Every finding
below was re-verified against the current tree, not carried over on trust.

Each executor: read the plan fully, honour its STOP conditions, record the baseline test
counts before starting, and update your row when done.

## Execution order & status

| Plan | Title | Priority | Effort | Depends on | Status |
|------|-------|----------|--------|------------|--------|
| 014 | Follow a renamed file instead of discarding its memory | P1 | M | — | DONE |
| 011 | Stop discarding notes whose anchor we cannot resolve | P1 | M | — | DONE |
| 013 | Make note files immutable, everything that changes per-writer | P1 | M | — | DONE |
| 015 | Capture decisions from commit trailers, without importing the git log | P2 | M | — | DONE |
| 006 | Make the shell integration survive failure and survive `done` | P2 | S | — | DONE |
| 007 | Carry what git ignores, and nothing at all without reflink | P2 | M | — | DONE |
| 016 | Fail `lane done` before it writes, not after | P2 | S | — | DONE |
| 010 | Clear the three small things that mislead | P3 | S | — | DONE |
| 018 | Keep the pending queue out of the worktree | P1 | S | — | DONE |
| 017 | Teach agents to use lane, via `lane install skill` | P2 | M | 018 | DONE |
| 019 | Put lanes inside the repository, and make their paths survive a move | P1 | M | — | DONE |
| 020 | Let `lane init` repair a protocol it wrote earlier | P1 | S | — | DONE |
| 021 | Stop a cloned symlink from pointing back at the parent worktree | P1 | S | — | DONE |
| 022 | Say something when a `Why:` trailer cannot be captured | P2 | S | — | DONE |
| 023 | Make an installed hook replaceable, and stop `uninstall` lying | P1 | S | — | DONE |
| 024 | Stop an audit from erasing the drift it just found | P1 | M | — | DONE |
| 025 | Stop counting reads, and make `lane why` a pure read | P2 | M | — | DONE |
| 026 | Serialize landings with a lock, and mark them in trunk's history | P1 | M | — | DONE |
| 027 | Preserve the baseline that was actually compared against | P1 | M | — | DONE |
| 028 | An interactive tour that teaches lane by driving it | P3 | M | — | DONE |
| 029 | Put everything lane owns under `.lane/` | P2 | L | — | DONE |
| 030 | Make the state file impossible to half-write | P2 | S | — | IN PROGRESS |
| 012 | Make the grammar set a build-time choice | P3 | M | 011 | TODO |
| 003 | Stop rewriting unchanged notes, so a merge cannot destroy one | P1 | M | — | DONE |
| 009 | Bound the read ledger and make its counts survive a merge | P3 | M | 003 | SUPERSEDED by 013 |

Status values: TODO | IN PROGRESS | DONE | BLOCKED (reason) | REJECTED (rationale)

## Severity, if you would rather work that way

1. **014** — `git mv` on a noted file evicts every note about it on the next audit, and
   `lane why <new path>` returns nothing. Verified. Renaming files is routine, and git
   reported `R100` the whole time.
2. **011** — a note on any language outside the thirteen shipped grammars is created and
   evicted on the first audit. Verified with Swift. A regression from the Python
   implementation, whose loose regexes matched `func verify` fine.
3. **013** — the design fix behind 003 and 009. Not urgent on its own now that 003 has
   landed, but every remaining mutation of a shared file is a merge hazard, and it is
   cheaper to do before more state accumulates than after.
4. **015** — the reason for a change is typed into the commit message and thrown away.
   Not a defect; the largest gap between what the tool promises and what it collects.
5. **007** — a lane carries nothing for a monorepo and no `.env` anywhere, so it does not
   run the project; and without reflink it byte-copies the caches it exists to share.
6. **016** — `lane done` can fail after committing memory, leaving trunk unadvanced.
7. **006** — the documented shell function can `cd` into an error message, or leave you in
   a deleted directory.
8. **010, 012** — cosmetics and binary size. No user loses work.

011 and the closed half of 003 are regressions the rewrite introduced. 014 predates it —
the Python implementation lost memory on rename too, and neither audit caught it until the
immutable-notes design forced the question of what a note's path really is.

## Dependency notes

- **012 depends on 011.** Feature-gating grammars means a trimmed build has fewer
  languages; without 011's `unverifiable` tier that silently evicts notes, turning a size
  optimisation into data loss.
- **013 supersedes 009** and removes the need for the workarounds 003 landed. 003 stays
  landed; 013 removes the class rather than the symptom.
- **014 is independent of 013**, and should go first because it is losing memory today.
  013 makes 014's implementation simpler — once the directory is the only source of a
  note's path, following a rename is a pure file move with no content change — so if you
  are doing both, note the ordering caveat in 014 step 2.
- **017 depends on 018.** Both edit `init()` in `cli.rs`; 018 removes its `.gitignore`
  write, 017 adds a line to `PROTOCOL`. Landing 018 first keeps the rebase trivial.
- **019 must land with no lanes open.** It relocates them; a lane created under the old
  sibling layout is invisible to `lane ls` afterwards. Land or remove every lane first.
- **021 is independent of 019** but shares its theme, and touches `cow.rs`, which 019
  explicitly excludes so the two can run in parallel lanes.
- **027 repairs 024.** 024's guarantee — drift stays flagged — is void across a branch
  boundary, which is every lane. Its tests were single-branch and could not see it.
- **020 is independent of everything.** It only touches `init()`'s `AGENTS.md` branch and
  is the one plan with a real, unsynthesised fixture: this repository's own stale protocol.
- **015 is independent of everything.** It adds a producer for `pending.jsonl`; nothing
  downstream knows where a pending note came from.
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
  `..`, so `../outside.txt` still escaped — fixed in `43e404f` with canonicalization and
  lexical `..` folding, plus two tests. *The unresolvable-anchor warning moved into 011.*
- **008 — retire the superseded design.** `ctx`, `test_ctx.sh`, `post-create` and
  `pre-done` are deleted. Every assertion only `test_ctx.sh` held was ported first: init
  scaffolding, signature-versus-body, comment churn, the per-anchor budget with its attic,
  and the two-branch merge.
- **010, partially.** The dead `if kw ... { pass }` branch and the unused `ATTIC` import
  went with the file they lived in. Three items remain and are still plan 010.

## Findings considered and rejected

- **An append-only state file.** Proposed to make `.lane/branch/<name>/state.json`
  conflict-free under `merge=union`, like the log beside it. Rejected: plan 026's lock
  already serializes the only place two branches write one state file, so the merge benefit
  had evaporated by the time it was considered. The remaining argument was torn writes,
  which an atomic rename fixes for three lines and no migration — see plan 030. Append-only
  would buy that at the cost of unbounded growth plus periodic compaction, and compaction is
  itself a rewrite, so the problem returns. Revisit only if state gains concurrent writers
  or the landing lock is removed.

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
