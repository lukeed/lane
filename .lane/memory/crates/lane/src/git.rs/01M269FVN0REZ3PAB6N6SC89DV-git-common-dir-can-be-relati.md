---
id: 01M269FVN0REZ3PAB6N6SC89DV
anchor: fn primary_worktree
created: 2026-09-10T18:32:42Z
norm: '1'
sig: 39f8121dda410eb6
body_hash: 105b0ad86172d2c6
raw_hash: a9cc464364efc7b8
lines: 104-130
---

GIT_COMMON_DIR can be relative to the invoking worktree. The primary config query runs from its candidate parent, so it must use the resolved common directory and clear inherited worktree paths to read the correct config.
