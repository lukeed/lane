---
layout: ../layouts/Doc.astro
title: Capturing decisions from commits
description: "How a Why: trailer in a commit message becomes a lane note, what it refuses to do, and what it costs."
section: lane(7)
here: memory
---

# Capturing decisions from commits

What plan 015 means in practice.

## What changes for you

Today the reason gets typed twice, in two places, at the same moment:

```
$ lane note -p src/auth.rs -a "fn verify" "early return leaks token length"
$ git commit -am "make verify constant-time"
```

The first line is the one you forget — it needs a separate decision to run a
separate command, right when you are thinking about the commit.

After:

```
$ git commit -am "make verify constant-time

Why: src/auth.rs#fn verify | early return leaks token length"
```

One place. A `post-commit` hook reads the trailer, appends it to
`.git/lane/pending.jsonl`, and the next audit promotes it. From that point it is
indistinguishable from a note made by hand:

```
$ lane why src/auth.rs

src/auth.rs#fn verify
    early return leaks token length
      01M0B9MBYB · main · 2026-08-19
```

Setup is `lane install hooks`, once. It covers every lane, because worktrees
share the hooks directory.

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
lane note -p … -a … "…"  ───────────────────────  ┘
```

Nothing downstream knows which one a note came from. Same promotion, same
fingerprinting, same budget, same review.

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

Automatic: the hook firing, parsing, validation, the append to
`.git/lane/pending.jsonl`, promotion at the next audit, and deduplication. You never
run any of it.

Not automatic: writing the `Why:` line at all. That is deliberate. A note
generated from the commit without you asking would be the git log again, which
is the thing this design refuses.

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
