---
id: 01M0TRRKQE397F7WNHV66M08JQ
anchor: fn done
created: 2026-08-24T20:52:56Z
branch: push-command
norm: '1'
sig: 52d46897eab5ba03
body_hash: f1dfb7472ddc41b0
raw_hash: 9cd6a012847a6417
lines: 948-1006
supersedes: 01M0TQYBPCHTWG41WA1PMS1G4Y
---

done exits 2 outside a lane and 1 for a dirty lane or a blocked trunk, but prepare signals both as errors and every error exits 1, so the mapping has to stay in done or two distinct failures collapse into one
