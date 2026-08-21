---
name: lane
description: Use lane in this repository — open a worktree with `lane new`, read what earlier lanes learned about a file before editing it, and record what must stay true as a `Why:` commit trailer or a `lane note`.
---

# lane

This repository keeps memory about its own code in `.lane/`. Notes are plain markdown,
written once and never rewritten. Read them before you edit, and leave one behind when you
learn something the next agent would otherwise rediscover.

## The loop

```bash
lane new fix-login     # branch + worktree; the build cache arrives by reference
# edit and commit as usual
lane done              # rebase, audit memory, fast-forward trunk, delete the lane
```

`lane new` prints the path. Work there, not in the parent tree. `lane done` rebases, so
commit or stash tracked changes first; untracked files are fine.

## Read before you edit

```bash
lane why src/auth.rs
```

Do this for every file you are about to change. The mark on a line is freshness: blank is
current, `~` the implementation moved, `!` the described thing changed shape, `x` the symbol
is gone, `?` the file's language has no grammar so nothing was checked. A `~` note is often
still true — the words are what matter, not the hash.

Before `lane done`, run `lane check --json`. For each `~` or `!`, read the note and its
current span, then take exactly one action:

- `lane holds <id>` when the note is still true.
- `lane note -p <path> -a <anchor> --supersedes <id> "<rewrite>"` when the subject is
  still right but the sentence is wrong.
- Delete the note file and commit when the constraint is gone.

A `?` cannot be resolved because the file has no grammar. An `x` means the symbol is gone;
let audit move that note to the attic instead of vouching for it.

Reading bumps a counter, and notes nobody reads are evicted first.

## Record what you learned

You are already writing a commit message, so use it:

```
make verify constant-time

Why: src/auth.rs#fn verify | early return leaks token length
```

The form is `Why: <path>[#<anchor>] | <text>`, in the trailer block at the end. The ` | `
and the path are required; omitting `#<anchor>` means the whole file. If nothing is
captured, run `lane install hooks` once.

When the insight does not arrive at a commit boundary:

```bash
lane note -p src/auth.rs -a "fn verify" "must stay constant-time"
```

`-p` is required.

## The one rule

**Record what must stay true, not what you did.**

The subject above already says `make verify constant-time`; the note says why that has to
hold. A subject describes a change, a note describes a constraint that outlives it. A
trailer that mostly restates its own subject is rejected rather than stored.

## Anchors

An anchor is what the note is *about*, not where it lives.

| anchor | matches |
|---|---|
| `fn verify` | a declaration, by keyword and name |
| `verify` | any declaration of that name |
| `#script`, `#style` | a top-level block in `.svelte`, `.vue`, html |
| `## Rate limiting` | a markdown section |
| `@file` | the whole file — the default |

One note, one thought. Do not classify it — a note costs one command, on purpose.

## Do not

- Rewrite anything under `.lane/` by hand; delete a note only to retire its constraint.
- Pass `--dirty` unless you want the parent tree's uncommitted work in your lane.
- Write notes about what a commit changed, or notes you have not read the file to confirm.

`lane --help` lists every command; `lane <command> --help` explains one.
