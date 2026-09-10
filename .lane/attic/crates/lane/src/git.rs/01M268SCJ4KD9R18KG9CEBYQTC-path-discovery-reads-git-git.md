---
id: 01M268SCJ4KD9R18KG9CEBYQTC
anchor: fn layout
created: 2026-09-10T18:20:26Z
norm: '1'
sig: 8d764c9bac251436
body_hash: 3626ff17a60c7fc3
raw_hash: d21fcf6e0849e1ca
lines: 64-70
supersedes: 01M0TMPTRNJVJYQ52G0CF8ZV5J
---

Path discovery reads .git, gitdir pointers, and commondir directly because macOS Git startup dominates command time. The core.bare check requires one Git config query when the common directory could belong to a primary worktree.
