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
lane done --no-merge                 stop at the commit, for a pull request to carry
lane sweep                           remove lanes whose branch has landed
```

Lanes live in `.lane/trees/` inside the repository and are excluded through `.git/info/exclude`,
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

`.lane/` holds lane's memory, its record, and worktrees:

```
.lane/
  memory/<path>/<ulid>-<slug>.md the note; written once, never rewritten
  attic/<path>/<ulid>-<slug>.md  the same file, retired, byte-identical
  log.jsonl                      the record: evictions, holds, landings
  trees/<name>/                  the lane worktree
```

Nothing derived is committed. A note file is written once and never modified, and
the log is only ever appended to, so two branches can never edit the same bytes
and a merge has nothing to resolve.

A note's baseline — the shape of the code it describes — comes from its own
frontmatter, written at creation, and is moved only by a `holds` record. Both are
committed, so every clone compares against the same thing and a confirmation
survives a merge you did not make. The current fingerprint is recomputed every run
and stored nowhere.

`memory/` is reserved. Everything under it mirrors your paths, which is why a repo may
have its own `attic/` without colliding with ours.

The only `merge=union` rule is for `log.jsonl`, the one genuinely append-only
file — which is what union merge is actually for. Notes need no rule because
they never change; a conflict on one means two people disagreed about `pinned`,
and that should be loud.

Staleness is computed per anchor, not per file, against a normalized hash of
that anchor's span only:

| tier | meaning | action |
|---|---|---|
| `fresh` | span unchanged | none |
| `content-changed` | implementation moved, contract held | stays flagged until resolved |
| `contract-changed` | the described thing changed shape | resolve |
| `anchor-missing` | symbol gone | evict to `attic/` |

A renamed or moved file is followed, not evicted: `lane audit` reads git's own rename
detection and moves the notes with it. Eviction means the file or the symbol is
genuinely gone.

A drifted note stays flagged until you run `lane holds <id>`, replace it with
`lane note --supersedes <id>`, or delete its note file and commit. Until then,
`lane check` keeps reporting it.

Anchors are `fn verify`, `#script`, `## Heading`, `@file`, resolved by
tree-sitter rather than by regex: a span ends where the declaration ends, a `#`
in a code fence is not a heading, and a brace inside a string does not truncate
anything. Comments come from the parse tree too, so a formatter run does not
stale the store, a changed URL inside a string does, and editing `#script`
leaves a note on `#style` alone.

Each `(path, anchor)` has a hard budget (5 notes / 1200 chars). Audit keeps
notes in this order: `pinned > touched-by-this-lane > freshness > age`, then
evicts the remainder to the attic with a timestamped reason. Reading a note is
not a vote for it: you often read one to find out it is wrong.

## Editor pickers

To keep lane's store out of pickers built on ripgrep or fd, add a repo-root `.ignore`:

```
.lane/
```

Ripgrep and fd honour this file, so pickers built on them inherit it. Git ignores
`.ignore` completely. VS Code does not read `.ignore`; add these entries to
`.vscode/settings.json` instead:

```json
{
  "files.exclude": { ".lane/": true },
  "search.exclude": { ".lane/": true }
}
```

Lane writes neither file; they are yours to add.

## Why `done` runs the audit after the rebase

Notes stay as pending JSON until audit. `lane done` rebases first, *then*
resolves and fingerprints spans against the post-rebase tree. Nothing is ever
anchored to a commit the rebase is about to rewrite, so `notes.rewriteRef` is
unnecessary and squash semantics never come up.

## Pull requests

`lane done` lands the lane itself: it fast-forwards trunk and removes the worktree.
Where trunk is protected, `lane done --no-merge` stops one step earlier — rebase,
audit, commit — and leaves the merge to wherever the pull request is merged.

```bash
lane done --no-merge
git push -u origin fix-login
```

Turn on **"Require branches to be up to date before merging."** The audit fingerprints
spans against the post-rebase tree, so a pull request merged on a stale base describes
a tree nobody has. That setting is what serializes two clones, the way the landing lock
serializes two lanes on one machine.

Then the lane sits on disk until the pull request merges, which may be a week. `lane ls`
marks it `landed` once trunk carries its landing record, and `lane sweep` removes it:

```
$ lane ls
fix-login    fix-login    landed   clean   0 pending note(s)
$ lane sweep
removed fix-login
```

The marker is tree content, not a commit, so a squash or a rebase merge cannot hide it —
`git branch -d` refuses both even when the trees are identical. It names the lane rather
than the branch, because `fix` twice in a week is normal and the second one has landed
nothing. Sweep still checks that nothing on the branch is missing from trunk, so work
committed to the lane after the pull request merged is never discarded, and it removes
nothing you are standing in.

## Install

A prebuilt binary, for macOS and Linux on arm64 and x86_64:

```bash
curl -fsSL https://lane.lukeed.com | sh
```

It lands in `~/.local/bin`. Set `LANE_INSTALL` to move it, `LANE_VERSION` to pin
a release, and read the script first at <https://lane.lukeed.com/install.sh>.

The same binary through cargo, or from source:

```bash
cargo binstall --git https://github.com/lukeed/lane lane
cargo install --git https://github.com/lukeed/lane
cargo install --path crates/lane          # from a checkout
```

Once per machine, in `.zshrc` or `.bashrc`:

```bash
eval "$(lane shellenv)"     # makes `lane new` leave you inside the lane
```

Once per repository:

```bash
cd yourrepo && lane init    # scaffold .lane/, merge rule, AGENTS.md protocol
lane install hooks          # optional: capture `Why:` trailers from commits
lane install skill          # optional: the fuller agent workflow for .agents/
```

Rust 1.85+, edition 2024. Anchor resolution ships grammars for rust, go, python,
javascript, typescript, tsx, c, c++, java, bash, css, html and markdown; adding
one is a line in the table in `crates/lane/src/syntax.rs`.

## Tour

```bash
cargo run -p lane-tour -- start
```

Builds a throwaway sandbox repository next to this checkout, prints its path, and
waits. Open that directory in an editor, then pick numbered options to drive real
workflows — a note drifting, three lanes landing out of order, two landings
colliding. Every command is printed before it runs, so you finish having seen the
ones you would actually type. Delete the sandbox when you are done.

The tour is a separate binary and shares nothing with `lane`.

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

## Resolving drift

`lane check` lists every note that is not fresh, each with the id you need next.
Use `lane holds <id>` when it remains true, `lane note -p <path> -a <anchor>
--supersedes <id> "<rewrite>"` when its sentence must change, or delete the note
file and commit when the constraint is gone. Any unambiguous prefix of an id
works. `--json` adds each note's body and current span. Lane never calls a model.
See [lane.lukeed.com/usage](https://lane.lukeed.com/usage).

## Still stubbed

- **Commit decisions are explicit.** `lane install hooks` captures targeted `Why:`
  trailers into `pending.jsonl`. Agent sessions are not distilled yet; without a
  trailer, `lane note` still needs calling.
- **A file whose language has no grammar resolves `@file` and nothing else.**
  Named-anchor notes there are kept and reported `unverifiable`. Supporting drift
  checks for another language requires a table entry in `crates/lane/src/syntax.rs`.
