---
id: 01M0TT8G1FHE50NFAE3JJ45DFA
anchor: POST_COMMIT_BLOCK
created: 2026-08-24T21:19:11Z
branch: fix/issue-13-rebase-capture
norm: '1'
sig: 05a975d77f5b9730
body_hash: f98d652429c9d778
raw_hash: 04668bfb70bc4c1f
lines: 244-253
---

rebase-merge and rebase-apply both replay commits through post-commit; those runs must not recapture Why trailers
