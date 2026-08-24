# lane push

> **Written at**: commit `cf2a474`, 2026-08-24

## Problem statement

Where trunk is protected, a lane cannot land itself. Today that is `lane done --no-merge`:
a command whose name says "finish" and whose flag says "do not finish". It stops after the
memory commit and prints the command you are expected to type next:

```
prepared fix-login; main not moved
  git push -u origin fix-login
```

A tool that tells you what to type has not finished the job. Three further problems follow
from the same place:

- **The second run is broken.** Every `done` rebases onto trunk. When trunk moved during
  review — the ordinary case — the rebase rewrites every SHA and the push you were told to
  run is rejected as a non-fast-forward, after the branch has already been rewritten locally.
- **Nothing remembers the lane is out there.** `lane ls` reads `open` for a lane pushed last
  week and for one created this morning. A week passes between opening a pull request and its
  merging, and `ls` is where you go to remember the lane exists.
- **The base is a guess.** Lane picks the first of `main`, `master`, `trunk` that exists as a
  ref. A repository whose default branch is `develop` but which still carries a stale `main`
  gets rebased onto the wrong ref, silently.

## Solution

`lane push` — a sibling verb to `lane done`, not a flag that switches it off.

```
lane push [--base <ref>]
```

It does everything `done --no-merge` did — rebase, audit, commit memory — and then sends the
branch to the remote, forcing with a lease so a rewritten history lands without destroying
anything the remote holds that you have not seen.

`lane done` keeps its job: land the lane locally. It loses `--no-merge` entirely.

Alongside it, a lane now records the ref it branched from, so both commands rebase onto a
fact rather than a guess; and `lane ls` grows a third state so a pushed lane says so.

## User stories

1. As a developer on a repository with a protected trunk, I want one command that rebases,
   audits, commits memory and sends the branch to the remote, so that finishing a lane is one
   thing I type rather than one thing I type and one thing I copy.
2. As a developer whose pull request has been in review for a week, I want `lane push` to work
   the second and third time I run it, so that a trunk that moved underneath me does not break
   the loop.
3. As a developer whose colleague pushed a fix onto my branch, I want my next push refused
   rather than their commit destroyed, so that force-pushing my own branch is not a gamble.
4. As a developer who accepted a "commit suggestion" in the GitHub review UI, I want the same
   protection, because that commit exists only on the remote.
5. As a developer with several lanes open, I want `lane ls` to tell me which are already on the
   remote, so that I know which are waiting on other people.
6. As a developer who has committed since pushing, I want `ls` to stop saying `pushed`, because
   the remote no longer has all my work.
7. As a developer on a repository whose default branch is not `main`, `master` or `trunk`, I
   want my lanes rebased onto the branch I actually work from.
8. As a developer standing in one lane, I want `lane new` to give me a lane off the repository's
   branch, exactly as it does today, so that starting unrelated work is never an accidental stack.
9. As a developer who wants a stack, I want to say so explicitly and get one.
10. As a developer who pushed a lane and then decided to land it locally, I want to be told when
    the host will leave my pull request open, so that I close it rather than discover it months later.
11. As a developer in the ordinary case — where the host will close the pull request itself — I
    want to be told nothing, because a warning that always fires is not a warning.
12. As a developer with no remote configured, I want an error that names the fix rather than a
    git failure.
13. As a developer with lanes created before this change, I want them to keep working without a
    migration step.
14. As a reader of `lane done --help`, I do not want a flag whose only purpose is to turn the
    command off.
15. As a maintainer, I want the push path exercised by a test that performs a real push, so that
    the whole reason for the command is not the one thing left unverified.

## Implementation decisions

### `push` is a new command, not a renamed flag

The behavioural delta against `done --no-merge` is exactly one thing: it pushes. Everything
else is subtraction. That is the honest accounting, and it is enough — the push absorbs the
printed hint, and `done` sheds both a flag whose only job was to disable it and a `--cd`
branch that printed the path you were already standing in. Making `--no-merge` perform a
network write instead would leave the worse name in place.

`--no-merge` is deleted rather than deprecated. Its skip of the blocking-changes check
disappears with it, because a command that never merges has nothing to check.

### Order of operations

Refuse outside a lane; refuse a dirty lane; take the landing lock; rebase onto the base;
audit; append the landing record; commit memory if it changed; resolve the remote; push.

Audit runs after the rebase, unchanged, so spans fingerprint against the tree the pull request
will actually carry. The push runs last so it carries the memory commit. A failed push leaves a
rebased, audited, committed branch — re-running `lane push` is the whole recovery.

The landing lock is retained. It no longer protects trunk's ref in this path, but the rebase
reads the base, and a base moving underneath a rebase is the thing the lock exists to prevent.

### Forcing, and why the lease

The rebase rewrites SHAs whenever the base moved, so the remote branch and the local branch
diverge and a plain push is rejected. The alternatives were: skip the rebase on a re-push,
which abandons the guarantee that audit fingerprints against the tree the pull request has;
or print a force-push command for the user to run, which is the problem this command exists
to remove.

`--force-with-lease --force-if-includes`. The lease refuses when the remote holds a commit
absent from the local remote-tracking ref; `--force-if-includes` closes the case where the
remote-tracking ref was refreshed but never incorporated. Verified: the pair succeeds on a
first push where no remote-tracking ref exists yet, so the first push needs no special case.

**`push` does not fetch first.** Counter-intuitive but decisive: fetching refreshes the
remote-tracking ref and the lease then compares a stranger's commit against itself and passes.
Not fetching leaves the ref at the value of the last push, so anyone else's commit makes the
comparison fail and the push is refused. This mirrors `sweep`, which already reads the
remote-tracking ref without fetching.

This requires git 2.30 or newer, which the project has not previously stated.

### Remote resolution

The branch's configured upstream if set, otherwise `origin`, otherwise an error naming the
fix. No `--remote` flag until someone has two remotes and a reason.

### A lane records its base

Chosen once at creation, in this order:

1. `--base <ref>` when given.
2. Otherwise, the branch the **main worktree** is on.
3. Otherwise the fallback ladder: `origin/HEAD`, then the `main`/`master`/`trunk` probe, then
   the main worktree's HEAD.

Stored in the repository's local config, in the section lane already owns for `lane.exclude`:

```
[lane "fix-login"]
	base = main
```

Local, so it is not committed and plan 032 is not reopened. In the common config, so every
worktree reads one value — not worktree-scoped config, which would give each lane its own copy
and defeat the purpose.

**A ref name, never a commit.** A frozen SHA would make the lane rebase onto the same commit
forever and never pick up the base's movement, which destroys the up-to-date guarantee the
whole design rests on.

**Rule 2 is the main worktree's branch, not the branch you are standing in.** These differ in
exactly one case — running `lane new` from inside another lane — and that is the only case where
an accidental stack could occur. Git refuses to check out one branch in two worktrees, so the
main worktree can never hold a lane branch, and rule 2 therefore cannot produce a lane-on-lane
base. Basing on where you stand was considered and rejected: it turns "start unrelated work from
inside a lane" into a silent stack. Stacking stays available through `--base`, which already
does it today.

When the recorded base is absent (lanes created before this change) or no longer resolves (the
base branch landed and was deleted), fall back to the ladder, silently. For a deleted base the
fallback is not a rescue — its commits are in trunk by then, so trunk is the correct target.

### `base`, not `trunk`

Two words had been doing one job: `new --base` and `done --trunk`. They now mean different
things — **base** is the ref this lane branched from, **trunk** is the repository default that
the ladder guesses at — so `done --trunk` is renamed `done --base`, and `push` takes `--base`.
Cheap: the old flag appears in no test and in no prose, only in the generated command reference.

Eliminating `trunk` as a concept entirely is deliberately not attempted here; see Out of scope.

### `lane ls` gains `pushed`

Reported when the branch has an upstream whose tip equals the local tip. Commit again and it
falls back to `open`, which is honest — the remote no longer has everything. Precedence is
`landed > pushed > open`.

### The orphan pull request

After `lane done` lands a lane whose branch is on a remote, warn only when the host will not
close the pull request itself.

Hosts decide this by reachability: is the pull request's head commit an ancestor of the base?
When the rebase was a no-op, the fast-forward preserves every SHA and GitHub, GitLab and
Bitbucket all mark the pull request merged unprompted. When the rebase rewrote history, or
`--squash` was used, the pushed tip is unreachable and the pull request stays open forever.

**The test is plain ancestry, deliberately not the containment probe.** The probe sees through a
rewrite by patch-id; hosts do not. Using it would stay silent in precisely the case that needs
the warning.

Best-effort enrichment: name the pull request number when a cheap `ls-remote` finds one, since
GitHub and Gitea publish PR head refs and no API token is needed beyond the push credentials.
Silent where the host publishes nothing.

### Rejected

- **A `Closes #N` trailer in the landing commit.** Discovery works — PR head refs are published —
  but `done`'s default path is a fast-forward, which creates no commit to carry a trailer. The
  trailer would have to go into the memory commit, deciding to close a pull request before the
  landing that justifies it, and after a rebase the number has to be matched against the
  pre-push tip. A wrong guess writes a stranger's pull request number into history that cannot
  be edited. The warning costs nothing and is host-agnostic.
- **Creating the pull request.** Brings a `gh` dependency and an auth story the tool has avoided
  everywhere else.
- **Duplicate landing records.** Re-pushing appends a second record. Left alone: a second push is
  a second event, the records differ by timestamp, and landing detection is set-valued, so
  nothing downstream notices.
- **Blocking a push on drifted notes.** `done` lands with drift today; a gate on one command and
  not the other is arbitrary. The audit report already prints.

## Testing decisions

Two seams, both already in use.

**`test_lane.sh`, against a bare repository on disk as `origin`.** The highest seam available:
the real binary, real git, real pushes. It covers everything user-visible — the push, the
re-push after the base moved, the lease refusing a commit that arrived on the branch from
elsewhere, `ls` reading `pushed` and falling back to `open`, `done`'s warning firing after a
rewriting rebase and staying silent after a clean one, and the recorded base being the ref
that gets rebased onto.

The remote is created by a helper the push scenarios call after `setup`, not inside `setup`
itself. `setup` is called by 32 of 34 scenarios; there is no reason for the 28 that never push
to start with a changed fixture.

Prior art: scenarios 32 through 37, which already drive prepare, merge elsewhere, and sweep.

**Unit tests in the worktree module, for base resolution.** The ladder has states that are
tedious to build in shell and trivial in a temporary repository: `origin/HEAD` present versus
absent, a repository whose default is `develop` while a stale `main` still exists, a detached
HEAD. Each test puts a throwaway repository in one state, calls the resolver once, and asserts
the ref name returned.

Prior art: the existing tests that build a repository in a temp dir, drive it with a `run`
closure, and assert on one function's return.

**No seam for the lease itself.** `--force-with-lease` is git's behaviour. The tests assert
that our command composes with it correctly, not that git implements its own flag.

Every assertion is external behaviour: what a command prints, what a ref points at, what
`git config` holds, whether a push succeeded.

## Out of scope

- **Converting `sweep`, `ls` and the landing lock to per-lane bases.** Each still uses one
  repository-level ref. The conversion is coherent and probably right — two lanes on different
  bases should not serialize on one lock — but it opens its own case table and belongs in its
  own plan.
- **Managing a stack.** Lanes will stack; lane will not restack them, order their landings, or
  set a pull request's base branch.
- **A subsumed stacked lane.** Land a stack out of order and `ls` calls the subsumed lane `open`
  forever. Detectable with the containment probe, but running it for every lane on every `ls` is
  a real cost for a case that only arises when you stack and then land out of order.
- **Creating or closing pull requests.**
- **A migration command.** The fallback covers every lane that predates this.

## Further notes

- This repository records rationale in `plans/`, not in `docs/adr/`. The trade-offs above stay
  here rather than becoming separate ADR files, consistent with how plans 003 through 032
  recorded theirs.
- `lane new` should print the base it chose. Rule 2 removes the surprise that made this
  necessary, but the value is now a fact worth seeing rather than a guess worth hiding.
- The `main`/`master`/`trunk` probe is improved rather than replaced, by consulting `origin/HEAD`
  first. It cannot be made deterministic offline: `origin/HEAD` is a clone-time cache, absent
  after `git init` and stale when a remote's default changes, and a repository with no remote has
  no default branch at all.
