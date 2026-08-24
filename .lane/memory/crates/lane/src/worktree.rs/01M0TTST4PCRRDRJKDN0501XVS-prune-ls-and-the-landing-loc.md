---
id: 01M0TTST4PCRRDRJKDN0501XVS
anchor: fn trunk_name
created: 2026-08-24T21:27:24Z
branch: rename-sweep-prune
norm: '1'
sig: 6388df923cf25623
body_hash: 1fe611ea9801b789
raw_hash: d852df978d3a005c
lines: 19-32
supersedes: 01M0TRRKSWK26RY06J8DX6AQEY
---

prune, ls and the landing lock feed this straight into `git show <ref>:` and into a lock filename, so it must name a local branch and must not follow whatever is checked out: origin/main breaks both consumers, and standing on develop makes lanes landed into main stop reporting landed
