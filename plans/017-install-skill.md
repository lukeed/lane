# Plan 017: Teach agents to use lane, via `lane install skill`

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**:
> `git diff --stat 828ba60..HEAD -- crates/lane/src/cli.rs scripts/test.sh explainer.md USAGE.md`

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: plans/018-pending-out-of-the-worktree.md
- **Category**: dx
- **Planned at**: commit `bcb9a8b`, 2026-08-20; amended at `828ba60`, 2026-08-19

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

## The skill's prose is supplied, not specified

`crates/lane/assets/skill.md` is **already written and already committed on the lane
branch you are working in**. It is the deliverable of this plan and a spec cannot stand in
for it, so it was authored rather than described.

**Do not rewrite, reword, trim or extend it.** Read it, so you know what the rest of the
plan is wiring up, and leave the bytes alone. If you believe it is wrong, that is a STOP
condition — report what you think is wrong and stop.

For reference, it covers: the daily loop; `lane why` before editing; the `Why:` trailer
and `lane note` as the two ways to record; the rule that a note states what must stay true
rather than what changed; the anchor grammar; and what not to do. It points at
`lane --help` for the reference material — deliberately not at `USAGE.md`, which does not
exist in the repositories this skill installs into.

## Current state

- `crates/lane/src/cli.rs` — the `Hooks` clap variant with `Install`/`Uninstall`
  subcommands, `hooks_dir()`, `POST_COMMIT_MARKER`, `PREPARE_MARKER`, and the install
  routine that refuses to touch a foreign hook.
- `crates/lane/src/cli.rs` — `PROTOCOL`, the three lines `init()` appends to `AGENTS.md`.
- `crates/lane/Cargo.toml` — no `assets/` directory exists yet.
- `scripts/test.sh` — section 22 exercises capture; it calls `lane hooks install` and will
  need the new spelling.
- Four other places name the old spelling and all are in scope: `cli.rs:344` (the closing
  line `init()` prints), `README.md:148`, `USAGE.md:80` and its Reference rows at 250-251,
  and `explainer.md:37`.

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
| End to end | `./scripts/test.sh` | `failed: 0`, baseline + 5 |
| Linux gates | `./scripts/check-linux.sh` | all pass |

Record both counts before you start and express your result as a delta. At `828ba60` they
are 53 and 86, but plan 018 lands ahead of this one and adds an assertion.

## Scope

**In scope**: `crates/lane/src/cli.rs`, `crates/lane/assets/skill.md` (new), `scripts/test.sh`,
`README.md`, `USAGE.md`, `explainer.md`.

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

Update `scripts/test.sh` section 22's `lane hooks install` call in the same commit.

**Verify**:
- `lane install hooks` and `lane uninstall hooks` behave as `lane hooks install`/`uninstall` did
- `lane hooks install` now exits non-zero with an unrecognised-subcommand error
- `./scripts/test.sh` → `failed: 0` at the baseline count

### Step 2: Confirm the skill asset is present

`crates/lane/assets/skill.md` already exists on this branch. You are not creating it.

**Verify**:
- `head -4 crates/lane/assets/skill.md` shows YAML frontmatter with `name: lane` and a
  `description:` line
- `git log --oneline -- crates/lane/assets/skill.md` shows a commit you did not make
- `git diff --stat -- crates/lane/assets/skill.md` is empty at every step from here on

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

### Step 4: Fix `PROTOCOL`, then point it at the skill

`PROTOCOL` at `cli.rs:204` is the three lines `lane init` appends to every user's
`AGENTS.md`. Its second line is a command that does not run:

```
- Record non-obvious findings with `lane note -a <anchor> "..."`.
```

`--path` is required, so an agent following that instruction gets:

```
error: the following required arguments were not provided:
  --path <PATH>
```

Verified against a scratch repo at `828ba60`; exit code 2. Fix the line to carry `-p`, and
add a fourth line naming the skill — an agent will not invoke a skill it has not heard of,
and `AGENTS.md` is what it always sees:

```
- Record non-obvious findings with `lane note -p <path> -a <anchor> "..."`.
- Detailed workflow lives in the `lane` skill; run `lane install skill` if it is absent.
```

`USAGE.md`'s "Working with agents" section quotes those same three lines verbatim. Update
that block to match `PROTOCOL` exactly — it is the same text in two places and it is
already the reason this defect went unnoticed.

**Verify**, in a fresh scratch repo:
- `lane init` → `grep -c 'lane skill' AGENTS.md` → `1`
- `lane note -p src/x.rs -a "@file" "x"` → exit 0
- the command on the `lane note` line of `AGENTS.md`, run literally with real arguments,
  exits 0
- `diff <(grep '^- ' AGENTS.md) <(grep '^- Before editing\|^- Record non-obvious\|^- Do not edit\|^- Detailed workflow' USAGE.md)` → empty

### Step 5: Cover it

Add a new section to `scripts/test.sh` immediately before the summary, numbered one past the
last existing section. Six assertions:

```bash
echo "== N. lane install skill =="
setup
"$LANE" install skill > /tmp/skill.out 2>&1
is "the skill lands at the conventional path" \
   "$([ -f .agents/skills/lane/SKILL.md ] && echo yes || echo no)" "yes"
is "it has frontmatter naming the skill" \
   "$(grep -c '^name: lane$' .agents/skills/lane/SKILL.md)" "1"
is "it teaches the Why trailer form" \
   "$(grep -c '^Why: src/auth.rs#fn verify' .agents/skills/lane/SKILL.md)" "1"
is "it teaches lane note with a path" \
   "$(grep -c 'lane note -p ' .agents/skills/lane/SKILL.md)" "1"
"$LANE" install skill > /tmp/skill2.out 2>&1
is "installing twice is a no-op" "$?" "0"
echo "edited by hand" >> .agents/skills/lane/SKILL.md
"$LANE" install skill > /tmp/skill3.out 2>&1
is "an edited skill is not clobbered" "$?" "1"
```

Those greps are pinned to the asset's actual content as committed — do not adjust the
asset to satisfy an assertion. If a grep returns something other than `1`, the assertion is
what is wrong; fix the assertion and say so in your report.

Confirm the whole section fails against the pre-Step-3 binary before you rely on it.

**Verify**: `./scripts/test.sh` → `failed: 0`, baseline + 6.

### Step 6: Document it

`USAGE.md`'s Reference table gains `lane install skill|hooks` and
`lane uninstall skill|hooks`, replacing the `lane hooks install` row. Its "Working with
agents" section should mention the skill as the fuller version of the `AGENTS.md` stub.
`README.md`'s command list gains a line if it lists commands.

`explainer.md:37` says "Setup is `lane hooks install`, once." — same rename.

**Verify**: `grep -rn 'lane hooks' README.md USAGE.md explainer.md` → no matches.

## Done criteria

- [ ] `cargo test` passes, baseline + 2; `./scripts/test.sh` passes, baseline + 6
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] `./scripts/check-linux.sh` passes
- [ ] `lane install skill` writes `.agents/skills/lane/SKILL.md`; a second run is a no-op;
      an edited file is refused
- [ ] `lane hooks install` no longer exists
- [ ] `grep -rn 'hooks install' crates/lane/src/ README.md USAGE.md explainer.md scripts/test.sh`
      → no matches
- [ ] `git diff --stat main -- crates/lane/assets/skill.md` → empty; the asset is untouched
- [ ] the `lane note` line in a freshly-initialised `AGENTS.md` runs and exits 0
- [ ] `plans/README.md` row updated

## STOP conditions

- You conclude that `crates/lane/assets/skill.md` needs an edit for any reason, including
  a factual error or a command that does not match the surface you built in Step 1. Report
  it; do not fix it. Its prose is not yours to change.
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
