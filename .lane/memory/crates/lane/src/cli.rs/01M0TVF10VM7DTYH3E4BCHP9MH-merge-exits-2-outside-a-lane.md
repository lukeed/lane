---
id: 01M0TVF10VM7DTYH3E4BCHP9MH
anchor: fn merge
created: 2026-08-24T21:39:42Z
branch: rename-done-to-merge
norm: '1'
sig: beef067f589982b9
body_hash: f1dfb7472ddc41b0
raw_hash: 4716ab0ee443fe07
lines: 910-968
supersedes: 01M0TRRKQE397F7WNHV66M08JQ
---

merge exits 2 outside a lane and 1 for a dirty lane or blocked trunk, but prepare signals both as errors and every error exits 1, so the mapping must stay in merge or distinct failures collapse into one
