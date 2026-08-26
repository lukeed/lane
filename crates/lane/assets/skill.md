---
name: lane
description: Use lane in this repository — create isolated worktrees, read existing context before editing, and record constraints that future work must preserve.
---

# lane

This repository keeps durable context as Markdown under `.lane/`. Read it before editing a file, then record only the non-obvious constraints another agent would otherwise have to rediscover.

## Workflow

```bash
lane new fix-login     # create a branch and worktree
# edit and commit as usual
lane merge             # land on local trunk
lane merge --squash    # land on local trunk as one commit
lane push              # push the lane for a pull request
```

`lane new` prints the worktree path. Work there, not in the parent checkout.

`lane merge` rebases, audits memory, updates local trunk, and removes the lane. Commit or stash tracked changes first.

When trunk is protected, use `lane push` instead. After the pull request merges, run `lane prune`. Use `lane ls --json` for machine-readable lane state.

## Read before editing

```bash
lane why src/auth.rs
lane why src/
```

Run `lane why` for every file you intend to edit. A directory returns all notes beneath it; `--json` includes full ids, anchors, timestamps, and note text.

Run `lane check` before landing. For every stale note, take exactly one action:

```bash
lane note confirm <id>              # the note is still true
lane note replace <id> "<rewrite>"  # the constraint has changed
lane note retire <id>               # the constraint no longer applies
```

An unambiguous id prefix is enough. Use `lane note edit <id>` for the interactive menu, or the direct commands above in scripts and agent workflows.

Other lifecycle commands:

```bash
lane note restore <id>  # restore a retired note
lane note pin <id>      # protect a note from eviction
lane note unpin <id>    # remove that protection
```

## Record what must stay true

Not every file or commit needs a note. Record significant, non-obvious constraints — never a summary of the change.

Add a `Why:` trailer when the finding belongs with a commit:

```text
make verification constant-time

Why: src/auth.rs#fn verify | early return leaks token length
```

The form is `Why: <path>[#<anchor>] | <text>`. The path and ` | ` separator are required; omit the anchor to target the whole file. Run `lane install hooks` once if trailers are not being captured.

Record a finding directly when it does not belong to a commit:

```bash
lane note add src/auth.rs -a "fn verify" "must stay constant-time"
```

Supplying text is non-interactive and defaults to `@file` when `-a` is omitted. Omit the text only when you want the interactive prompt.

## Anchors

An anchor describes what a note is about:

| anchor | matches |
|---|---|
| `fn verify` | a declaration by kind and name |
| `verify` | a uniquely named declaration |
| `#script`, `#style` | a top-level component block |
| `## Rate limiting` | a Markdown section |
| `@file` | the whole file |

Run `lane anchors <path> --json` and prefer the canonical anchor it returns. Ambiguous declaration names are refused with their available choices.

## Rules

- Never edit `.lane/` by hand; use the `lane note` commands.
- Do not pass `--dirty` unless the lane should inherit the parent checkout's uncommitted work.
- Keep each note to one constraint.
- Do not record what a commit changed.

Run `lane --help` for the command list, or `lane <command> --help` for details.
