# lane

Copy-on-write worktrees with memory that survives them. One tool.

```
lane init                            probe reflink, scaffold memory + merge rules
lane new fix-login                   CoW worktree + branch
lane new spike --dirty               carry uncommitted work too
lane ls
lane note -p src/auth.rs -a "fn verify" "..."
lane why src/auth.rs                 read what earlier lanes learned
lane done                            rebase, audit memory, fast-forward, remove
```

Lanes live in `.lanes/` inside the repository and are excluded through `.git/info/exclude`,
so nothing is committed.

## Copy-on-write

`crates/lane/src/cow.rs` calls the kernel primitives directly — `FICLONE` (0x40049409)
on Linux via `rustix`, `clonefile(2)` on macOS — rather than shelling out to
`cp --reflink`, because per-file we need to know whether we got real sharing
or a silent full copy. `cp` will not tell you.

Works on btrfs, XFS with `reflink=1`, APFS, bcachefs, recent ZFS. Everywhere
else `probe()` reports it and `lane new` leaves a plain worktree. Byte-copying
an entire build cache would perform the expensive operation lane exists to avoid.

Two materialization strategies:

| | tracked files | everything git ignores, at any depth | uncommitted work |
|---|---|---|---|
| default | git checks them out | cloned by reference | not carried |
| `--dirty` | same | same | carried |

Default is the safe one and still fixes the real problem: `git worktree add`
gives you a clean tree without anything git ignores. Lane asks git for those
entries, including nested paths such as `packages/a/node_modules` and files such
as `.env`, then clones them by reference. Untracked files that are not ignored
and uncommitted tracked changes are left behind unless `--dirty` is passed.

`--dirty` uses `--no-checkout`, clones everything, then rebuilds the index with
`reset --mixed` + `update-index --refresh` — populating the index from the base
tree *without rewriting a single file*, so the sharing survives. Test 3 asserts
exactly one modified file afterward, not several hundred.

Exclude a reported path from cloning with multi-valued git configuration:

```bash
git config --add lane.exclude target
git config --add lane.exclude packages/legacy/node_modules
```

## Memory

`.context/` holds two kinds of file and nothing else:

```
.context/
  -/<path>/<ulid>-<slug>.md     the note; written once, never rewritten
  attic/<path>/<ulid>-<slug>.md the same file, retired, byte-identical
  state/<branch>.json           per-branch cache: fingerprints and read counts
  log/<branch>.jsonl            per-branch record: verdicts and evictions
```

A note file is written once and never modified, so two branches can never edit
the same bytes and a merge has nothing to resolve. State files are written by
more than one branch, but a lock held for the duration of `lane done`
serializes those writes, so a landing is exclusive. `lane done` folds a lane's
state and log into the trunk's, so nothing accumulates.

`-` is reserved. Everything under it mirrors your paths, which is why a repo may
have its own `attic/` without colliding with ours.

The only `merge=union` rule is for `log/*.jsonl`, the one genuinely append-only
file — which is what union merge is actually for. Notes need no rule because
they never change; a conflict on one means two people disagreed about `pinned`,
and that should be loud.

Staleness is computed per anchor, not per file, against a normalized hash of
that anchor's span only:

| tier | meaning | action |
|---|---|---|
| `fresh` | span unchanged | none |
| `body-drift` | implementation moved, contract held | stays flagged until resolved |
| `signature-changed` | the described thing changed shape | review |
| `anchor-missing` | symbol gone | evict to `attic/` |

A renamed or moved file is followed, not evicted: `lane audit` reads git's own rename
detection and moves the notes with it. Eviction means the file or the symbol is
genuinely gone.

A drifted note stays flagged until a reviewer resolves it or a human rewrites it, so
`lane check` keeps reporting it.

Anchors are `fn verify`, `#script`, `## Heading`, `@file`, resolved by
tree-sitter rather than by regex: a span ends where the declaration ends, a `#`
in a code fence is not a heading, and a brace inside a string does not truncate
anything. Comments come from the parse tree too, so a formatter run does not
stale the store, a changed URL inside a string does, and editing `#script`
leaves a note on `#style` alone.

Each `(path, anchor)` has a hard budget (5 notes / 1200 chars). Audit ranks by
`pinned > reads > touched-by-this-lane > freshness > age` and evicts the
remainder to the attic with a timestamped reason. An LRU on attention, needing
no opinion about importance.

## Why `done` runs the audit after the rebase

Notes stay as pending JSON until audit. `lane done` rebases first, *then*
resolves and fingerprints spans against the post-rebase tree. Nothing is ever
anchored to a commit the rebase is about to rewrite, so `notes.rewriteRef` is
unnecessary and squash semantics never come up.

## Install

```bash
cargo install --path crates/lane
eval "$(lane shellenv)"     # adds cd-into-the-lane behaviour
cd yourrepo && lane init
```

Rust 1.85+, edition 2024. Anchor resolution ships grammars for rust, go, python,
javascript, typescript, tsx, c, c++, java, bash, css, html and markdown; adding
one is a line in the table in `crates/lane/src/syntax.rs`.

## Tests

```
cargo test        # clone layer, worktrees, anchors, hashing, state, verdicts
./test_lane.sh    # end to end, against real git repos in a tmpdir
```

## What the clone layer is tested against

Extent sharing is verified on APFS: `cargo test` clones a 64 MiB file and fails
if the filesystem spent more than 16 MiB of free space on it. The same assertion
covers btrfs, XFS with `reflink=1`, bcachefs and ZFS wherever the suite runs;
none of those have been run by the author. Where `probe()` says no, the test
skips and the fallback copy is expected to cost full price. `filefrag -v` on a
cloned file shows `shared` extents on Linux.

## Review

Drifted notes go to a model during `done` — the one judgment here that a hash
cannot make. Verdicts: `holds` refreshes the fingerprint, `superseded` writes a
**new** note and attics the old one (mutation would break union merge),
`contradicted` quarantines, `unsure` leaves it flagged. Off unless
`ANTHROPIC_API_KEY` or `LANE_REVIEW_CMD` is set; only drifted notes are sent.
See USAGE.md.

## Still stubbed

- **Commit decisions are explicit.** `lane install hooks` captures targeted `Why:`
  trailers into `pending.jsonl`. Agent sessions are not distilled yet; without a
  trailer, `lane note` still needs calling.
- **A file whose language has no grammar resolves `@file` and nothing else.**
  Named-anchor notes there are kept and reported `unverifiable`. Supporting drift
  checks for another language requires a table entry in `crates/lane/src/syntax.rs`.
