# Plan 021: Stop a cloned symlink from pointing back at the parent worktree

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat 2855db6..HEAD -- crates/lane/src/cow.rs crates/lane/tests/cow.rs`

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `2855db6`, 2026-08-19

## Why this matters

Lane clones everything git ignores into a new lane, and preserves symlinks byte for byte.
For a relative link that is exactly right. For an absolute link that points back inside the
repository, the lane ends up holding a link into the **parent** worktree:

```
in the lane:
  abs-link -> /tmp/symtest2.CFgq/repo/real/tool      # the parent's path
    runs: PARENT copy
  rel-link -> ../../real/tool
    runs: LANE copy
```

Reproduced end to end. The lane reads and executes the parent's file while believing it is
its own. Lane's central promise — that two agents in two lanes cannot see each other's work
— is void along any such path, and the failure is silent: nothing errors, you just get the
wrong bytes.

This is deliberate behaviour with an unintended edge, not an oversight. `cow.rs:56` passes
`CLONE_NOFOLLOW` so `clonefile` copies the link rather than its target, and the fallback
walk calls `symlink(read_link(src))`. Both are correct for the common case.

## The design

When cloning, a symlink whose target is an absolute path **inside the source root** is
rewritten to the same relative position under the destination root. Every other symlink is
copied unchanged — relative links of any kind, and absolute links pointing anywhere else
(`/nix/store`, `/opt/homebrew`, a sibling project). Getting that distinction wrong is worse
than the bug: rewriting a link that points outside the repository would break a working
setup that has nothing to do with lane.

```rust
/// An absolute link into the source tree would still resolve to the parent worktree, so the
/// lane would read the parent's file. Links outside it are none of our business.
fn retarget(link: &Path, src_root: &Path, dst_root: &Path) -> Option<PathBuf> {
    let rel = link.strip_prefix(src_root).ok()?;
    Some(dst_root.join(rel))
}
```

Canonicalize `src_root` before comparing. On macOS a `/tmp` path and its `/private/tmp`
realpath are the same directory spelled two ways, and every comparison will miss if you skip
this — the tests below run in `tempfile::tempdir()`, which is under `/tmp`, so this is not
hypothetical.

**Both clone paths must agree.** This is the part that is easy to half-finish:

- the fallback walk in `cow.rs` recreates links in Rust and is straightforward to fix
- the reflink path never passes through Rust at all — `clone_file` hands the whole entry to
  `clonefile`/`FICLONE` with `CLONE_NOFOLLOW`, so the kernel reproduces the link

For the reflink path, either read the link before cloning and rewrite it afterwards, or walk
the cloned entry for symlinks needing a retarget and fix them in place. Either is acceptable;
what is not acceptable is fixing one path and leaving the other, because which one runs
depends on the filesystem and the bug would then reappear only on some machines.

## Current state

`crates/lane/src/cow.rs`, the fallback walk:

```rust
        if entry.file_type().is_symlink() {
            let dest = fs::read_link(entry.path())?;
            std::os::unix::fs::symlink(dest, &target)?;
            stats.links += 1;
            continue;
        }
```

`crates/lane/src/cow.rs:56`, the reflink path's comment, which explains why the kernel
reproduces the link:

```rust
    // CLONE_NOFOLLOW: clone the symlink itself, never its target.
    let rc = unsafe { libc::clonefile(c_src.as_ptr(), c_dst.as_ptr(), 1) };
```

`crates/lane/tests/cow.rs:34` is the exemplar to follow, and the test whose promise this
plan narrows:

```rust
#[test]
fn fallback_tree_is_byte_identical_and_symlinks_survive() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let out = dst.path().join("out");

    fs::create_dir_all(src.path().join("sub")).unwrap();
    fs::write(src.path().join("a.bin"), vec![7u8; 4096]).unwrap();
    std::os::unix::fs::symlink("a.bin", src.path().join("link")).unwrap();

    let stats = cow::clone_tree(src.path(), &out, &|_, _| false).unwrap();
    ...
    assert!(fs::symlink_metadata(out.join("link")).unwrap().is_symlink());
    assert_eq!(stats.links, 1);
}
```

Note the file is at `crates/lane/tests/cow.rs`, not `tests/cow.rs`. The public entry point
is `cow::clone_tree(src, dst, skip)`; `cow::probe(path)` reports whether reflink is
available, and the existing tests use it to skip assertions that only hold with reflink.

Conventions: one-line comments, and only where the reason is not obvious from the code;
`anyhow::Result` in the CLI, `CloneError` inside `cow.rs`. Commit subjects are Conventional
Commits, `type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 3 |
| Just this file | `cargo test --test cow` | all pass |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, baseline unchanged |

Record both baselines before starting; at `2855db6` they are 55 and 94.

`./scripts/check-linux.sh` cannot run from a lane — it copies the repo into a container and
a linked worktree's `.git` is a file pointing outside the mount. Do not modify it; it is out
of scope. Skip it and say so.

## Scope

**In scope**: `crates/lane/src/cow.rs`, `crates/lane/tests/cow.rs`.

**Out of scope**:
- `test_lane.sh`. The clone layer is covered by `cargo test` by design — see the comment at
  the top of `test_lane.sh` — and plan 019 is rewriting that file in parallel.
- `crates/lane/src/worktree.rs`, including where lanes are placed. That is plan 019.
- Hardlinks, sockets, device files, and directory symlinks that form cycles.
- Resolving `..` chains, or converting relative links to absolute ones. See the rule below.

## Steps

### Step 1: Retarget on the fallback path

Add `retarget` and apply it in the symlink branch of the fallback walk. A link that
`retarget` returns `None` for is copied exactly as today.

**Verify**: `cargo test --test cow` still passes, including
`fallback_tree_is_byte_identical_and_symlinks_survive` unchanged.

### Step 2: Retarget on the reflink path too

Make the reflink path reach the same result. State in your report which of the two
approaches from "The design" you took and why.

**Verify**: on a filesystem where `cow::probe` returns true, a clone of a directory
containing an absolute in-repo symlink produces a link pointing into the destination — not
the source. Assert it rather than eyeballing it.

### Step 3: Cover all three cases, on both paths

Add three tests to `crates/lane/tests/cow.rs`, following the exemplar:

1. an absolute symlink whose target is inside the source root → points inside the
   destination root after the clone, and resolves to the destination's copy of the file
2. an absolute symlink whose target is **outside** the source root → byte-identical to the
   original, `read_link` unchanged
3. a relative symlink → byte-identical to the original

Make case 1 assert on *content*, not just on the path string: write different bytes to the
source's copy and the destination's copy, then read through the link and assert you got the
destination's. A path assertion alone would pass on a link that resolves nowhere.

Where the existing tests gate on `cow::probe`, follow the same pattern so the suite still
passes on a filesystem without reflink.

Also update the comment on `fallback_tree_is_byte_identical_and_symlinks_survive` so it and
the new tests read as one intent rather than a contradiction — "symlinks survive" is now
"symlinks survive, and in-repo absolute ones follow the clone". Do not weaken its assertions.

**Verify**: `cargo test` → baseline + 3, `./test_lane.sh` unchanged at baseline.

## Done criteria

- [ ] `cargo test` passes, baseline + 3; `./test_lane.sh` passes, baseline unchanged
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] An absolute in-repo symlink resolves to the destination's copy after a clone, asserted
      by content, on both the reflink and the fallback path
- [ ] An absolute symlink outside the source root is byte-identical to the original
- [ ] A relative symlink is byte-identical to the original
- [ ] `git diff --stat -- crates/lane/src/worktree.rs test_lane.sh crates/lane/assets/skill.md AGENTS.md` → empty

## STOP conditions

- Retargeting would rewrite a symlink whose target is outside the source root. That is the
  one outcome this plan exists to prevent — stop and report rather than adding a special case.
- The reflink and fallback paths cannot be made to agree without restructuring `clone_tree`.
  Report the design problem; do not restructure it here.
- `cow::probe` returns false on your machine, so Step 2 cannot be verified. Say so plainly —
  do not claim a path you could not exercise.

## Maintenance notes

- The rule is deliberately narrow: absolute, and lexically inside the source root. Widening
  it to resolve `..` chains, or to follow relative links to their absolute targets, would
  start rewriting links that work correctly today. If a future case argues for widening it,
  that is a new plan with its own evidence.
- Plans 018, 019 and this one are three faces of one invariant: a lane's state is its own.
  018 stopped it inheriting a queue, 019 stopped its location leaking into its pointers, and
  this stops its files reaching back into the parent.
