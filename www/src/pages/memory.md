---
layout: ../layouts/Doc.astro
title: Capturing Decisions
description: "How a Why: trailer in a commit message becomes a lane note."
section: lane(7)
here: memory
rail:
  what-changes-for-you: What changes
  the-two-sentences-are-different: Two sentences
  what-it-refuses: Refuses
  both-not-either: Both
  what-it-will-not-do: Limits
  one-side-benefit: Side benefit
  what-is-automatic-and-what-is-not: Automatic
  the-honest-cost: The cost
---

# Capturing Decisions

Write the reason into the commit you were already writing.

## What changes for you

Without hooks, the reason gets typed twice:

```
$ lane note add src/auth.rs -a "fn verify" "early return leaks token length"
$ git commit -am "make verify constant-time"
```

Both lines are worth keeping. The first is the one you skip: it is a second command at the moment you are already thinking about the commit.

### Install the hooks

Once per repository:

```
$ lane install hooks
installed .git/hooks/post-commit
installed .git/hooks/prepare-commit-msg
```

Worktrees share the hooks directory, so this covers every lane. Without the hooks, git keeps the `Why:` line in the commit message and lane never sees it.

A hook is a copy, not a link. `lane check` and `lane init` name one an older release wrote:

```
$ lane check
warning: .git/hooks/post-commit is out of date; run `lane install hooks`
```

### What the hooks give you

One command where there were two:

```
$ git commit -am "make verify constant-time
>
> Why: src/auth.rs#fn verify | early return leaks token length"
```

`post-commit` reads the trailer, appends it to `.git/lane/pending.jsonl`, and the next audit promotes it. From that point it is indistinguishable from a note made by hand:

```
$ lane why src/auth.rs

[fn verify]
  - 01M0B9MBYB · 2026-08-19
    early return leaks token length
```

### Hand the line to an agent

Writing the `Why:` line is still yours to remember. `lane install skill` gives that part away:

```
$ lane install skill
installed /w/proj/.agents/skills/lane/SKILL.md
```

The skill carries the trailer format and is loaded only when an agent is doing lane work. An agent that has just changed a function writes the line into the commit it was already making. The same hooks capture it.

## The two sentences are different

The subject says what you did. The trailer says what must stay true.

```
make verify constant-time                      ← what changed, once
Why: ... | early return leaks token length     ← why it must stay that way
```

## What it refuses

**No target** — `Why: refactor the parser` is rejected. You must name a file and anchor. A commit summary has no target, so it cannot be written in this syntax.

**A pasted subject** — if the trailer text mostly repeats the commit subject, it is refused.

**A repeat** — the same text on the same anchor is one note, however many commits or amends carry it.

In all three cases the commit still succeeds. A hook that fails your commit over a malformed note is worse than losing the note.

## Both, not either

`lane note` does not go away. The two are producers for the same file:

```
git commit  →  Why: trailer  →  post-commit hook  ┐
                                                  ├─→  .git/lane/pending.jsonl  →  audit  →  note
lane note add … -a … "…"  ──────────────────────  ┘
```

Nothing downstream knows which one a note came from. Same promotion, same fingerprinting, same budget.

Overlap is safe. Promotion drops a pending record whose text already matches a live note on the same anchor, so writing a `Why:` trailer *and* running `lane note` with the same sentence gives you one note, not two.

Reach for the trailer when the decision and the change happen together. Reach for `lane note` when they do not:

- you learned something reading code you are not changing
- the insight arrives mid-work, nowhere near a commit boundary
- the note is long, or awkward to type inside a commit message

The pasted-subject check applies only to trailers. `lane note` has no commit subject to paste.

## What it will not do

It reads one field. Not your diff, not the commit body, not commits without trailers. If you never write a `Why:` line, nothing is captured. That is also why it will not import the git log: the mechanism has no input except the sentence you chose to write.

## One side benefit

Because it is a real git trailer, the reason also stays in git history. `git log --grep`, `git interpret-trailers`, and GitHub's UI all see it. `git log` for someone reading history; `.lane/` for someone about to edit the function.

## What is automatic and what is not

Automatic, once `lane install hooks` has run: the hook firing, parsing, validation, the append to `.git/lane/pending.jsonl`, promotion at the next audit, and deduplication.

Not automatic: deciding there is a `Why:` line to write. A note generated from a commit that nobody chose to annotate would be the git log again. An agent with the lane skill writes the line because it judged the constraint worth recording.

The line looks like this:

```
Why: <path>[#<anchor>] | <text>
```

`<anchor>` is optional and defaults to `@file`.

You do not have to memorise it. A `prepare-commit-msg` hook puts the form in front of you as a comment whenever an editor opens:

```
# Why: <path>#<anchor> | what must stay true (optional, records a lane note)
```

Git strips comment lines from editor commits, so it never reaches the stored message. The hint is skipped entirely for `git commit -m`, where comments are *not* stripped and would pollute the message.

## The honest cost

You still decide, at each commit, whether anything here is worth keeping.

What makes it likely to stick is timing: you are already writing a commit message, so the prompt is already on screen. `lane note` asks you to have that thought at a moment when nothing is asking you for it.
