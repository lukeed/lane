# lane

Copy-on-write worktrees with memory that survives them. One tool.

```
lane init                            probe reflink, scaffold memory + merge rules
lane new fix-login                   CoW worktree + branch
lane new spike --fork                clone the whole tree by reference, dirt included
lane ls
lane note -p src/auth.rs -a "fn verify" "..."
lane why src/auth.rs                 read what earlier lanes learned
lane done                            rebase, audit memory, fast-forward, remove
```

## Copy-on-write

`crates/lane/src/cow.rs` calls the kernel primitives directly — `FICLONE` (0x40049409)
on Linux via `rustix`, `clonefile(2)` on macOS — rather than shelling out to
`cp --reflink`, because per-file we need to know whether we got real sharing
or a silent full copy. `cp` will not tell you.

Works on btrfs, XFS with `reflink=1`, APFS, bcachefs, recent ZFS. Everywhere
else `probe()` reports it, `lane new` prints the verdict, and each file falls
back to a byte copy — correct tree, just an expensive one.

Two materialization strategies:

| | tracked files | untracked + ignored | dirty state |
|---|---|---|---|
| default | git checks them out | cloned by reference | not carried |
| `--fork` | cloned by reference | cloned by reference | carried |

Default is the safe one and still fixes the real problem: `git worktree add`
gives you a clean tree with a stone-cold `node_modules` and `target`. Those
are gitignored, so git will never materialize them, and they are the entire
reason a fresh worktree feels expensive.

`--fork` uses `--no-checkout`, clones everything, then rebuilds the index with
`reset --mixed` + `update-index --refresh` — populating the index from the base
tree *without rewriting a single file*, so the sharing survives. Test 3 asserts
exactly one modified file afterward, not several hundred.

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
the same bytes and a merge has nothing to resolve. Everything that changes over
time is per-writer instead: one branch, one file. `lane done` folds a lane's
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
| `body-drift` | implementation moved, contract held | flag once |
| `signature-changed` | the described thing changed shape | review |
| `anchor-missing` | symbol gone | evict to `attic/` |

A renamed or moved file is followed, not evicted: `lane audit` reads git's own rename
detection and moves the notes with it. Eviction means the file or the symbol is
genuinely gone.

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
cargo test        # 34 assertions: clone layer, anchors, hashing, state, verdicts
./test_lane.sh    # 64 assertions against real git repos in a tmpdir
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

- **No session distillation.** `lane note` still needs calling. The version
  that gets adopted derives notes from the agent session at `done` time and
  writes them to `pending.jsonl` — a producer swap, not a redesign.
- **A file whose language has no grammar resolves `@file` and nothing else.**
  Named anchors there report as missing rather than as unverifiable.
