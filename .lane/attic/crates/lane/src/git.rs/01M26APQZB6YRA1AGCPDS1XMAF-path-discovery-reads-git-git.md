---
id: 01M26APQZB6YRA1AGCPDS1XMAF
anchor: fn layout
created: 2026-09-10T18:53:56Z
norm: '1'
sig: 8d764c9bac251436
body_hash: 3626ff17a60c7fc3
raw_hash: d21fcf6e0849e1ca
lines: 64-70
supersedes: 01M268SCJ4KD9R18KG9CEBYQTC
---

Path discovery reads .git, gitdir pointers, and commondir directly because macOS Git startup dominates command time. Explicit local core.bare values use the same in-process path; complex configuration falls back to Git.
