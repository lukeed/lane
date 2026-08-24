---
id: 01M0TMPTRNJVJYQ52G0CF8ZV5J
anchor: fn layout
created: 2026-08-24T19:14:32Z
norm: '1'
sig: 8d764c9bac251436
body_hash: 3626ff17a60c7fc3
raw_hash: d21fcf6e0849e1ca
lines: 56-62
---

on macOS /usr/bin/git costs ~20 ms to reach main() — `git --version` with no repo measures 20.4 ms — so a spawn is worth ~20 ms whatever it does, and spawn COUNT is the number that moves wall time here, not what each command asks for. that is why layout reads .git, the gitdir: pointer and commondir directly instead of asking rev-parse
