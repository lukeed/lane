# Plan 006: Make the shell integration survive failure and survive `done`

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 6dc6647..HEAD -- crates/lane/src/cli.rs`

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `6dc6647`, 2026-08-18

## Why this matters

`eval "$(lane shellenv)"` is in the README install steps, so it is on the happy path.
Both of its interesting branches are broken, and the Rust rewrite ported them verbatim.

**`lane new` cannot fail.** The function pipes into `tail -1`, so the exit status it
tests belongs to `tail`. A failure leaves the shell running `cd "error: lane work
already exists at /Users/..."`.

**`lane done` leaves you in a deleted directory.** The function runs
`cd "$(git rev-parse --show-toplevel)"` *after* `done` removed the worktree the shell is
standing in. `git rev-parse` fails from a deleted cwd, the substitution is empty, and
`cd ""` is a no-op. The `set_current_dir` inside `done()` moves the lane process, not the
shell.

Both come from the same cause: the shell has no reliable channel for the destination.

## Current state

`crates/lane/src/cli.rs`, `shellenv()`:

```rust
    new)  shift; local p; p=$(command lane new --cd "$@" | tail -1) && cd "$p" ;;
    cd)   shift; local p; p=$(command lane path "$1") && cd "$p" ;;
    done) command lane done "${@:2}" && cd "$(git rev-parse --show-toplevel)" ;;
```

`new()` prints the path twice under `--cd` — once bolded, once bare — which is why the
function needs `tail -1`:

```rust
    println!("{}", bold(&created.path.to_string_lossy()));
    if cd {
        println!("{}", created.path.display());
    }
```

`path()` prints a path without checking the lane exists. `done()` has `root` bound early
and still valid after the lane is removed.

Conventions: one-line comments, `anyhow::Result`, `cli::run` returns an exit code.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline, all pass |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 4 |
| Shell syntax | `lane shellenv \| bash -n` and `\| zsh -n` | exit 0 |

## Scope

**In scope**: `crates/lane/src/cli.rs` (`shellenv`, `new`, `path`, `done`, and the `done`
subcommand's args), `test_lane.sh`, and the `shellenv` lines in `README.md` / `USAGE.md`
if they describe the old behaviour.

**Out of scope**: the order of operations in `done()`; `set_current_dir` before removal;
fish or PowerShell support.

## Steps

### Step 1: One `--cd` contract

> With `--cd`, stdout carries exactly one line — the directory to cd into. Everything a
> human reads goes to stderr. The exit status is the command's own.

In `new()`, route the informational lines to stderr when `cd` is set, and print the path
alone on stdout:

```rust
    // With --cd, stdout is reserved for the path so the shell can capture it without a pipe.
    let info: &mut dyn std::io::Write = if cd { &mut std::io::stderr() } else { &mut std::io::stdout() };
```

Write the reflink note, the stats line and the bolded path through `info`; print
`created.path` on stdout only under `cd`.

**Verify**: `lane new probe --cd 2>/dev/null | wc -l` → `1`. Clean up with `lane rm probe --force`.

### Step 2: Give `done` the same flag

Add `#[arg(long)] cd: bool` to the `Done` variant, thread it into `done()`, route every
informational `println!` through the same `info` writer, pass the writer to
`audit::report`, and end with `if cd { println!("{}", root.display()); }`.

`root` is bound before the lane is removed and is the directory the user should land in.
The two early error returns already write to stderr and produce no stdout, so the shell
will not cd on failure.

**Verify**: from inside a lane with a committed change, `lane done --cd 2>/dev/null | wc -l` → `1`.

### Step 3: `lane path` must refuse a lane that is not there

```rust
    if !dest.exists() {
        anyhow::bail!("no lane named {name}");
    }
```

**Verify**: `lane path nosuchlane` exits 1, prints `error: no lane named nosuchlane`, and
nothing on stdout.

### Step 4: Rewrite the function

```bash
lane() {
  case "$1" in
    new)  shift; local p; p=$(command lane new --cd "$@")  || return; cd "$p" ;;
    cd)   shift; local p; p=$(command lane path "$1")      || return; cd "$p" ;;
    done) shift; local p; p=$(command lane done --cd "$@") || return; cd "$p" ;;
    *)    command lane "$@" ;;
  esac
}
```

No pipes, so no status laundering; `return` with no argument propagates the failure. The
`done` branch takes its destination from `lane done` itself, which is what fixes the
deleted-cwd problem — the answer comes from a process not standing in the doomed directory.

**Verify**: `lane shellenv | bash -n` and `| zsh -n` both exit 0;
`lane shellenv | grep -c 'tail -1'` → `0`.

### Step 5: Cover it

Add to `test_lane.sh` before the summary. The section needs the binary's directory on
`PATH` so the function's `command lane` resolves to the binary under test:

```bash
echo "== N. shell integration survives failure and survives done =="
setup
PATH="$(dirname "$LANE"):$PATH"
eval "$(command lane shellenv)"

is "new --cd puts only the path on stdout" \
   "$(command lane new probe --cd 2>/dev/null | wc -l | tr -d ' ')" "1"
command lane rm probe --force > /dev/null 2>&1

cd "$TMP/repo"
lane new dup > /dev/null 2>&1
lane new dup > /dev/null 2>&1
is "a failed new leaves the shell where it was" "$PWD" "$TMP/.lanes-repo/dup"
cd "$TMP/repo"

lane new land > /dev/null 2>&1
echo "fn x() {}" > src/x.rs && git add -A && git commit -qm x > /dev/null
lane done > /dev/null 2>&1
is "done lands the shell in the main worktree" "$PWD" "$TMP/repo"
is "that directory exists" "$([ -d "$PWD" ] && echo yes || echo no)" "yes"
```

The second assertion is the regression: after the first `lane new dup` the shell is
inside the lane, and the failing second call must not move it.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 4.

## Done criteria

- [ ] `./test_lane.sh` passes, baseline + 4; `cargo test` unchanged and passing
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `lane shellenv | grep -cE 'tail -1|rev-parse'` → `0`
- [ ] `lane shellenv | bash -n` and `| zsh -n` both exit 0
- [ ] `lane new <existing> --cd` exits non-zero with nothing on stdout
- [ ] `plans/README.md` row updated

## STOP conditions

- An existing assertion changes result. Sections 2, 3 and 4 grep `lane new` and
  `lane done` output, but call them **without** `--cd`, so the split must not reach them.
- `bash -n` accepts the function and `zsh -n` does not, or the reverse. Both must pass;
  `local` and `$(...)` are the only non-POSIX constructs.

## Maintenance notes

- The contract from step 1 is the thing to protect: **`--cd` means stdout is the path and
  nothing else.** Any new `println!` in `new()` or `done()` must go through `info`.
- If a third command grows `--cd`, factor the writer selection into a helper.
- Deferred: fish and PowerShell users have no integration and no message saying so.
