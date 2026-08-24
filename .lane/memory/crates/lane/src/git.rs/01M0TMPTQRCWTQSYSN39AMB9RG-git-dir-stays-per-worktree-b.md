---
id: 01M0TMPTQRCWTQSYSN39AMB9RG
anchor: fn layout
created: 2026-08-24T18:50:48Z
branch: perf-audit
norm: '1'
sig: 8d764c9bac251436
body_hash: 3626ff17a60c7fc3
raw_hash: d21fcf6e0849e1ca
lines: 56-62
---

git_dir stays per-worktree because lane identities and pending queues must not leak from a primary worktree into a linked lane
