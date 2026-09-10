---
id: 01M26AXM8DMG135SCDEDRPBRB1
anchor: fn local_bare
created: 2026-09-10T18:57:41Z
norm: '1'
sig: 1bbe2a8490bba61b
body_hash: c87874b56449c05a
raw_hash: f0168ddf8f1b4e7e
lines: 135-165
---

gix-config accepts decimal i64 boolean values that Git rejects as invalid base-0 bounded integers. Only textual booleans, empty values, and 0 or 1 can use the fast path; other numeric forms must go through Git.
