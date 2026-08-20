# Using lane

A lane is a throwaway worktree that costs almost nothing to open, and leaves
something behind when it closes.

```bash
lane new fix-login          # branch + worktree, build cache arrives by reference
# work, commit as usual
lane done                   # rebase, distill memory, fast-forward trunk, delete lane
```

---

## Setup

```bash
cargo install --path crates/lane
eval "$(lane shellenv)"          # add to .zshrc: makes `lane new` cd into the lane
cd yourproject && lane init
git add .context .gitattributes AGENTS.md .gitignore
git commit -m "lane: context memory"
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
lane new fix-login
  reflink: yes (reflink available)
  1284 files cloned (612.4 MiB shared, 0 copied)
  /Users/you/.lanes-yourproject/fix-login
```

Tracked files come from git. Everything git ignores, at any depth, arrives by
reference — nested `node_modules`, `target`, `.env`, and whatever your own ignore
rules name. Uncommitted work is not carried. The shared bytes mean no copy, no
reinstall, and no cold rebuild.

Carry uncommitted work across too:

```bash
lane new spike --dirty
  carried 3 uncommitted change(s) from the parent tree
```

`--dirty` does the same as the default and also carries your uncommitted work.
Use it when you want to hand your current mess to an agent and keep working
yourself. Without reflink support, either mode leaves a plain worktree.

Opt a gitignored entry out with multi-valued configuration:

```bash
git config --add lane.exclude target
git config --add lane.exclude packages/legacy/node_modules
```

### Leave notes while you work

```bash
lane note -p src/auth.rs -a "fn verify" \
  "must stay constant-time; early return leaks token length"
```

Install the commit hooks once to leave the same kind of note in a commit message:

```bash
lane hooks install

git commit -m "make verify constant-time

Why: src/auth.rs#fn verify | early return leaks token length"
```

The target path is required; omit `#<anchor>` to use `@file`. Record why it must
stay true, not what you did. The hook appends valid `Why:` trailers to the same
pending queue as `lane note`, and the next audit promotes them.

`-a` is the anchor — what the note is *about*:

| anchor | matches |
|---|---|
| `fn verify` | a declaration by keyword + name |
| `verify` | any declaration of that name |
| `#script`, `#style` | top-level blocks in `.svelte`/`.vue`/html |
| `## Rate limiting` | a markdown section |
| `@file` | the whole file (default) |

One note, one thought. Don't classify it, don't decide whether it's an
invariant or a rationale — the whole point is that a note costs a single
command with no taxonomy decision.

### Read what earlier lanes learned

```bash
lane why src/auth.rs

src/auth.rs#fn verify
    must stay constant-time; early return leaks token length
      01M0B9MBYB · fix-login · 2026-08-14
  ~ callers rely on false-on-expiry, not an error
      01M0B4KQTX · rate-limit · 2026-07-30   [body-drift]
```

The leading mark is freshness: blank is fresh, `~` drifted, `!` the signature
changed, `x` the anchor is gone. Reading bumps a counter that decides what
survives the budget later.

### Close the lane

```bash
lane done
  rebased onto main
  memory: +2 new, 7 fresh, 1 body-drift, 0 signature-changed, 0 missing
  reviewed 1 drifted note(s) via anthropic(claude-haiku-4-5-20251001)
  superseded    src/sync.rs#fn reconnect -> 01M0B9MFVB
  committed memory update
  fast-forwarded main
  removed lane fix-login
```

`done` never touches the network for git. Add `--keep` to preserve the
worktree, `--trunk <name>` for a non-default trunk.

---

## What happens to notes over time

Every audit re-resolves each anchor and hashes only that anchor's span,
normalized so comments and whitespace don't count:

| tier | meaning | what happens |
|---|---|---|
| `fresh` | unchanged | nothing, costs nothing |
| `body-drift` | implementation moved | sent for review |
| `signature-changed` | the described thing changed shape | sent for review |
| `anchor-missing` | symbol gone | evicted to `.context/attic/` |

A renamed or moved file is followed, not evicted: `lane audit` reads git's own rename
detection and moves the notes with it. Eviction means the file or the symbol is
genuinely gone.

Editing `#script` never stales a note on `#style`. Running a formatter stales
nothing at all.

### Review

When a span drifts, a hash can tell you it changed but not whether the note is
still true. That judgment goes to a model, once, during `done`:

```bash
export ANTHROPIC_API_KEY=...            # or
export LANE_REVIEW_CMD='claude -p'      # any CLI that reads stdin, writes stdout
```

| verdict | action |
|---|---|
| `holds` | fingerprint refreshed, note stays |
| `superseded` | a **new** note is written with a rewrite, old one to the attic |
| `contradicted` | quarantined to the attic — a confidently wrong note is worse than none |
| `unsure` | left flagged for you |

Supersede writes a new file rather than editing the old one. That's what keeps
parallel lanes conflict-free, so it holds even here.

Review is **off unless you configure it**. No key, no command, no spending, and
`lane done` still works on a plane. Only drifted notes are sent, capped by
`--review-max` (default 20).

```bash
lane audit --review none        # force off for one run
lane audit --review cmd --review-cmd './my-reviewer'
```

### Budget

Each `(file, anchor)` holds at most 5 notes / 1200 characters. Audit ranks by
`pinned > times read > touched by this lane > freshness > age` and moves the
rest to `.context/attic/` with the reason recorded in `.context/log/`.
Nothing is deleted.

Keep something permanently:

```
pinned: true        # add to the note's frontmatter
```

Recover something:

```bash
git mv .context/attic/src/auth.rs/01M0B4KQTX-*.md .context/-/src/auth.rs/
```

---

## Working with agents

`lane init` writes the protocol into `AGENTS.md`. Symlink `CLAUDE.md` to it if
you keep both:

```
## Context memory
- Before editing a file, read `.context/-/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note -a <anchor> "..."`.
- Do not edit `.context/` by hand; `lane done` manages it.
```

Notes are plain markdown at predictable paths, so an agent finds them without
any tool integration — the reason to store them as files rather than in a
sidecar object store.

Run several agents at once:

```bash
lane new agent-a && lane new agent-b && lane new agent-c
lane ls
  agent-a    agent-a    clean   3 pending note(s)
  agent-b    agent-b    dirty   1 pending note(s)
```

Each has its own warm build cache at no disk cost. They can annotate the same
file, the same anchor, at the same time: a note file is written once and never
modified, so there is nothing to lock and nothing to conflict.

Land them in any order.

---

## Reference

| command | |
|---|---|
| `lane init` | scaffold, probe reflink |
| `lane new <name> [--dirty] [--base <ref>]` | create a lane |
| `lane ls` | lanes, branch, dirt, pending notes |
| `lane path <name>` | print a lane's path |
| `lane note -p <file> -a <anchor> "<text>"` | record a finding |
| `lane hooks install` | install commit decision capture hooks |
| `lane hooks uninstall` | remove lane's hook blocks |
| `lane why <file> [-a <anchor>]` | read context, bump read counts |
| `lane check [--json]` | staleness report; exits 1 on missing anchors |
| `lane audit [--base <ref>] [--review ...]` | run the memory pass alone |
| `lane done [--keep] [--trunk <ref>]` | rebase, audit, fast-forward, remove |
| `lane rm <name> [--force]` | discard a lane; it keeps a branch holding commits trunk does not have, `--force` drops them |
| `lane shellenv` | shell integration |

### Environment

| var | |
|---|---|
| `ANTHROPIC_API_KEY` | enables review |
| `LANE_REVIEW_CMD` | reviewer command, takes precedence over the API |
| `LANE_REVIEW_MODEL` | default `claude-haiku-4-5-20251001` |
| `LANE_REVIEW` | `auto` (default), `none`, `cmd`, `anthropic` |

### Layout

```
yourproject/
  .context/
    -/src/auth.rs/01M0B9MBYB-must-stay-constant-time.md   the note, never rewritten
    attic/                        evicted, recoverable
    state/<branch>.json           fingerprints and read counts, per branch
    log/<branch>.jsonl            verdicts and evictions, per branch
  .gitattributes                  one union rule, for log/*.jsonl
  AGENTS.md
../.lanes-yourproject/
  fix-login/                      the lane worktree
```

### When things go wrong

**`trunk has diverged`** — someone else pushed. `git pull --rebase` on trunk,
then `lane done` again.

**Rebase conflict** — resolve in the lane, `git rebase --continue`, rerun
`lane done`. Pending notes are untouched; they're only resolved after the
rebase succeeds.

**`lane has uncommitted changes`** — commit or stash first; the rebase refuses
tracked changes either way. Untracked files are fine and need no stashing.

**`main has uncommitted changes`** — clean the named tracked files in the main
worktree; commit or stash there first. Nothing in the lane was touched.

**A note is wrong** — delete the file and commit. Nothing else references it.
