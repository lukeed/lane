# lane [![CI](https://github.com/lukeed/lane/actions/workflows/ci.yml/badge.svg)](https://github.com/lukeed/lane/actions/workflows/ci.yml)

> Copy-on-write Git worktrees with memory that survives them.

Lane creates isolated worktrees without leaving behind the ignored build caches that make a checkout fast. It can also attach durable notes to a file or symbol, then flag them when the relevant code changes. Lane never calls a model.

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

Add the shell wrapper to `.zshrc` or `.bashrc`:

```sh
eval "$(lane shellenv)"
```

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
$ lane install hooks
$ lane install skill
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
```

`lane new` creates a branch and worktree under `.lane/trees/`. On APFS, btrfs, and reflink-enabled XFS, ignored files are cloned by reference. Otherwise, Lane creates a normal Git worktree and skips ignored files.

Notes record what must stay true, not what a commit changed. They are stored as Markdown under `.lane/memory/` and anchored to a declaration, Markdown section, component block, or whole file. When an anchor drifts, choose one action:

```sh
$ lane note confirm <id>                  # still true
$ lane note replace <id> '<replacement>'  # needs an update
$ lane note retire <id>                   # no longer applies
```

For repositories with protected branches, use `lane push` instead of `lane merge`. After the pull request lands, run `lane prune`.

Run `lane --help` for the command list, or visit the [usage guide](https://lane.lukeed.com/usage) for the full workflow.

## Development

```sh
$ cargo fmt --all --check
$ cargo clippy --workspace --all-targets -- -D warnings
$ cargo test --workspace
$ ./scripts/test.sh
```

## License

MIT © [Luke Edwards](https://lukeed.com)
