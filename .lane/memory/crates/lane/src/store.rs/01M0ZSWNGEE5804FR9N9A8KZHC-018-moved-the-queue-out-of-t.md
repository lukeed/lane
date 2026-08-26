---
id: 01M0ZSWNGEE5804FR9N9A8KZHC
anchor: fn promote_pending
created: 2026-08-26T19:48:56Z
norm: '1'
sig: c9e98853e976b416
body_hash: 86d0d0a30cc7bdbf
raw_hash: ed9018ac40e5dcab
lines: 313-391
---

018 moved the queue out of the worktree to stop a lane inheriting it, but that only happened because the queue was gitignored and lane new clones ignored entries; a note written as a tracked file is not carried
