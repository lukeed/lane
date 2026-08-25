# lane

> Copy-on-write worktrees with memory that survives them. One tool.

Lane opens an isolated branch and worktree without giving up the ignored build caches that make a checkout fast. While you work, notes attach decisions to a file and symbol. When that code changes later, lane flags the note instead of quietly trusting stale context. Lane itself never calls a model.

```bash
lane new fix-login
lane why src/auth.rs
lane note add src/auth.rs -a 'fn verify' 'tokens rotate on refresh'
# edit and commit as usual
lane check
lane merge
```

## Why another worktree tool?

`git worktree add` creates a clean checkout but leaves behind everything Git ignores: `target/`, `node_modules/`, virtual environments, generated files, and local `.env` files. On a reflink filesystem, lane clones those entries by reference. The new lane starts warm while allocating new storage only for the blocks it changes.

Lane uses `clonefile(2)` on APFS and `FICLONE` on Linux filesystems that support it, including btrfs and XFS with reflink enabled. If reflinks are unavailable, lane creates a normal worktree and does not byte-copy ignored caches. `lane new --dirty` also carries tracked edits and untracked, non-ignored files; without reflinks, those changed files are copied normally.

Lanes live under `.lane/trees/` and are excluded through `.git/info/exclude`, so the worktrees themselves are never committed.

## Install

Prebuilt binaries are available for macOS and Linux on arm64 and x86_64:

```bash
curl -fsSL https://lane.lukeed.com | sh
```

The installer writes to `~/.local/bin` by default. Set `LANE_INSTALL` to choose another directory or `LANE_VERSION` to pin a release. You can also install with Cargo:

```bash
cargo binstall --git https://github.com/lukeed/lane lane
cargo install --git https://github.com/lukeed/lane
```

From a source checkout, build and replace the `lane` binary in Cargo's bin directory with:

```bash
./scripts/build.sh
```

Lane requires Rust 1.85 or newer.

## Set up a repository

Enable the shell wrapper once in `.zshrc` or `.bashrc`; it leaves the shell in the new lane after `lane new` and returns it to the main worktree after `lane merge`:

```bash
eval "$(lane shellenv)"
```

Then initialize each repository:

```bash
cd yourrepo
lane init
git add .lane AGENTS.md
git commit -m 'initialize lane'
```

`lane init` creates the memory store and a short context protocol in `AGENTS.md`, and reports whether the current filesystem supports reflinks.

Optional integrations:

```bash
lane install hooks    # capture targeted Why: trailers from commits
lane install skill    # install the fuller workflow for coding agents
```

A hook is a copy on disk, so a release that changes one needs `lane install hooks`
again. `lane check` and `lane init` name a hook block that an older release wrote.

## Daily workflow

Open a lane and read its existing context before editing:

```bash
lane new fix-login
lane why src/auth.rs
```

Record what must stay true, not a summary of the change:

```bash
lane note add src/auth.rs -a 'fn verify' \
  'must stay constant-time; early return leaks token length'
```

After committing the code, inspect any drift and land the lane:

```bash
lane check
lane merge                    # rebase, audit memory, fast-forward, remove
lane merge --squash           # land the lane as one commit
```

For a protected trunk, prepare and push the lane instead:

```bash
lane push
```

The worktree remains available while its pull request is open. After the local trunk contains the lane's changes, including through a squash or rebase merge, `lane ls` marks it `landed` and `lane prune` removes it. Prune refuses lanes that still contain uncommitted work, pending notes, or commits absent from trunk.

Run `lane --help` or `lane <command> --help` for the complete command surface.

## Commands

```bash
lane init                            # probe reflink, scaffold memory + context protocol
lane new fix-login                   # CoW worktree + branch
lane new spike --dirty               # carry uncommitted work too
lane ls
lane anchors src/auth.rs             # discover canonical anchors and line ranges
lane note add src/auth.rs -a "fn verify" "..."  # record a finding
lane note edit <id>                              # choose a lifecycle action interactively
lane note replace <id> "replacement text"       # queue a successor
lane note confirm <id>                           # re-vouch for a drifted note
lane note retire <id>                            # move a live note to the attic
lane note restore <id>                           # restore a retired note
lane note pin <id>                               # protect a note from eviction
lane note unpin <id>                             # remove eviction protection
lane why src/auth.rs                 # read what earlier lanes learned
lane merge                           # rebase, audit memory, fast-forward, remove
lane push                            # rebase, audit, and push for a pull request
lane prune                           # remove lanes whose branch has landed
```

## Memory

Memory is ordinary Markdown committed with the repository:

```text
.lane/
  memory/<path>/<ulid>-<slug>.md   live notes and confirmed fingerprints
  attic/<path>/<ulid>-<slug>.md    retired notes, still recoverable
  trees/<name>/                    local worktrees, never committed
```

New notes use distinct files, so parallel lanes can annotate the same code without editing the same bytes. `lane note confirm <id>` is the deliberate exception: it re-vouches for a drifted note by updating that note's confirmed fingerprint. Pin and unpin likewise update retention metadata. If two branches make conflicting judgments about the same note, Git should make that disagreement visible.

Pending notes live in the lane's own Git directory until the next audit. Audit promotes them, follows source-file renames, and moves superseded, unpinned missing, or unpinned over-budget notes to `.lane/attic/` rather than deleting them.

Freshness is computed for the anchored span, not the whole file:

| result | meaning |
|---|---|
| `fresh` | the anchored span is unchanged |
| `content-changed` | its implementation changed |
| `contract-changed` | its declaration changed |
| `anchor-missing` | the symbol no longer resolves |
| `unverifiable` | lane has no grammar for that anchor |

Anchors include declarations such as `fn verify`, Markdown headings such as `## Rate limiting`, component blocks such as `#script`, and `@file` for a whole file. Run `lane anchors src/auth.rs` to list the canonical values and their line ranges. A unique bare name such as `verify` is stored as its canonical value; a name shared by multiple declaration kinds is refused with the available choices. Comments and whitespace are normalized out of fingerprints.

Resolve drift with exactly one of these actions:

```bash
lane note confirm <id>
lane note replace <id> '<replacement>'
lane note retire <id>
```

Run `lane note edit <id>` for a guided terminal menu over the same actions,
including pinning or unpinning the note.

Replacement inherits the live note's path and anchor. Retire and restore move bytes unchanged between live memory and the attic; pin and unpin control eviction. For a new note, supplied text never prompts and defaults to `@file` without `-a`; omit text to opt into the interactive anchor selector and one-line prompt.

## Development

```bash
./scripts/build.sh        # release-build and install the local lane binary
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace   # unit and integration tests
./scripts/test.sh         # end to end against temporary Git repositories
./scripts/check-linux.sh  # the same gates in Linux without reflink support
```

The interactive tour creates a disposable example repository:

```bash
cargo run -p lane-tour -- start
```

Full usage and command reference: [lane.lukeed.com/usage](https://lane.lukeed.com/usage).
