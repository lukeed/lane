# lane [![CI](https://github.com/lukeed/lane/actions/workflows/ci.yml/badge.svg)](https://github.com/lukeed/lane/actions/workflows/ci.yml)

> Copy-on-write Git worktrees with memory that survives them.

Lane creates isolated worktrees without leaving behind the ignored build caches that make a checkout fast. It can also attach durable notes to a file or symbol, then flag them when the relevant code changes. Lane never calls a model.

> [!TIP]
> **Why another worktree tool?**
>
> When you run `git worktree add`, it creates a clean checkout but leaves behind everything Git ignores: `target/`, `node_modules/`, virtual environments, generated files, and local `.env` files. This means you must reinstall and/or build from scratch, despite already having all your caches warm on disk. On a reflink filesystem, lane clones those entries by reference. The new lane starts warm while allocating new storage only for the blocks it changes.
> 
> Lane uses `clonefile(2)` on APFS and `FICLONE` on Linux filesystems that support it, including btrfs and XFS with reflink enabled. If reflinks are unavailable, lane creates a normal worktree and does not byte-copy ignored caches. When desired, `lane new --dirty` also carries tracked edits and untracked, non-ignored files; without reflinks, those changed files are copied normally.
>
> Lanes live under `.lane/trees/` and are excluded through `.git/info/exclude`, so the worktrees themselves are never committed.
>
> Lane can also attach durable notes/memories to a file or symbol, then flag them when the relevant code changes. 
>
> Lane never calls a model.

## Install

Prebuilt binaries are available for macOS and Linux on arm64 and x86_64:

```sh
$ curl -fsSL https://lane.lukeed.com | sh
```

The installer writes to `~/.local/bin` by default. Set `LANE_INSTALL` to choose another directory, or `LANE_VERSION` to pin a release.

You may also install with Cargo:

```sh
$ cargo binstall --git https://github.com/lukeed/lane lane
# or
$ cargo install --git https://github.com/lukeed/lane
```

Lane requires Rust 1.85 or newer when building from source.

## Setup

For each new shell, you will need to install the `lane shellenv` wrapper to automatically `cd` into & out of lanes.
Or you may add the shell wrapper to `.zshrc` or `.bashrc` to auto-run: 

```sh
eval "$(lane shellenv)"
```

> While not necessary, this more convenient than manually running `cd .lane/trees/<new lane>` repeatedly.

Then initialize each repository:

```sh
$ cd yourrepo
$ lane init
$ git add .lane AGENTS.md
$ git commit -m 'initialize lane'
```

This creates the memory store, adds a short agent protocol to `AGENTS.md`, and reports whether the filesystem supports reflinks.

Optional tooling can capture `Why:` commit trailers or install the fuller agent workflow:

```sh
$ lane install hooks  # capture targeted Why: trailers from commits
$ lane install skill  # install the fuller workflow for coding agents
```

## Usage

```sh
$ lane new fix-login
$ lane why src/auth.rs
$ lane note add src/auth.rs -a 'fn verify' \
    'must stay constant-time; early return leaks token length'

# edit and commit as usual
$ lane check
$ lane merge
# or
$ lane push
```

`lane new` creates a branch and worktree under `.lane/trees/`. On APFS, btrfs, and reflink-enabled XFS, ignored files are cloned by reference. Otherwise, Lane creates a normal Git worktree and skips ignored files.

Notes record what must stay true, not what a commit changed. They are stored as Markdown under `.lane/memory/` and anchored to a declaration, Markdown section, component block, or whole file.

For repositories with protected branches, use `lane push` instead of `lane merge`. After the pull request lands, run `lane prune`.

Run `lane --help` for the command list, or visit the [usage guide](https://lane.lukeed.com/usage) for the full workflow. See [Audit](#audit) for how to review existing memories before landing.

## Memory

Memory is ordinary Markdown committed with the repository:

```text
.lane/
  memory/<path>/<ulid>-<slug>.md   live notes and confirmed fingerprints
  attic/<path>/<ulid>-<slug>.md    retired notes, still recoverable
  trees/<name>/                    local worktrees, never committed
```

New notes use distinct files, so parallel lanes can annotate the same code without editing the same bytes. `lane note confirm <id>` re-vouches for a drifted note by updating its confirmed fingerprint. Pin and unpin likewise update retention metadata. If two branches make conflicting judgments about the same note, Git makes that disagreement visible.

Notes are written directly to `.lane/memory/` without a baseline. The next audit baselines them after any rebase, follows source-file renames, and moves superseded, unpinned missing, or unpinned over-budget notes to `.lane/attic/` rather than deleting them.

Anchors include declarations such as `fn verify`, Markdown headings such as `## Rate limiting`, component blocks such as `#script`, and `@file` for a whole file. Run `lane anchors src/auth.rs` to list the canonical values and their line ranges. A unique bare name such as `verify` is stored as its canonical value; a name shared by multiple declaration kinds is refused with the available choices. Comments and whitespace are normalized out of fingerprints.

### Audit

Run `lane check` to get the status report for existing memories:

```sh
$ lane check
```

Freshness is computed for the anchored span, not the whole file:

| result | meaning |
|---|---|
| `fresh` | the anchored span is unchanged |
| `content-changed` | its implementation changed |
| `contract-changed` | its declaration changed |
| `anchor-missing` | the symbol no longer resolves |
| `unverifiable` | lane has no grammar for that anchor |

Resolve drift with exactly one of these actions:

```sh
$ lane note confirm <id>                  # still true
$ lane note replace <id> '<replacement>'  # needs an update
$ lane note retire <id>                   # no longer applies
```

Run `lane note edit <id>` for a guided terminal menu over the same actions, including pinning or unpinning the note.

Replacement inherits the live note's path and anchor. Retire and restore move bytes unchanged between live memory and the attic; pin and unpin control eviction. For a new note, supplied text never prompts and defaults to `@file` without `-a`; omit text to use the interactive anchor selector and one-line prompt.

## Development

```sh
$ ./scripts/build.sh        # release-build and install the local lane binary
$ cargo fmt --all --check
$ cargo clippy --workspace --all-targets -- -D warnings
$ cargo test --workspace
$ ./scripts/test.sh         # end to end against temporary Git repositories
$ ./scripts/check-linux.sh  # run the same gates without reflink support
```

## License

MIT © [Luke Edwards](https://lukeed.com)
