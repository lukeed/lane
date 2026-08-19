# Plan 007: Let a project choose what a lane carries, and say what actually happens

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 6dc6647..HEAD -- crates/lane/src/worktree.rs crates/lane/src/cli.rs README.md`

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `6dc6647`, 2026-08-18

## Why this matters

The README's table promises this for the default mode:

| | tracked files | untracked + ignored | dirty state |
|---|---|---|---|
| default | git checks them out | cloned by reference | not carried |

It clones exactly the ten hardcoded entries in `worktree::WARM_DEFAULT` and nothing else.
A gitignored `.env` is absent from a new lane, so the lane does not run the project —
the one thing the feature exists to guarantee. `create()` takes a `warm` parameter but
`cli::new` passes `None`, so there is no way to say otherwise.

**Considered and rejected**: carrying all untracked and ignored files by default, which
is what the README claims. Unbounded — editor state, OS junk, coverage output, caches
nobody wants twice — and it duplicates secrets into a new tree unasked. `--fork` already
does that and says so.

## Current state

`crates/lane/src/worktree.rs`:

```rust
pub const WARM_DEFAULT: [&str; 10] = [
    "node_modules", "target", ".venv", "vendor", "dist",
    ".next", ".turbo", ".gradle", "build", ".cargo",
];
```

```rust
    let warm: Vec<String> =
        warm.unwrap_or_else(|| WARM_DEFAULT.iter().map(|s| s.to_string()).collect());
```

The `skip` closure in the default branch already handles a warm entry naming a **file**
rather than a directory, because the file arm tests `!warm_set.contains(top)` where `top`
is the filename for a top-level path. `.env` needs no change there — only a way into
`warm_set`. Do not rewrite the closure.

`crates/lane/src/cli.rs`, `new()` calls `wt::create(name, base, fork, None)`, and prints
each string in `created.notes`, which is how `create` already talks to the user.

`git_ok` and `try_git` in `crates/lane/src/git.rs` are the pattern for reading optional
git state; `trunk_name` is the exemplar.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline, all pass |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 4 |

## Scope

**In scope**: `crates/lane/src/worktree.rs`, `cli.rs`, `test_lane.sh`, `README.md`, `USAGE.md`.

**Out of scope**: the `--fork` branch; `cow::clone_tree` and the `skip` closure's logic;
any new config file format.

## Steps

### Step 1: Read the list from git config

Git config is the right home: per-repository, already present, no new format, inspectable
with `git config --get-all lane.warm`, and not committed by accident.

In `worktree.rs`:

```rust
/// Top-level entries to clone by reference. Entries may name a file; `.env` is the case.
pub fn warm_dirs(root: &Path, override_: Option<Vec<String>>) -> Vec<String> {
    if let Some(list) = override_ {
        if !list.is_empty() {
            return list;
        }
    }
    let configured: Vec<String> = try_git(&["config", "--get-all", "lane.warm"], Some(root))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if configured.is_empty() {
        WARM_DEFAULT.iter().map(|s| s.to_string()).collect()
    } else {
        configured
    }
}
```

Call it from `create()` in place of the `unwrap_or_else`.

**Verify**: a unit test in `worktree.rs` covering override, configured, and default.

### Step 2: A `--warm` flag for one-off overrides

In `cli.rs`, on the `New` variant:

```rust
        /// clone this top-level entry by reference; repeatable, overrides lane.warm
        #[arg(long, value_name = "DIR")]
        warm: Vec<String>,
```

Pass `Some(warm)` when non-empty. Two sources — persistent config and a per-run flag —
is the whole interface. Do not add an environment variable.

**Verify**: `lane new probe --warm .env` runs; clean up with `lane rm probe --force`.

### Step 3: Say what was carried

In `create()`, after the default branch's `clone_tree`, push to `notes`:

```rust
        notes.push(format!("warm: {}", warm.join(" ")));
```

Silence is what let the mismatch survive.

**Verify**: `lane new probe 2>&1 | grep -c '^  warm: '` → `1`.

### Step 4: Correct the docs

Replace the README table and the sentence after it. The new text must say: default checks
out tracked files and clones the warm list by reference, everything else untracked or
ignored is **not** carried; `--fork` clones the whole tree; configure with
`git config --add lane.warm .env`, or `--warm` for one run; and that `lane.warm`
**replaces** the defaults rather than extending them, so a project that sets it must name
`node_modules` itself.

Update `USAGE.md`'s "Open a lane" section and its Reference table the same way.

**Verify**: `grep -c 'untracked + ignored' README.md` → `0`;
`grep -c 'lane.warm' README.md USAGE.md` → at least `1` each.

### Step 5: Cover it

Add to `test_lane.sh` before the summary, modelled on section 2:

```bash
echo "== N. the warm list is configurable =="
setup
echo "SECRET=1" > .env
printf 'node_modules/\n.env\n' > .gitignore
git add -A && git commit -qm ignore-env

"$LANE" new plain > /dev/null 2>&1
is "an ignored file is not carried by default" \
   "$([ -f "$TMP/.lanes-repo/plain/.env" ] && echo yes || echo no)" "no"
is "the default warm dir still is" \
   "$([ -f "$TMP/.lanes-repo/plain/node_modules/pkg/blob.bin" ] && echo yes || echo no)" "yes"
"$LANE" rm plain --force > /dev/null 2>&1

git config --add lane.warm node_modules
git config --add lane.warm .env
"$LANE" new configured > /tmp/warm.out 2>&1
is "lane.warm carries the file it names" \
   "$([ -f "$TMP/.lanes-repo/configured/.env" ] && echo yes || echo no)" "yes"
is "new reports the warm list" "$(grep -c '^  warm: ' /tmp/warm.out)" "1"
```

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 4.

## Done criteria

- [ ] `./test_lane.sh` passes, baseline + 4; `cargo test` passes with the new unit test
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `grep -c 'untracked + ignored' README.md` → `0`
- [ ] `git config --add lane.warm .env` then `lane new x` yields a lane containing `.env`
- [ ] `plans/README.md` row updated

## STOP conditions

- The "tracked files not re-cloned" or "lane status clean" assertions in section 2 change
  result. Those constrain the `skip` closure, which this plan does not touch.
- A warm entry containing a path separator (`target/debug`) is needed. The closure keys on
  the top-level component only, so nested entries silently do nothing. Report it — nested
  support is a real feature and deserves its own plan.

## Maintenance notes

- `lane.warm` replaces rather than extends. Simpler to explain, but a project adding
  `.env` loses `node_modules` unless it lists it. If that grates, add a `lane.warm-add`
  key — do not merge the two lists silently, or defaults become unremovable.
- The `warm:` line in `lane new` output is load-bearing for discoverability; it must
  survive any future `--quiet`.
- Deferred: `lane init` could seed `lane.warm` from the project type — a `Cargo.toml`
  implies `target`, a `package.json` implies `node_modules`.
