# Plan 020: Let `lane init` repair a protocol it wrote earlier

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. The reviewer who dispatched you maintains `plans/README.md`; do not
> edit it.
>
> **Drift check (run first)**:
> `git diff --stat 6391059..HEAD -- crates/lane/src/cli.rs test_lane.sh`

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `6391059`, 2026-08-19

## Why this matters

`lane init` writes a three-line protocol into the user's `AGENTS.md`, and then can never
change it. The guard at `crates/lane/src/cli.rs` is:

```rust
    let agents = root.join("AGENTS.md");
    if !agents.exists() {
        std::fs::write(&agents, format!("# AGENTS\n{PROTOCOL}"))?;
    } else if !std::fs::read_to_string(&agents)?.contains("## Context memory") {
```

Once that heading exists, `init` skips forever and still reports success.

Plan 017 corrected a real defect in that text: the line told agents to run
`lane note -a <anchor> "..."`, which exits 2 because `--path` is required. Every repository
initialised before 017 still carries the broken line, and re-running `lane init` does not
repair it. **This repository is one of them** — that is the fixture for Step 4, so do not
edit `AGENTS.md` by hand at any point.

Lane writes text into several files and each has a different update story. Hooks use marker
comments and can be replaced surgically. The skill compares bytes and refuses when edited.
The protocol has nothing. This plan gives it the same treatment as the hooks.

## The design

Wrap the protocol in HTML comment markers, which render as nothing in markdown:

```markdown
<!-- lane:protocol -->
## Context memory

- Before editing a file, read `.context/-/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note -p <path> -a <anchor> "..."`.
- Do not edit `.context/` by hand; `lane done` manages it.
- Detailed workflow lives in the `lane` skill; run `lane install skill` if it is absent.
<!-- /lane:protocol -->
```

`init` then has five cases:

| state of `AGENTS.md` | action |
|---|---|
| absent | write it, markers included |
| markers present, region byte-identical | nothing, say so |
| markers present, region differs | replace the region between the markers |
| no markers, but a `## Context memory` section that byte-matches a known older protocol | replace that section with the marked block, and say it was upgraded |
| no markers, and the section does not match a known older protocol | **do not touch the file**; print the block, tell the user to replace it, exit 1 |

The last row matters most. A user who edited their protocol deliberately must never have
that silently overwritten. This mirrors `hooks_install`, which refuses a foreign hook and
prints the block instead — read it at `crates/lane/src/cli.rs`, it is the pattern to copy.

Recognising the old text needs the old text kept verbatim:

```rust
/// The protocol as shipped before markers, recognised so an upgrade can replace it.
/// Never edit this; it is a fingerprint of what is already in users' files, not content.
const PROTOCOL_V1: &str = "## Context memory\n\n\
- Before editing a file, read `.context/-/<path>/` if it exists, or run `lane why <path>`.\n\
- Record non-obvious findings with `lane note -a <anchor> \"...\"`.\n\
- Do not edit `.context/` by hand; `lane done` manages it.\n";
```

That is the exact text this repository's `AGENTS.md` holds today, confirmed byte for byte
(245 bytes total including the `# AGENTS` header and blank line).

## Current state

- `crates/lane/src/cli.rs` — `PROTOCOL` is the current four-line block, defined near the
  `POST_COMMIT_MARKER` constants. `init()` contains the guard quoted above.
- `crates/lane/src/cli.rs` — `hooks_install()` is the exemplar: it checks for its marker,
  refuses when the file exists without it, prints the block to stderr and returns `Ok(1)`.
- `crates/lane/src/cli.rs` — `append_line()` is used for `.gitattributes`. It is not
  suitable here and must not be reused for the protocol; a marked region is replaced, not
  appended.
- `test_lane.sh` — section 12 asserts `AGENTS.md` has the protocol
  (`grep -c 'Context memory' AGENTS.md` → `1`). The harness gives you `setup`, `is`, `ok`,
  `bad` and `sedi` (use `sedi`, not `sed -i`, so the suite stays portable to BSD sed).

Conventions: one-line comments, and only where the reason is not obvious; `anyhow::Result`;
tests in `#[cfg(test)] mod tests` at file end — `cli.rs` already has one. Commit subjects
are Conventional Commits, `type: verb object`, one short clause, no scope; put the detail
in the body.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 3 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 4 |
| Linux gates | `./scripts/check-linux.sh` | **cannot run from a lane** — see below |

Record both baselines before starting; at `6391059` they are 55 and 94.

`./scripts/check-linux.sh` copies the repo into a container, and a linked worktree's `.git`
is a file pointing outside the mount, so it fails from a lane for reasons unrelated to your
change. Do not modify it and do not try to make it work. Skip it, say you skipped it, and
the reviewer will run it from the main checkout.

## Scope

**In scope**: `crates/lane/src/cli.rs`, `test_lane.sh`.

**Out of scope**:
- `AGENTS.md` in this repository. It is the Step 4 fixture. Do not edit it by hand, at any
  point, for any reason.
- The skill, the hooks, `.gitattributes`, and everything else `init` writes.
- Adding a new command such as `lane install agents`. The markers make that easy later;
  this plan does not add surface.
- `README.md` and `USAGE.md`.

## Steps

### Step 1: Mark the protocol

Add the markers to `PROTOCOL` and add `PROTOCOL_V1` beside it, exactly as given in "The
design". Keep `PROTOCOL`'s four bullet lines byte-identical to what they are now — this
step changes only the wrapper.

**Verify**: `cargo build` succeeds; `lane init` in a fresh scratch repo produces an
`AGENTS.md` containing both marker comments and the four bullets.

### Step 2: Replace a marked region, and recognise the legacy one

Rewrite the `AGENTS.md` branch of `init()` to implement the five-case table. Extract it into
its own function rather than growing `init()`:

```rust
fn write_protocol(agents: &Path) -> Result<i32>
```

returning the exit code so `init` can propagate a refusal. For the legacy case, locate the
`## Context memory` heading, take everything from it to the next line beginning `## ` or to
end of file, and compare it trimmed against `PROTOCOL_V1` trimmed. Equal means lane wrote
it and nobody touched it, so replacing it is safe. Anything else is the user's text.

When you refuse, print to stderr what the user must do, and return `Ok(1)`. When you
upgrade, print that the protocol was upgraded. When nothing changes, say it is current.

**Verify** in scratch repos, one case at a time:
- no `AGENTS.md` → created with markers, exit 0
- run `lane init` twice → second says it is current, file byte-identical, exit 0
- hand-edit a line *inside* the markers, re-run → region restored, exit 0
- write a legacy `AGENTS.md` containing exactly `PROTOCOL_V1`, re-run → upgraded to the
  marked block, exit 0
- write an `AGENTS.md` with a `## Context memory` section that differs from `PROTOCOL_V1`
  → file unchanged byte for byte, exit 1, stderr explains

### Step 3: Cover it

Add unit tests to `cli.rs`'s existing `mod tests` for the classification logic — given file
contents, which of the five cases applies. Three tests: marked-and-differing, legacy-exact,
legacy-modified.

Add to `test_lane.sh`, as a new section before the summary, numbered one past the last:

```bash
echo "== N. init repairs a protocol it wrote earlier =="
setup
is "init writes the marked protocol" \
   "$(grep -c 'lane:protocol' AGENTS.md)" "2"
cat > AGENTS.md <<'AGENTSEOF'
# AGENTS

## Context memory

- Before editing a file, read `.context/-/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note -a <anchor> "..."`.
- Do not edit `.context/` by hand; `lane done` manages it.
AGENTSEOF
"$LANE" init > /dev/null 2>&1
is "a legacy protocol is upgraded" \
   "$(grep -c 'lane note -p <path>' AGENTS.md)" "1"
printf '# AGENTS\n\n## Context memory\n\n- my own notes, do not touch\n' > AGENTS.md
BEFORE=$(cat AGENTS.md)
"$LANE" init > /dev/null 2>&1
is "an edited protocol is refused, not overwritten" "$(cat AGENTS.md)" "$BEFORE"
is "and the bullet the user wrote is still there" \
   "$(grep -c 'my own notes' AGENTS.md)" "1"
```

Confirm all four fail against the pre-Step-2 binary.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 4.

### Step 4: Prove it on the real fixture

This repository's `AGENTS.md` is a genuine stale protocol, written by a pre-017 `lane init`
and still carrying the broken `lane note -a` line. It is the only unsynthesised test case
available, so use it, and do not repair it by editing.

From the **main checkout** (not a lane), after building:

```bash
cargo run -- init
grep 'lane note' AGENTS.md
```

Expected: the line now reads `lane note -p <path> -a <anchor> "..."`, the file gained both
marker comments, and the command on that line runs and exits 0 when given real arguments.

If you are working in a lane and cannot reach the main checkout, **do not** run this
against your own lane's copy and report it as done. Say you could not run it, and the
reviewer will.

**Verify**: report the exact before/after of the `lane note` line.

## Done criteria

- [ ] `cargo test` passes, baseline + 3; `./test_lane.sh` passes, baseline + 4
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] All five cases from the design table behave as specified, each verified in a scratch repo
- [ ] A `## Context memory` section that differs from `PROTOCOL_V1` is left byte-identical
      and `init` exits 1
- [ ] Step 4 run against this repository's real `AGENTS.md`, before/after reported
- [ ] `crates/lane/assets/skill.md` untouched: `git diff --stat -- crates/lane/assets/skill.md` empty

## STOP conditions

- The legacy comparison would replace a section that does not byte-match `PROTOCOL_V1`.
  Silently rewriting a user's own text is the one outcome this plan exists to prevent.
- You find that `PROTOCOL_V1` as written here does not match this repository's `AGENTS.md`.
  Report the difference; do not adjust the constant to fit, and do not edit `AGENTS.md`.
- Implementing the five cases needs changes outside `cli.rs` and `test_lane.sh`.
- You are tempted to edit `AGENTS.md` in this repository for any reason. Stop and report.

## Maintenance notes

- `PROTOCOL_V1` is a fingerprint of text already sitting in users' files, not content. When
  `PROTOCOL` changes again, add `PROTOCOL_V2` and keep V1; deleting an old fingerprint
  strands every repository still carrying it.
- Three surfaces now describe lane to an agent and they must not drift: `PROTOCOL` (always
  loaded, rules only), `assets/skill.md` (loaded during lane work, workflow), and
  `USAGE.md` (documentation). `USAGE.md` quotes `PROTOCOL` verbatim — if that quote is not
  updated with it, the next person to notice will be a user whose agent ran a broken command.
- The markers make `lane install agents` a small change if a refresh command is ever wanted
  without re-running `init`.
