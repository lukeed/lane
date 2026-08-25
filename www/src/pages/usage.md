---
layout: ../layouts/Doc.astro
title: Using lane
description: Every lane command in the order you meet them — opening a lane, leaving notes, reading what earlier lanes learned, and closing it again.
section: lane(1)
here: usage
rail:
  what-happens-to-notes-over-time: History
  working-with-agents: Agents
---

# Using lane

A lane is a throwaway worktree that costs almost nothing to open, and leaves
something behind when it closes.

```bash
$ lane new fix-login        # branch + worktree, the build cache by reference
# work, commit as usual
$ lane merge                # rebase, distill memory, fast-forward, delete
```

---

## Setup

```bash
$ cargo install --path crates/lane
$ eval "$(lane shellenv)"   # add to .zshrc: makes `lane new` cd into the lane
$ cd yourproject && lane init
$ git add .lane .gitattributes AGENTS.md
$ git commit -m "lane: context memory"
```

`lane init` prints whether your filesystem supports reflink:

```
reflink on this filesystem: yes (reflink available)
```

`yes` on APFS, btrfs, XFS with `reflink=1`, bcachefs, recent ZFS. `no` means
lanes still work as plain worktrees. Ignored files are not cloned because a full
byte copy would do the expensive work lane exists to avoid.

---

## Daily flow

### Open a lane

```bash
$ lane new fix-login
  reflink: yes (reflink available)
  1284 files cloned (612.4 MiB shared, 0 copied)
  /Users/you/yourproject/.lane/trees/fix-login
```

Tracked files come from git. Everything git ignores, at any depth, arrives by
reference — nested `node_modules`, `target`, `.env`, and whatever your own ignore
rules name. Uncommitted work is not carried. The shared bytes mean no copy, no
reinstall, and no cold rebuild.

Carry uncommitted work across too:

```bash
$ lane new spike --dirty
  carried 3 uncommitted change(s) from the parent tree
```

`--dirty` does the same as the default and also carries your uncommitted work.
Use it when you want to hand your current mess to an agent and keep working
yourself. Without reflink support, either mode leaves a plain worktree.

Opt a gitignored entry out with multi-valued configuration:

```bash
$ git config --add lane.exclude target
$ git config --add lane.exclude packages/legacy/node_modules
```

### Leave notes while you work

There are two ways in. They write to the same queue and the next audit promotes
both, so a note made either way is the same note afterwards. Use whichever
suits the moment, or both.

**Option one — a command, whenever you notice something.**

```bash
$ lane note add src/auth.rs -a "fn verify" \
>   "must stay constant-time; early return leaks token length"
```

Supplied text never prompts and an omitted `-a` defaults to `@file`. Omit the
text only when you want an interactive anchor selector and one-line note prompt.

**Option two — a trailer, in the commit you were already writing.** Install the
hooks once per repository, then never think about it again.

```bash
$ lane install hooks

$ git commit -m "make verify constant-time
>
> Why: src/auth.rs#fn verify | early return leaks token length"
```

Option one costs a separate decision at the moment you are thinking about the
commit, which is the one you forget. Option two costs a line. Neither is more
correct than the other.

The target path is required; omit `#<anchor>` to use `@file`. Record why it must
stay true, not what you did.

`-a` is the anchor — what the note is *about*:

| anchor | matches |
|---|---|
| `fn verify` | a declaration by keyword + name |
| `verify` | any declaration of that name |
| `#script`, `#style` | top-level blocks in `.svelte`/`.vue`/html |
| `## Rate limiting` | a markdown section |
| `@file` | the whole file (default) |

Discover the exact canonical values and their inclusive line ranges before
recording a note:

```bash
$ lane anchors src/auth.rs
@file       1-8
fn verify   1-4
fn refresh  6-8
```

Use `lane anchors <path> --json` for structured output. A unique bare name such
as `verify` is stored as its canonical value; a name shared by multiple
declaration kinds is refused and lists the available choices.

One note, one thought. Don't classify it, don't decide whether it's an
invariant or a rationale — the whole point is that a note costs a single
command with no taxonomy decision.

### Read what earlier lanes learned

```bash
$ lane why src/auth.rs

[fn verify]
  - 01M0B4KQTX · 2026-07-30
    callers rely on false-on-expiry, not an error
  - 01M0B9MBYB · 2026-08-14
    must stay constant-time; early return leaks token length
```

The path is omitted from headers because the command already names it. A directory
reads the whole subtree beneath it, and a bare `lane why` reads the whole store; both
keep `path#anchor` in each header so groups remain unambiguous:

```bash
$ lane why src/

[src/auth.rs#fn verify]
  - 01M0B9MBYB · 2026-08-14
    must stay constant-time; early return leaks token length

[src/db/pool.rs#@file]
  - 01M0C1PDQR · 2026-08-15
    the pool is created once per process; a second one exhausts the server
```

Matching is by whole path component, so `src` never claims `src-gen/lib.rs`.
`lane why` changes nothing and carries no branch provenance. Run `lane check` to see
whether a note's anchored code has drifted and get the id needed to resolve it.

### Close the lane

```bash
$ lane merge
  rebased onto main
  memory: +2 new; checked 8: 7 fresh, 1 content-changed, 0 contract-changed, 0 missing
  committed memory update
  fast-forwarded main
  removed lane fix-login
```

`merge` never touches the network for git. Add `--keep` to preserve the
worktree, `--squash` to land one commit, or `--base <name>` to use a different
base. Use `lane push` for a pull request — see [Pull requests](#pull-requests).

---

### Pull requests

Where trunk is protected, `lane push` rebases, audits, commits memory, and pushes the lane:

```
$ lane push
```

Turn on **Require branches to be up to date before merging**. The audit fingerprints
spans against the post-rebase tree, so a pull request merged on a stale base describes a
tree nobody has. That setting serializes two clones the way the landing lock serializes
two lanes on one machine.

The lane stays on disk until the pull request merges. `lane ls` marks it `pushed` while the
remote has its tip, then `landed` once trunk carries its landing record; `lane prune` removes it. The marker is tree content
rather than a commit, so neither a squash nor a rebase merge can hide it — `git branch -d`
refuses both even when the trees are identical. It names the lane rather than the branch,
because `fix` twice in a week is normal and the second one has landed nothing. Prune still
checks that nothing on the branch is missing from trunk, so work committed after the merge
is never discarded, and it removes nothing you are standing in.

## What happens to notes over time

Every audit re-resolves each anchor and hashes only that anchor's span,
normalized so comments and whitespace don't count:

| tier | meaning | what happens |
|---|---|---|
| `fresh` | unchanged | nothing, costs nothing |
| `content-changed` | implementation moved | flagged until you resolve it |
| `contract-changed` | the described thing changed shape | flagged until you resolve it |
| `anchor-missing` | symbol gone | evicted to `.lane/attic/` |

A renamed or moved file is followed, not evicted: `lane audit` reads git's own rename
detection and moves the notes with it. Eviction means the file or the symbol is
genuinely gone.

A drifted note stays flagged until you run `lane note confirm <id>`, replace it
with `lane note replace <id>`, or run `lane note retire <id>`. Until then,
`lane check` keeps reporting it.

Editing `#script` never stales a note on `#style`. Running a formatter stales
nothing at all.

The two drift tiers split a span at its declaration line: `fn verify(t: &str)`,
`<script>`, `## Rate limiting`. Only an anchor that has one can report
`contract-changed`. An `@file` note has no declaration — its first line is an
import or a shebang — so every change to it is `content-changed`. A heading's
declaration *is* its anchor, so changing it reports `anchor-missing`, not
`contract-changed`.

### Resolving drift

Before `lane merge`, run `lane check`. It lists every note that is not fresh with
the id you need next:

```bash
$ lane check
fresh              7
content-changed         1
contract-changed  0
anchor-missing     0
unverifiable       0

[content-changed]
~ 01M0B4KQTX  src/auth.rs#fn verify
```

Read the note and the code it points at, then take one action: `lane note confirm
<id>` when the sentence remains true; `lane note replace <id> "<rewrite>"` when
the subject is right but the sentence must change; or `lane note retire <id>`
when the constraint is gone. Replacement inherits the predecessor's path and
anchor unless you override them. At a terminal, `lane note edit <id>` presents
these actions plus pin/unpin as a guided menu. Lane never calls a model.

Any unambiguous prefix of an id works, so the ten characters above are enough;
an ambiguous one is refused and names what it matched. Add `--json` for the same
rows plus each note's body and current span, which is what an agent reads.

Replace writes a new file and moves the predecessor to the attic when audit
promotes it. A `?` has no
grammar and cannot be resolved. An `x` means the symbol is gone, so audit moves
the note to the attic instead of vouching for it.

### Budget

Each `(file, anchor)` holds at most 5 notes / 1200 characters. Audit ranks by
`pinned > touched by this lane > freshness > age` and moves the rest to
`.lane/attic/`; its location is the retirement record.
Nothing is deleted.

Keep something permanently:

```bash
$ lane note pin 01M0B4KQTX
```

Use `lane note unpin <id>` to return it to ordinary retention. Recover something:

```bash
$ lane note restore 01M0B4KQTX
```

Retire and restore move the note bytes unchanged.

---

## Working with agents

`lane init` writes the protocol into `AGENTS.md`. Symlink `CLAUDE.md` to it if
you keep both:

```
## Context memory
- Before editing a file, read `.lane/memory/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note add <path> -a <anchor> "..."`.
- Do not edit `.lane/` by hand; `lane merge` manages it.
- Detailed workflow lives in the `lane` skill; run `lane install skill` if it is absent.
```

That stub is always in context and stays short. `lane install skill` writes the
fuller version — the daily loop, the `Why:` trailer form, the anchor grammar —
to `.agents/skills/lane/SKILL.md`, loaded only when an agent is doing lane work.

Notes are plain markdown at predictable paths, so an agent finds them without
any tool integration — the reason to store them as files rather than in a
sidecar object store.

Run several agents at once:

```bash
$ lane new agent-a && lane new agent-b && lane new agent-c
$ lane ls
  agent-a    open     clean   3 pending note(s)
  agent-b    landed   dirty   1 pending note(s)
```

For a stable machine-readable inventory, orchestrators can use:

```bash
$ lane ls --json
[{"name":"agent-a","path":"/w/proj/.lane/trees/agent-a","branch":"agent-a","state":"open","dirty":false,"pending_notes":3}]
```

Before editing a file, `lane why <path> --json` provides its full note ids,
timestamps, anchors, and text without scraping the compact human view.
If the anchor is not already known, `lane anchors <path> --json` supplies the
canonical candidates without requiring the agent to guess.

Each has its own warm build cache at no disk cost. They can annotate the same
file, the same anchor, at the same time: each new finding gets its own note file,
so there is nothing to lock and nothing to conflict.

Land them in any order.

---

## Reference

The short version. See [commands](/commands) for full information.

| command | |
|---|---|
| `lane init` | scaffold, probe reflink |
| `lane new <name> [--dirty] [--base <ref>]` | create a lane |
| `lane ls` | lanes, whether they landed, dirt, pending notes |
| `lane path <name>` | print a lane's path |
| `lane anchors <file> [--json]` | list canonical anchors and line ranges |
| `lane note add <file> [-a <anchor>] [<text>]` | record a finding; omit text for interaction |
| `lane note edit <id>` | interactively confirm, replace, retire, pin, or unpin a live note |
| `lane note replace <id> [<text>]` | queue a successor, inheriting path and anchor |
| `lane note confirm <id>` | re-vouch for a resolved live note |
| `lane note retire <id>` | move a live note to the attic |
| `lane note restore <id>` | restore a retired note |
| `lane note pin <id>` | protect a live note from eviction |
| `lane note unpin <id>` | remove eviction protection |
| `lane install skill|hooks` | install the agent skill, or the commit decision capture hooks |
| `lane uninstall skill|hooks` | remove them |
| `lane why <path> [-a <anchor>]` | read the notes on a file or a directory; changes nothing |
| `lane check [--json]` | staleness report; exits 1 on missing anchors |
| `lane audit [--base <ref>]` | run the memory pass alone |
| `lane merge [--keep] [--base <ref>] [--squash] [--cd]` | rebase, audit, fast-forward, remove |
| `lane push [--base <ref>]` | rebase, audit, commit memory, and push for a pull request |
| `lane prune [--dry-run]` | remove lanes whose branch has landed in trunk |
| `lane rm <name> [--force]` | discard a lane; it stops and names uncommitted work, pending notes, or commits trunk does not have, `--force` drops them |
| `lane shellenv` | shell integration |

### Layout

```
yourproject/
  .lane/
    memory/src/auth.rs/01M0B9MBYB-must-stay-constant-time.md   the note and its baseline
    attic/                        evicted, recoverable
    trees/
      fix-login/                  the lane worktree
  AGENTS.md
  .git/lane/pending.jsonl         notes not yet promoted, per worktree
```

Lanes live in `.lane/trees/` inside the repository and are excluded through `.git/info/exclude`,
so nothing is committed.

### When things go wrong

**`trunk has diverged`** — someone else pushed. `git pull --rebase` on trunk,
then `lane merge` again.

**Rebase conflict** — resolve in the lane, `git rebase --continue`, rerun
`lane merge`. Pending notes are untouched; they're only resolved after the
rebase succeeds.

**`lane has uncommitted changes`** — commit or stash first; the rebase refuses
tracked changes either way. Untracked files are fine and need no stashing.

**`main has uncommitted changes`** — clean the named tracked files in the main
worktree; commit or stash there first. Nothing in the lane was touched.

**`another lane is landing; try again`** — another landing or trunk-side audit
holds the memory lock. It exits immediately rather than waiting; rerun the
command after that operation finishes.

**A note is simply wrong** — delete the file and commit. Nothing else references it.
