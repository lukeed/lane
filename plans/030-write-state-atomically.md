# Plan 030: Make the state file impossible to half-write

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**: `git diff --stat fde27da..HEAD -- crates/lane/src/store.rs`

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `fde27da`, 2026-08-20

## Why this matters

`write_state_file` truncates the file and then writes it:

```rust
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)?;
```

`std::fs::write` opens with `O_TRUNC`, so between the truncate and the last byte the file on
disk is a prefix of valid JSON, which is not valid JSON. A crash, a full disk, or a killed
process in that window leaves a file that cannot be parsed — and `.lane/branch/<name>/state.json`
holds every fingerprint for that branch. Losing it means every note it covers reverts to its
creation fingerprint, so drift that was already resolved comes back and drift that was
recorded disappears.

The window is small and the fix is standard: write a sibling temp file, then `rename` over
the target. `rename(2)` is atomic within a filesystem — a reader sees either the old file or
the new one, never a prefix of either.

**This must not add contention**, which is the constraint that shapes the design below.

## The design

```rust
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(text.as_bytes())?;
    tmp.persist(path)?;
```

`tempfile` is already a dependency of `crates/lane`. Three properties matter, and each is a
thing to verify rather than assume:

**The temp file must be in the same directory as the target.** `rename` is only atomic
within a filesystem; a temp in `/tmp` crossing to the repository would be a copy, not a
rename, and would reintroduce the torn window. Use `new_in(parent)`, never `NamedTempFile::new()`.

**Unique temp names, so concurrent writers cannot collide.** `NamedTempFile` generates a
random name, so two processes writing the same state file get two temp files and two
renames. Last rename wins, atomically. That is strictly less contention than today, where
two truncating writers can interleave into a corrupt file.

**No `fsync`.** Deliberately. `rename` already gives every other process an all-or-nothing
view, which is what this plan is for. Adding `fsync` on the file and its directory would buy
durability against power loss and cost a disk flush on every audit — a real slowdown for a
command that runs constantly, to solve a problem nobody has reported. If durability against
power loss is ever wanted it is a separate, measured decision.

**The no-op short-circuit stays exactly as it is.** The `old == text` early return is what
keeps a repeat audit from writing at all, which plans 024 and 025 both depend on and both
test. It must run before any temp file is created — otherwise every audit creates and
renames a file, which is exactly the added contention this plan is required to avoid.

## Current state

`crates/lane/src/store.rs` — `write_state_file`, quoted above. Its callers are
`save_state` and `roll_up`.

Also in that file and deliberately **out of scope**:

- `append_log` opens with `.append(true)` and writes one line. Small appends to a file
  opened `O_APPEND` are atomic on POSIX, and the log is union-merged, so a torn line is a
  problem this plan does not have.
- `evict` uses `std::fs::rename`, already atomic.
- `note::write` creates a new file. A torn write there produces one bad new note rather than
  destroying existing data, and notes are never rewritten. It is a smaller problem with a
  different shape; do not fold it in.

`crates/lane/Cargo.toml` already lists `tempfile = "3.27.0"` under `[dependencies]`, so no
manifest change is needed. Confirm that before writing code.

Conventions: comments only where the reason is not obvious, one line. Commit subjects are
Conventional Commits, under 28 characters, describing WHAT; the reason goes in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | 81 + your new tests |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./scripts/test.sh` | `failed: 0`, 128, unchanged |
| Linux gates | `./scripts/check-linux.sh` | exit 0 |

`./scripts/test.sh` must stay at exactly 128. This plan changes how a file is written, not
what it contains.

Capture every exit code without a pipe (`cmd > /tmp/out 2>&1; echo $?`).

## Scope

**In scope**: `crates/lane/src/store.rs`.

**Out of scope**: `append_log`, `evict`, `note::write`, the state file's format, and
anything under `crates/lane-tour/`. Adding `fsync`. Changing `Cargo.toml`.

## Steps

### Step 1: Write through a temp file in the same directory

Replace the `std::fs::write` call. Keep the short-circuit and the `create_dir_all` ahead of
it, in that order.

**Verify**: `cargo test` passes at 81; `cargo clippy --all-targets` clean.

### Step 2: Prove there is no new writing

This is the constraint the plan exists under, so measure it rather than reason about it.

In a scratch repository with notes: run `lane audit --review none` twice, and confirm the
second run leaves the state file's modification time **and** content unchanged. Then confirm
no stray temp files remain anywhere under `.lane/`.

**Verify**: paste the `stat` output for both runs, and the result of
`find .lane -name '*.tmp*' -o -name '.tmp*'` — which must be empty.

### Step 3: Prove the write is atomic

A reader must never observe a partial file. Demonstrate it directly:

- confirm the temp file is created in the same directory as the target, not in `$TMPDIR` —
  the simplest proof is to make the parent directory read-only and show the write fails
  rather than silently falling back to a cross-filesystem copy
- confirm that after a successful write the target's inode has changed, which is what
  distinguishes a rename from a truncate-and-write

**Verify**: paste the inode before and after a write that changes content.

### Step 4: Cover it

Add unit tests in `store.rs`:

1. writing state twice with identical content does not touch the file — assert on modified
   time, which is the property 024 and 025 rely on
2. writing changed content replaces it completely, and the result parses
3. a pre-existing file with invalid JSON is fully replaced by a good write, leaving no
   remnant of the old bytes

**Verify**: `cargo test` → 81 + 3; `./scripts/test.sh` → 128, unchanged.

## Done criteria

- [ ] `cargo test` passes at 84; `./scripts/test.sh` passes at exactly 128
- [ ] `cargo clippy --all-targets` zero warnings; `cargo fmt --all --check` exit 0
- [ ] `./scripts/check-linux.sh` exit 0
- [ ] A repeat audit writes nothing — same mtime, same content
- [ ] No temp files remain under `.lane/` after any operation
- [ ] The temp file is created beside the target, not in `$TMPDIR`
- [ ] `Cargo.toml` unchanged
- [ ] `git diff --stat -- crates/lane-tour/ scripts/test.sh` → empty

## STOP conditions

- A repeat audit starts writing, or the state file's mtime moves when content has not
  changed. That is added contention and the plan is not worth having.
- Any temp file survives a normal operation, or appears inside a lane's worktree.
- The temp file is created outside the target's directory.
- You want to add `fsync`, or to change `Cargo.toml`.
- `./scripts/test.sh` moves off 128.

## Maintenance notes

- The invariant: **a reader of the state file sees a whole file or the previous whole file.**
  Any future code that writes it must go through the same path; a bare `fs::write` anywhere
  near it reintroduces the window silently, because the failure only appears on a crash.
- `append_log` is deliberately not atomic in the same sense and does not need to be — it is
  append-only under a union merge, which is what that rule is for.
- An append-only state file was considered as an alternative and rejected; the reasoning is
  recorded in `plans/README.md` under findings considered and rejected. In short: plan 026's
  lock had already removed the merge argument, and this change removes the durability one
  for three lines and no migration.
