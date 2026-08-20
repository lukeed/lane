# Plan 017: Teach agents to use lane, via `lane install skill`

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat bcb9a8b..HEAD -- crates/lane/src/cli.rs test_lane.sh`

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `bcb9a8b`, 2026-08-20

## Why this matters

`lane` is built for agents — the README's pitch is that notes are plain markdown at
predictable paths so an agent finds them without tool integration. What an agent gets
today is three lines appended to `AGENTS.md` by `lane init`:

```
- Before editing a file, read `.context/-/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note -a <anchor> "..."`.
- Do not edit `.context/` by hand; `lane done` manages it.
```

That is enough to avoid damage and not enough to be useful. It says nothing about lanes,
nothing about the anchor grammar, and nothing about the `Why:` trailer that plan 015 just
built — which is the lower-friction way to record a decision, and the one an agent writing
a commit is best placed to use.

A skill is the right shape for the rest. `AGENTS.md` is always in context and should stay
three lines; the skill is loaded when an agent is actually doing lane work and can afford
to be detailed.

This plan also fixes the command shape while only one caller exists. `lane hooks install`
puts the noun first for no reason; with a second installable arriving it should be
`lane install hooks` and `lane install skill`, with `lane uninstall <thing>` as the
counterpart.

## The design

**Command surface**

```
lane install hooks       (was: lane hooks install)
lane install skill
lane uninstall hooks     (was: lane hooks uninstall)
lane uninstall skill
```

No alias for the old spelling. `lane hooks install` shipped hours ago, is unreleased and
has no users; carrying a deprecated form costs more than the rename.

**Where the skill goes**

`.agents/skills/lane/SKILL.md`, relative to the repo root. That is the project-local skill
convention already in use on this machine, e.g.
`~/repos/testing/stripe123/.agents/skills/stripe-projects-cli/SKILL.md`. The file is
markdown with YAML frontmatter:

```markdown
---
name: lane
description: <one line, used to decide when to load it>
---

# ...
```

**Where the text lives in the repo**

`crates/lane/assets/skill.md`, pulled in with `include_str!`. Not a string literal in
`cli.rs` — the skill is prose that will be edited often, and it should read as markdown in
a diff and be lintable as a file.

**How it relates to `AGENTS.md`**

They are different surfaces and both stay. `lane init` keeps writing its three lines, and
gains one more pointing at the skill, because an agent only invokes a skill it knows about.
The skill carries everything else.

## What the skill must contain

The executor writes the prose. It must cover, in this order:

1. **The daily loop** — `lane new <name>`, work and commit as usual, `lane done`. That
   `done` rebases, audits memory, fast-forwards trunk and removes the lane.
2. **Read before editing** — `lane why <path>` before touching a file, and that reading
   bumps a counter which decides what survives the budget.
3. **Recording a decision** — the `Why:` trailer as the default, because you are already
   writing a commit message:

   ```
   Why: src/auth.rs#fn verify | early return leaks token length
   ```

   and `lane note -p <path> -a <anchor> "..."` for when the insight does not arrive at a
   commit boundary.
4. **The one rule that matters** — record what must stay true, not what you did. A commit
   subject describes a change; a note describes an invariant that outlives it. Show the
   contrast concretely.
5. **The anchor grammar** — `fn verify`, `#script`, `## Heading`, `@file`, and that the
   anchor is what the note is *about*, not where it lives.
6. **What not to do** — do not edit `.context/` by hand, do not pass `--dirty` unless you
   want the parent tree's uncommitted work, do not write a trailer that restates the
   subject (it will be refused).

Keep it under roughly 80 lines. It is instructions, not documentation; `USAGE.md` is the
documentation and the skill should link to it rather than restate it.

## Current state

- `crates/lane/src/cli.rs` — the `Hooks` clap variant with `Install`/`Uninstall`
  subcommands, `hooks_dir()`, `POST_COMMIT_MARKER`, `PREPARE_MARKER`, and the install
  routine that refuses to touch a foreign hook.
- `crates/lane/src/cli.rs` — `PROTOCOL`, the three lines `init()` appends to `AGENTS.md`.
- `crates/lane/Cargo.toml` — no `assets/` directory exists yet.
- `test_lane.sh` — section 22 exercises capture; it calls `lane hooks install` and will
  need the new spelling.

The existing hook installer is the pattern to follow for the skill: create when absent,
say so and do nothing when already present and unchanged, refuse and explain when present
and modified.

Conventions: one-line comments, `anyhow::Result`, `#[cfg(test)] mod tests` at file end.
Commit subject: Conventional Commits, one short clause, no scope.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 2 |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 5 |
| Linux gates | `./scripts/check-linux.sh` | all pass |

At `bcb9a8b` the baselines are 53 and 86.

## Scope

**In scope**: `crates/lane/src/cli.rs`, `crates/lane/assets/skill.md` (new), `test_lane.sh`,
`README.md`, `USAGE.md`.

**Out of scope**:
- The capture logic in `capture.rs`. This plan moves a command and adds a file.
- Changing what the hooks do.
- Installing the skill from `lane init`. Scaffolding a store and installing agent tooling
  are separate decisions; `init` only gains a line mentioning it.
- Supporting skill locations other than `.agents/skills/lane/`.

## Steps

### Step 1: Move the commands under `install` and `uninstall`

Replace the `Hooks` variant with two variants, each taking what to act on:

```rust
    /// install lane's agent integrations
    Install {
        #[command(subcommand)]
        what: Installable,
    },
    /// remove lane's agent integrations
    Uninstall {
        #[command(subcommand)]
        what: Installable,
    },
```

with `Installable { Hooks, Skill }`. Keep the existing hook install and uninstall bodies
exactly as they are; only the dispatch changes.

Update `test_lane.sh` section 22's `lane hooks install` call in the same commit.

**Verify**:
- `lane install hooks` and `lane uninstall hooks` behave as `lane hooks install`/`uninstall` did
- `lane hooks install` now exits non-zero with an unrecognised-subcommand error
- `./test_lane.sh` → `failed: 0` at the baseline count

### Step 2: Write the skill

Create `crates/lane/assets/skill.md` covering the six points above, with frontmatter:

```markdown
---
name: lane
description: Use lane in this repository — open a lane, read what earlier lanes learned about a file, and record decisions as Why trailers or notes.
---
```

The description is what an agent matches on to decide whether to load it, so it must name
the situations: opening a worktree, reading context before editing, recording a decision.

**Verify**: `head -4 crates/lane/assets/skill.md` shows valid frontmatter, and the file is
under 80 lines.

### Step 3: `lane install skill`

```rust
const SKILL: &str = include_str!("../assets/skill.md");
const SKILL_PATH: &str = ".agents/skills/lane/SKILL.md";
```

Install into `<repo root>/.agents/skills/lane/SKILL.md`, creating parents:

- absent → write it, print the path
- present and byte-identical → say it is already installed, change nothing, exit 0
- present and different → refuse, exit 1, and say the file was edited and that removing it
  first will let the install proceed

`lane uninstall skill` removes the file, and prunes `.agents/skills/lane/` if it is then
empty. It must not remove `.agents/` or `.agents/skills/`, which may hold other skills.

**Verify**: install twice in a scratch repo — the second run reports "already installed"
and exits 0; edit one line and the third run refuses with exit 1.

### Step 4: Point `AGENTS.md` at it

Add one line to `PROTOCOL`:

```
- Detailed workflow lives in the `lane` skill; run `lane install skill` if it is absent.
```

An agent will not invoke a skill it has not heard of, and `AGENTS.md` is what it always
sees.

**Verify**: `lane init` in a fresh repo → `grep -c 'lane skill' AGENTS.md` → `1`.

### Step 5: Cover it

Add to `test_lane.sh` before the summary. Five assertions:

```bash
echo "== N. lane install skill =="
setup
"$LANE" install skill > /tmp/skill.out 2>&1
is "the skill lands at the conventional path" \
   "$([ -f .agents/skills/lane/SKILL.md ] && echo yes || echo no)" "yes"
is "it has frontmatter naming the skill" \
   "$(grep -c '^name: lane$' .agents/skills/lane/SKILL.md)" "1"
is "it teaches the Why trailer" \
   "$(grep -c 'Why:' .agents/skills/lane/SKILL.md)" "1"
"$LANE" install skill > /tmp/skill2.out 2>&1
is "installing twice is a no-op" "$?" "0"
echo "edited by hand" >> .agents/skills/lane/SKILL.md
"$LANE" install skill > /tmp/skill3.out 2>&1
is "an edited skill is not clobbered" "$?" "1"
```

Confirm they fail against the current code before implementing.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 5.

### Step 6: Document it

`USAGE.md`'s Reference table gains `lane install skill|hooks` and
`lane uninstall skill|hooks`, replacing the `lane hooks install` row. Its "Working with
agents" section should mention the skill as the fuller version of the `AGENTS.md` stub.
`README.md`'s command list gains a line if it lists commands.

**Verify**: `grep -c 'lane hooks install' README.md USAGE.md` → `0` for both.

## Done criteria

- [ ] `cargo test` passes, baseline + 2; `./test_lane.sh` passes, baseline + 5
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` passes
- [ ] `lane install skill` writes `.agents/skills/lane/SKILL.md`; a second run is a no-op;
      an edited file is refused
- [ ] `lane hooks install` no longer exists
- [ ] `grep -rc 'hooks install' crates/lane/src/ README.md USAGE.md test_lane.sh` → `0`
- [ ] `plans/README.md` row updated

## STOP conditions

- The skill grows past roughly 80 lines. It is instructions, not documentation — link to
  `USAGE.md` rather than restating it, and report if the six points genuinely cannot fit.
- You find another consumer of `lane hooks install` outside the files named in Scope.
- `include_str!` on `assets/skill.md` does not resolve from `cli.rs`. The path is relative
  to the source file; report rather than moving the asset somewhere less obvious.
- `lane uninstall skill` would remove a directory containing skills lane did not install.

## Maintenance notes

- Two surfaces now describe lane to an agent, and they must not drift: `PROTOCOL` in
  `cli.rs` is the always-loaded three lines, `assets/skill.md` is the detail. Anything that
  changes the workflow changes the skill; only something that changes the *rules* changes
  `PROTOCOL`.
- The skill is versioned with the binary via `include_str!`, so an upgraded lane ships an
  upgraded skill — but only for repos that re-run `lane install skill`. If that becomes a
  problem, the answer is a version marker in the frontmatter and a warning on mismatch, not
  silently overwriting a file the user may have edited.
- `lane install` is now the place any future integration goes — an editor config, a CI
  snippet. Resist adding installables that are not agent-facing.
