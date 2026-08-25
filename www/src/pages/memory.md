---
layout: ../layouts/Doc.astro
title: Capturing Decisions
description: "How a Why: trailer in a commit message becomes a lane note, what it refuses to do, and what it costs."
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

The reason a change had to be made, written in the commit you were already
writing.

## What changes for you

Today the reason gets typed twice, in two places, at the same moment:

```
$ lane note add src/auth.rs -a "fn verify" "early return leaks token length"
$ git commit -am "make verify constant-time"
```

Both lines are true and both are worth keeping. The first is the one you
forget: it takes a separate decision to run a separate command, at the moment
you are already thinking about the commit.

### Install the hooks

Once per repository, and it is the only setup step:

```
$ lane install hooks
installed .git/hooks/post-commit
installed .git/hooks/prepare-commit-msg
```

That covers every lane, because worktrees share the hooks directory. Nothing
before it reads a `Why:` trailer — without the hooks, git carries the line in
the commit message forever and lane never sees it.

A hook is a copy, not a link. `lane check` and `lane init` name one an older
release wrote:

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

`post-commit` reads the trailer, appends it to `.git/lane/pending.jsonl`, and
the next audit promotes it. From that point it is indistinguishable from a note
made by hand:

```
$ lane why src/auth.rs

[fn verify]
  - 01M0B9MBYB · 2026-08-19
    early return leaks token length
```

### Hand the line to an agent

Writing the `Why:` line is still yours to remember. `lane install skill` gives
that part away:

```
$ lane install skill
installed /w/proj/.agents/skills/lane/SKILL.md
```

The skill carries the trailer format and is loaded only when an agent is doing
lane work, so an agent that has just changed a function writes the line into
the commit it was already making. The same hooks capture it, and the note that
comes out is the same note.

## The two sentences are different

That is the whole idea. The subject says what you did. The trailer says what
must stay true.

```
make verify constant-time                      ← what changed, once
Why: ... | early return leaks token length     ← why it must stay that way
```

Six months later the first sentence is history and the second is still
load-bearing.

## What it refuses

**No target** — `Why: refactor the parser` is rejected. You must name a file and
anchor. This is the filter that does the real work: a commit summary has no
target, so it *cannot be written* in this syntax. There is no discipline to
maintain.

**A pasted subject** — if the trailer text mostly repeats the commit subject, it
is refused with a note about what the field is for.

**A repeat** — the same text on the same anchor is one note, however many commits
or amends carry it.

In all three cases the commit still succeeds. A hook that fails your commit over
a malformed note is worse than losing the note.

## Both, not either

`lane note` does not go away. The two are producers for the same file:

```
git commit  →  Why: trailer  →  post-commit hook  ┐
                                                  ├─→  .git/lane/pending.jsonl  →  audit  →  note
lane note add … -a … "…"  ──────────────────────  ┘
```

Nothing downstream knows which one a note came from. Same promotion, same
fingerprinting, same budget.

Overlap is safe. Promotion drops a pending record whose text already matches a
live note on the same anchor, so writing a `Why:` trailer *and* running
`lane note` with the same sentence gives you one note, not two.

Reach for the trailer when the decision and the change happen together, which is
most of the time. Reach for `lane note` when they do not:

- you learned something reading code you are not changing
- the insight arrives mid-work, nowhere near a commit boundary
- the note is long, or awkward to type inside a commit message

The only rule that differs is the pasted-subject check, and only because it
cannot apply: `lane note` has no commit subject to paste.

## What it will not do

It reads one field. Not your diff, not the commit body, not commits without
trailers. There is no stream being filtered — if you never write a `Why:` line,
nothing is ever captured. That is the answer to importing the git log: the
mechanism has no input except the sentence you chose to write.

## One side benefit

Because it is a real git trailer, the reason also stays in git history.
`git log --grep`, `git interpret-trailers`, and GitHub's UI all see it. The
decision ends up recorded in two places that serve different readers — `git log`
for someone reading history, `.lane/` for someone about to edit the function.

## What is automatic and what is not

Automatic, once `lane install hooks` has run: the hook firing, parsing,
validation, the append to `.git/lane/pending.jsonl`, promotion at the next
audit, and deduplication. You never run any of it.

Not automatic: deciding there is a `Why:` line to write. That is deliberate. A
note generated from a commit that nobody chose to annotate would be the git log
again, which is the thing this design refuses. An agent with the lane skill
writes the line because it judged the constraint worth recording — the same
judgment, made by someone else.

So the only thing you supply is one line, in this shape:

```
Why: <path>[#<anchor>] | <text>
```

`<anchor>` is optional and defaults to `@file`.

You do not have to memorise it. A `prepare-commit-msg` hook puts the form in
front of you as a comment whenever an editor opens:

```
# Why: <path>#<anchor> | what must stay true (optional, records a lane note)
```

Git strips comment lines from editor commits, so it never reaches the stored
message. The hint is skipped entirely for `git commit -m`, where comments are
*not* stripped and would pollute the message.

## The honest cost

You are still deciding,at each commit, whether anything here is worth keeping. Nothing
makes that judgement for you.

What makes it likely to stick is timing rather than ergonomics: you are already
writing a commit message, so the prompt to explain yourself is already on screen.
`lane note` asks you to have that thought at a moment when nothing is asking you
for it.
