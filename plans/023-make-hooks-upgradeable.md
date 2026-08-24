# Plan 023: Make an installed hook replaceable, and stop `uninstall` lying

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat c62f2d6..HEAD -- crates/lane/src/cli.rs scripts/test.sh`

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `c62f2d6`, 2026-08-19

## Why this matters

`lane uninstall hooks` reports success while doing nothing. Reproduced on this repository,
whose `post-commit` hook was installed before plan 022 changed the block text:

```
$ lane uninstall hooks
removed lane block from .git/hooks/post-commit
removed lane block from .git/hooks/prepare-commit-msg

$ cat .git/hooks/post-commit
#!/bin/sh
# lane: capture Why trailers
command -v lane >/dev/null 2>&1 && lane capture HEAD || true      <-- still there
```

The cause is at `crates/lane/src/cli.rs`:

```rust
        let existing = std::fs::read_to_string(&spec.path)?;
        if !existing.contains(spec.marker) {
            continue;
        }
        let remaining = existing.replace(spec.block, "");
        if remaining.trim() == "#!/bin/sh" {
            std::fs::remove_file(&spec.path)?;
        } else {
            std::fs::write(&spec.path, remaining)?;
        }
        println!("removed lane block from {}", spec.path.display());
```

It gates on the *marker*, which never changes, then removes by matching the *block* text
verbatim. When the block text has changed between versions, `replace` matches nothing, the
file is written back byte-identical, and success is printed anyway.

Two consequences, and the second is the serious one:

1. An installed hook can never be upgraded. `lane install hooks` sees the marker and says
   `already installed`, so an old hook body survives every future release.
2. **A command reports success while doing nothing.** A user who runs
   `lane uninstall hooks` believes lane is no longer touching their commits. It still is.

Plan 022's own guidance is wrong because of this — it tells the user to run
`lane uninstall hooks && lane install hooks` to refresh, and that sequence leaves the old
hook in place. Fixing this makes that message true.

Plan 020 already solved this exact shape for `AGENTS.md`: delimit the region lane owns, and
operate on the delimiters rather than on the content. Do the same here.

## The design

Give each hook block an end marker, so lane's region is delimited rather than guessed:

```sh
# lane: capture Why trailers
...
# lane: end
```

Then:

| state | `install` | `uninstall` |
|---|---|---|
| file absent | write it | nothing |
| delimited region present, identical | say it is current | remove the region |
| delimited region present, differs | **replace the region** | remove the region |
| legacy block, no end marker, byte-matches a known older block | replace it with the delimited form | remove it |
| marker present but the block is neither | refuse, exit 1, print the block | leave it, say why, exit 1 |

Removing the region must leave the rest of the user's hook untouched. When what remains is
only `#!/bin/sh` and whitespace, delete the file, as today.

Recognising the legacy form needs the old text kept verbatim, exactly as `PROTOCOL_V1` does
in the same file:

```rust
/// The post-commit body as shipped before end markers, recognised so it can be replaced.
/// Never edit this; it is a fingerprint of what is already in users' hooks, not content.
const POST_COMMIT_V1: &str = "# lane: capture Why trailers\n\
command -v lane >/dev/null 2>&1 && lane capture HEAD || true\n";
```

That is the exact text this repository's `post-commit` holds right now, which makes it the
fixture for Step 4.

`prepare-commit-msg` has never changed its block, so it has no legacy fingerprint yet. Give
it the same delimited treatment so the next change is free, and note in a comment that its
V1 and current text are identical today.

## Current state

`crates/lane/src/cli.rs`:

- `POST_COMMIT_MARKER`, `POST_COMMIT_BLOCK` — the block now installed, rewritten by plan 022
  to warn when `lane` is off `PATH`.
- `PREPARE_MARKER`, `PREPARE_BLOCK` — unchanged since it was introduced.
- `struct HookSpec { path, marker, block }` and `fn hook_specs(dir) -> [HookSpec; 2]`.
- `hooks_install()` — writes `format!("#!/bin/sh\n{}", spec.block)` when absent, prints
  `already installed` plus the refresh hint when the marker is present, refuses with exit 1
  when the file exists without the marker.
- `hooks_uninstall()` — quoted above.
- `write_protocol()` and `PROTOCOL_V1` in the same file are the pattern to follow. Read them
  first; this plan is the same idea applied to a different file format.

`scripts/test.sh` section 22 covers capture and the hooks. The harness gives you `setup`, `is`
and `sedi` (use `sedi`, never `sed -i`, so the suite stays portable to BSD sed).

Conventions: one-line comments, and only where the reason is not obvious; `anyhow::Result`;
tests in the existing `#[cfg(test)] mod tests` at the end of `cli.rs`. Commit subjects are
Conventional Commits, `type: verb object`, one short clause, no scope, detail in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 3 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./scripts/test.sh` | `failed: 0`, baseline + 4 |

Record both baselines before starting; at `c62f2d6` they are 65 and 104.

`./scripts/check-linux.sh` cannot run from a lane and is out of scope. Skip it and say so.

## Scope

**In scope**: `crates/lane/src/cli.rs`, `scripts/test.sh`.

**Out of scope**:
- What the hooks *do*. The post-commit body stays exactly as plan 022 left it; only its
  delimiters and the install/uninstall logic change.
- `crates/lane/src/capture.rs`, `AGENTS.md`, `crates/lane/assets/skill.md`, `README.md`,
  `USAGE.md`.
- The `AGENTS.md` protocol path. It already works; do not refactor the two into one helper
  unless nothing else changes, and if you are tempted, report it instead.

## Steps

### Step 1: Delimit the blocks

Add an end marker to both blocks and add `POST_COMMIT_V1`. Keep `POST_COMMIT_MARKER` and
`PREPARE_MARKER` byte-identical — they identify lane's region in hooks already installed,
and changing them orphans every one.

Extend `HookSpec` with the end marker and the legacy fingerprint.

**Verify**: `lane install hooks` in a scratch repo, then `sh -n .git/hooks/post-commit` and
`sh -n .git/hooks/prepare-commit-msg` both exit 0. A hook that does not parse breaks every
commit in that repository.

### Step 2: Replace the region on install, remove it on uninstall

Implement the five-row table. Both operations locate the region by its markers and splice
the surrounding text back together unchanged.

The refusal path stays as it is today: a hook file that exists with no lane marker at all is
foreign, and lane prints the block and exits 1 rather than touching it.

**Verify**, in scratch repos, one at a time:
- install twice → second reports current, file byte-identical, exit 0
- edit a line inside the region, re-install → region restored, exit 0
- write a hook containing user content *around* lane's region, uninstall → user content
  survives byte-identical, lane's region is gone
- uninstall a hook whose only content is `#!/bin/sh` plus lane's region → file deleted
- uninstall when no lane marker is present → file untouched, and no success message

### Step 3: Make the legacy hook upgradeable

A `post-commit` containing `POST_COMMIT_V1` and no end marker must be recognised and
replaced by the delimited current block, on `lane install hooks`.

**Verify**: write a scratch hook containing exactly `#!/bin/sh\n` + `POST_COMMIT_V1`, run
`lane install hooks`, and confirm the body is now the current block with both markers, exit
0. Then run `lane uninstall hooks` and confirm the file is gone.

### Step 4: Prove it on the real fixture

This repository's `.git/hooks/post-commit` is a genuine legacy hook — installed before plan
022 and never upgradeable since. It is the only unsynthesised fixture for this.

You are working in a lane, and hooks live in the shared `.git`, so **this affects the whole
repository including the main checkout**. Do not run it. Report that Step 4 is for the
reviewer, and the reviewer will run:

```bash
cat .git/hooks/post-commit          # legacy body, no end marker
lane install hooks                  # expect: upgraded
cat .git/hooks/post-commit          # expect: current body, both markers
```

**Verify**: state clearly in your report that you did not run Step 4 and why.

### Step 5: Cover it

Add unit tests to the existing `mod tests` in `cli.rs` for the classification: delimited and
identical, delimited and differing, legacy exact, foreign. Four tests.

Add to `scripts/test.sh` in or after section 22:

```bash
echo "== N. hooks can be upgraded and really removed =="
setup
"$LANE" install hooks > /dev/null
printf '#!/bin/sh\n# lane: capture Why trailers\ncommand -v lane >/dev/null 2>&1 && lane capture HEAD || true\n' > .git/hooks/post-commit
"$LANE" install hooks > /dev/null 2>&1
is "a legacy hook is upgraded" \
   "$(grep -c 'not on PATH' .git/hooks/post-commit)" "1"
printf '#!/bin/sh\necho mine\n# lane: capture Why trailers\ncommand -v lane >/dev/null 2>&1 && lane capture HEAD || true\n' > .git/hooks/post-commit
"$LANE" uninstall hooks > /dev/null 2>&1
is "uninstall keeps the user's own lines" \
   "$(grep -c 'echo mine' .git/hooks/post-commit)" "1"
is "and really removes lane's block" \
   "$(grep -c 'lane capture HEAD' .git/hooks/post-commit)" "0"
"$LANE" install hooks > /dev/null 2>&1
"$LANE" uninstall hooks > /dev/null 2>&1
is "a hook that was only lane's is deleted" \
   "$([ -f .git/hooks/post-commit ] && echo yes || echo no)" "no"
```

Confirm each fails against the pre-Step-2 binary.

**Verify**: `./scripts/test.sh` → `failed: 0`, baseline + 4.

## Done criteria

- [ ] `cargo test` passes, baseline + 3; `./scripts/test.sh` passes, baseline + 4
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `sh -n` exits 0 on both generated hooks
- [ ] `lane uninstall hooks` never prints success without having changed the file
- [ ] User content around lane's region survives an uninstall byte-identical
- [ ] A legacy `post-commit` is upgraded by `lane install hooks`
- [ ] `git diff --stat -- crates/lane/src/capture.rs crates/lane/assets/skill.md AGENTS.md` → empty

## STOP conditions

- A generated hook fails `sh -n`. Every commit in the repository runs it.
- `uninstall` would delete or alter any line the user wrote outside lane's region.
- Changing `POST_COMMIT_MARKER` or `PREPARE_MARKER` looks necessary. It is not, and it
  orphans every hook already installed.
- You are about to run Step 4. Hooks are shared through `.git` with the main checkout.

## Maintenance notes

- Three surfaces lane writes into now use the same idea: delimited regions it owns, plus a
  fingerprint of each previous version so an upgrade is recognisable. `AGENTS.md` has
  `PROTOCOL_V1`, hooks now have `POST_COMMIT_V1`. The skill instead compares whole bytes and
  refuses when edited, which is right for a file lane owns entirely.
- Every future edit to a hook block needs a new `*_V2` fingerprint and the old one kept.
  Deleting an old fingerprint strands every hook still carrying it.
- The bug this plan fixes was predicted in plan 022's own maintenance note and shipped
  anyway, because the note described a future risk rather than a present defect. When a
  maintenance note says "any future edit strands the old text", check whether an edit in the
  same change has already done so.
