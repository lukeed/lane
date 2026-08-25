# lane

Copy-on-write worktrees with memory that survives them. This glossary fixes the words the
tool uses for the things it moves: worktrees, the refs they came from, and the record of
what happened to them.

## Language

**Lane**:
A copy-on-write worktree together with the branch checked out inside it.
_Avoid_: workspace, branch — a branch is not a lane, and a lane outlives the branch it lands.

**Base**:
The ref a lane branched from and rebases onto. Chosen once when the lane is created and
recorded against it thereafter.
_Avoid_: trunk, parent, target, upstream — upstream is the remote-tracking ref, a different thing.

**Trunk**:
The repository's default branch. Only ever a fallback, used when a lane has no recorded base.
_Avoid_: main, master, mainline — the default branch is frequently none of these.

**Landing record**:
A marker in the lane's own git directory, written when a lane is prepared, holding the lane id
and the branch tip. Never committed. It records that a landing was prepared, not that one
arrived; the tip is what makes work added afterwards countable.
_Avoid_: merge marker, landing commit — it is worktree state, not history.

**Open**:
A lane whose work is not on the remote and not in its base.

**Pushed**:
A lane whose branch is on the remote at exactly its local tip. Committing again returns it
to open, because the remote no longer has everything.

**Landed**:
A marked lane whose branch its remote retired, or whose work its base already contains.
Collectable by prune once nothing has been added since the landing.
