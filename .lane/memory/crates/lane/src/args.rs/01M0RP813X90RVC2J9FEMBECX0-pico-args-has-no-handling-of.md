---
id: 01M0RP813X90RVC2J9FEMBECX0
anchor: fn terminated
created: 2026-08-24T01:30:25Z
norm: '1'
sig: 7b60764a4d3b6ee8
body_hash: 9c08eb1b1627f4b2
raw_hash: 15891290909f68f7
lines: 174-183
---

pico-args has no -- handling of its own, so the split must happen before Arguments::from_vec sees the words; otherwise a flag written after -- is still read as one
