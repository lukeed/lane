# Plan 028: An interactive tour that teaches lane by driving it

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**: `git diff --stat aea5887..HEAD -- Cargo.toml crates/`

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `aea5887`, 2026-08-20

## Why this matters

Lane is hard to evaluate from a README. The value only appears once several lanes exist at
once, a note drifts, and landings interleave — none of which a reader will set up by hand
to satisfy their curiosity.

This is a long-running program that builds a sandbox, prints its path, and then waits.
The reader opens that directory in an editor, picks a numbered option, and watches the
directory change. They never have to know a lane command to start; they finish having seen
the real ones, because every command is printed before it runs.

It is a teaching aid, not a product surface. It ships as its own binary and must not add a
single byte to `lane`.

## The content is already written

`crates/example/src/scenes.rs` **already exists and is committed on your branch**. It holds
the scene table, all narration, and every command the tour runs. It is the deliverable of
this plan and it was authored deliberately.

**Do not rewrite, reword, extend, trim, or reorder it.** Read it — you are building the
driver that sequences it, and its type definitions are the contract you implement against.
`git diff --stat -- crates/example/src/scenes.rs` must stay empty for the whole task.

If a command inside a scene turns out not to work, that is a STOP condition: report the
scene key and the failure. Do not "fix" the content to make it pass.

The types it defines:

```rust
pub enum Step {
    Say(&'static str),               // prose for the reader
    Do(&'static str),                // shell command, run in the sandbox root
    In(&'static str, &'static str),  // (subdirectory, shell command)
    Look(&'static str),              // pause; the reader inspects the directory
}

pub struct Scene {
    pub key: &'static str,
    pub title: &'static str,
    pub why: &'static str,
    pub steps: &'static [Step],
    pub records: &'static str,
}

pub const OPENING: &str;
pub const CLOSING: &str;
pub const SCENES: &[Scene];
```

## What to build

A new workspace crate, `crates/example`, producing a binary named `example`.

**It must not touch the `lane` binary.** It is a separate crate with its own dependency
list; `crates/lane/Cargo.toml` gains nothing, and `lane` must not depend on `example` nor
`example` on the `lane` library. Verify with a size comparison, below.

**Prefer zero dependencies.** Everything needed is in `std`: `std::process::Command`,
`std::io::stdin`, `std::fs`. Do not add `clap` for an argument surface this small. If you
believe a dependency is unavoidable, that is a STOP condition — report which and why.

### Command surface

```
example start [--at <path>]     build a sandbox and start the tour
example --help
```

Anything else prints usage and exits non-zero.

### The sandbox

Created at `<parent of cwd>/lane-example-<n>`, where `<n>` is the lowest integer that does
not already exist — a *sibling* of wherever the reader is, so it sits next to their project
in an editor's file tree. `--at <path>` overrides it entirely.

Print the absolute path prominently, twice: once when created, once in the menu header, so
a reader scrolling back always finds it.

Contents at creation:

- `git init`, with `user.name` and `user.email` set locally so commits work in a fresh
  environment, and `init.defaultBranch=main`
- `src/auth.rs` containing exactly:
  ```rust
  pub fn verify(token: &str) -> bool {
      parse(token).is_valid()
  }
  ```
- a `.gitignore` containing `target/`
- one commit
- then `lane init`, and a commit of what it produced

The scenes assume this starting point. Getting it wrong makes later scenes fail in ways
that look like lane bugs.

### The loop

Print `OPENING` once, then repeat: a menu, a prompt, a scene, back to the menu.

The menu lists every scene by `key` and `title`, marks the ones already played, and points
at the recommended next one. Scenes are written to run in order and depend on each other's
side effects; a reader may still pick any, so print a one-line warning when they choose one
out of order rather than refusing.

Extra options beyond the scenes, using keys that cannot collide with a scene key:

| key | |
|---|---|
| `t` | print the timeline so far |
| `d` | print the sandbox tree — `.context/`, `.lanes/`, and `git log --oneline` |
| `n` | play the recommended next scene |
| `q` | print `CLOSING`, the timeline, the sandbox path, and how to delete it |

### Playing a scene

Print the title and `why`. Then for each step:

- `Say` — the prose, wrapped and indented, visually distinct from command output
- `Do` / `In` — echo the command as `$ <command>` *before* running it, then run it through
  `sh -c` in the right directory and print its stdout and stderr indented beneath
- `Look` — print the text, then `[enter] to continue`, and wait

A command that exits non-zero is not necessarily wrong: several scenes deliberately show
lane refusing. Print the exit code when it is non-zero and carry on. Do not abort the scene.

When the scene ends, append `records` to the timeline with a wall-clock time.

### The timeline

An in-memory list of `(time, scene title, records)`. `t` renders it; `q` prints it last.
That is the "timeline of events" the tour is building — the reader should be able to look
back and see the story they just drove.

## Current state

- Workspace root `Cargo.toml` has `members = ["crates/lane"]`. Add `crates/example`.
- `crates/lane/Cargo.toml` is the shape to copy for edition and rust-version; use
  `version.workspace = true`, `edition.workspace = true`, `rust-version.workspace = true`.
- The workspace sets `[workspace.lints.clippy]`; the new crate should opt in with
  `[lints] workspace = true`, as `lane` does.
- `crates/example/src/scenes.rs` exists. You are adding `crates/example/src/main.rs` and
  `crates/example/Cargo.toml`.

Conventions: one-line comments, and only where the reason is not obvious; tests in
`#[cfg(test)] mod tests` at file end. Commit subjects are Conventional Commits,
`type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Build all | `cargo build` | both binaries |
| Unit + integration | `cargo test` | baseline, plus any you add |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| Lane's own suite | `./test_lane.sh` | `failed: 0`, baseline unchanged |
| Linux gates | `./scripts/check-linux.sh` | exit 0 |

Baselines at `aea5887`: `cargo test` 78, `./test_lane.sh` 128.

`./test_lane.sh` must be **unchanged** — this plan adds a crate, it does not alter lane.

## Scope

**In scope**: `Cargo.toml` (workspace members), `crates/example/Cargo.toml` (new),
`crates/example/src/main.rs` (new), `README.md`.

**Out of scope**:
- `crates/example/src/scenes.rs` — supplied, see above.
- Everything under `crates/lane/`. If the tour needs a lane change to work, STOP and report.
- `test_lane.sh`, `scripts/`, `USAGE.md`, `AGENTS.md`, `crates/lane/assets/skill.md`.
- Publishing, packaging, or a `cargo install` story for the tour.

## Steps

### Step 1: The crate, building nothing but itself

Create `crates/example/Cargo.toml` and a `main.rs` that compiles, and add the member to the
workspace.

**Verify**, and this is the constraint that matters most:

```sh
cargo build --release
ls -l target/release/lane target/release/example
```

Record `lane`'s release size before and after adding the crate. It must be **identical** —
if it moved at all, the crates are entangled and that is a STOP condition.

Also confirm `cargo install --path crates/lane` still installs only `lane`.

### Step 2: The sandbox

Implement `example start [--at <path>]` and the sandbox described above. At startup, check
that `lane` is on `PATH` and exit with a clear message naming the fix if it is not — a
reader running this has probably just cloned the repo.

**Verify**: run it, then in another shell inspect the printed path: `git log --oneline`
shows two commits, `.context/` exists, `AGENTS.md` exists, `src/auth.rs` has the two-line
`verify`, and `git status --porcelain` is empty.

### Step 3: The loop and the scene player

Implement the menu, the four extra keys, and the player.

**Verify**: play scene `1` and confirm every `Say` is legible prose, every command appears
as `$ ...` before its output, and `Look` waits for enter.

### Step 4: Play the whole tour, in order, and report what happens

Run every scene from `1` to the last, in the listed order, in one session. This is the real
test of this plan; the driver is easy and the sequencing is where it breaks.

For each scene, confirm the commands succeed — except where a scene is deliberately showing
a refusal, where a non-zero exit is the expected outcome and must be *visible* to the reader.

Two known-fragile ones, verify them specifically:

- scene `a` runs a `python3` one-liner that holds the landing lock and then tries to land
  into it. It should print lane's refusal. If `python3` is missing, or the lock is not
  actually contended, report exactly what happened — do not rewrite the scene.
- scene `c` deliberately makes `lane install skill` exit non-zero. That is correct
  behaviour, and the tour must not treat it as an error.

**Verify**: paste the full transcript of a complete run into your report. It is the primary
evidence for this plan.

### Step 5: Say it exists

`README.md` gains a short section — four lines at most — naming the tour, the one command
to start it, and that it builds a throwaway sandbox. Put it after "Install".

**Verify**: `grep -c 'example start' README.md` → at least `1`.

## Done criteria

- [ ] `cargo build --release` produces `example`; `lane`'s release binary size is byte-identical
      to before this plan
- [ ] `cargo test`, `cargo clippy --all-targets`, `cargo fmt --all --check` all clean
- [ ] `./test_lane.sh` passes at exactly its previous count, 128
- [ ] `./scripts/check-linux.sh` exit 0
- [ ] `example start` builds a sandbox and prints its absolute path
- [ ] Every scene from `1` to the last plays in order, transcript included in the report
- [ ] `t`, `d`, `n`, `q` all work; `q` prints CLOSING, the timeline and the sandbox path
- [ ] `git diff --stat -- crates/example/src/scenes.rs crates/lane/` → empty
- [ ] `example` has no dependencies, or the report explains why one was unavoidable

## STOP conditions

- `lane`'s release binary changes size. The tour must cost the shipped tool nothing.
- A scene's command does not work. Report the scene key and the output; do not edit
  `scenes.rs`.
- Making the tour work needs a change under `crates/lane/`.
- You want to add a dependency.
- `./test_lane.sh` count changes. This plan does not touch lane's behaviour.

## Maintenance notes

- `scenes.rs` is content and `main.rs` is a player. Keep it that way: a scene should never
  need a code change, and a driver change should never need new prose. If a future scene
  wants something the four `Step` variants cannot express, add a variant rather than
  special-casing a scene inside the driver.
- The tour shells out to `lane` on `PATH` rather than linking the library, deliberately.
  It demonstrates the commands a reader will actually type, and it cannot drift from the
  real CLI without failing loudly.
- Every scene depends on the ones before it. That is why the menu recommends an order. If
  scenes ever need to be independent, the sandbox has to be rebuildable to a known point,
  which is a larger change than it looks.
