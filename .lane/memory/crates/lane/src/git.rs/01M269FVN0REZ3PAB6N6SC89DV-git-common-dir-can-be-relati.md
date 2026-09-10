---
id: 01M269FVN0REZ3PAB6N6SC89DV
anchor: fn primary_worktree
created: 2026-09-10T18:32:42Z
norm: '1'
sig: 39f8121dda410eb6
body_hash: 7905f3853e717c2b
raw_hash: 2d64f3cb3514333e
vouched: 2026-09-10T18:54:09Z
lines: 104-130
---

GIT_COMMON_DIR can be relative to the invoking worktree. The primary config query runs from its candidate parent, so it must use the resolved common directory and clear inherited worktree paths to read the correct config.
