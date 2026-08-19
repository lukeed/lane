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

`lanelib/cow.py` calls the kernel primitives directly — `FICLONE` (0x40049409)
on Linux, `clonefile(2)` via ctypes on macOS — rather than shelling out to
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

Notes are immutable, one file per note, ULID-named, under `.context/<path>/`,
with `merge=union` in `.gitattributes`. Concurrent lanes never touch the same
bytes, so parallel memory writes cannot conflict — no lock, no lease, no
coordination. Test 6 opens two lanes from one trunk, has both annotate the
same anchor, and lands them in sequence.

Staleness is computed per anchor, not per file, against a normalized hash of
that anchor's span only:

| tier | meaning | action |
|---|---|---|
| `fresh` | span unchanged | none |
| `body-drift` | implementation moved, contract held | flag once |
| `signature-changed` | the described thing changed shape | review |
| `anchor-missing` | symbol gone | evict to `.context/.attic/` |

Anchors are `fn verify`, `#script`, `## Heading`, `@file`. Comments and
whitespace normalize away before hashing, so a formatter run does not stale the
store, and editing `#script` leaves a note on `#style` alone (test 5).

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
git clone <this> ~/.lane && ln -s ~/.lane/lane /usr/local/bin/lane
eval "$(lane shellenv)"     # adds cd-into-the-lane behaviour
cd yourrepo && lane init
```

Python 3.9+, no dependencies.

## Tests

```
./test_lane.sh    # 36 assertions against real git repos in a tmpdir
```

## Not verified here

The container this was built in runs ext4 on a kernel with no btrfs or XFS
modules, so **extent sharing itself is untested**. What is tested: the probe
returns a correct verdict, `clone_file` is attempted per file before any
fallback, the fallback tree is byte-identical, and symlinks are recreated
rather than dereferenced. Run `lane init` on APFS or btrfs to confirm the
sharing path, and `filefrag -v` on a cloned file to see `shared` extents.

## Review

Drifted notes go to a model during `done` — the one judgment here that a hash
cannot make. Verdicts: `holds` refreshes the fingerprint, `superseded` writes a
**new** note and attics the old one (mutation would break union merge),
`contradicted` quarantines, `unsure` leaves it flagged. Off unless
`ANTHROPIC_API_KEY` or `LANE_REVIEW_CMD` is set; only drifted notes are sent.
See USAGE.md.

## Still stubbed

- **Anchor resolution is regex** (`lanelib/memory.py`, `resolve_anchor`).
  Swap for tree-sitter; nothing downstream knows how a line range was produced.
- **No session distillation.** `lane note` still needs calling. The version
  that gets adopted derives notes from the agent session at `done` time and
  writes them to `pending.jsonl` — a producer swap, not a redesign.
