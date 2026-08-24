---
id: 01M0TRRKSWK26RY06J8DX6AQEY
anchor: fn trunk_name
created: 2026-08-24T20:52:56Z
branch: push-command
norm: '1'
sig: 6388df923cf25623
body_hash: 1fe611ea9801b789
raw_hash: d852df978d3a005c
lines: 19-32
supersedes: 01M0TQYBQVSRHXXA7M83H5GE1A
---

sweep, ls and the landing lock feed this straight into `git show <ref>:` and into a lock filename, so it must name a local branch and must not follow whatever is checked out: origin/main breaks both consumers, and standing on develop makes lanes landed into main stop reporting landed
