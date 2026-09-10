---
id: 01M0TMPTRQ3T4E9ZAVYT7G7E6Y
anchor: struct RepoLayout
created: 2026-08-24T19:14:32Z
norm: '1'
sig: 096b5de33f51b8a7
body_hash: 39cb5579b9306ec0
raw_hash: 9c55aacba30f6155
vouched: 2026-09-10T19:55:39Z
lines: 48-53
---

git_dir is per-worktree and common_dir is shared, and the two must not be swapped: lane/id and lane/pending.jsonl are deliberately per-worktree, which is what stops a lane inheriting the queue its parent has not promoted. info/exclude and hooks are NOT safe to compute this way — info/ is on git's shared list and hooks obeys core.hooksPath — so both still shell out
