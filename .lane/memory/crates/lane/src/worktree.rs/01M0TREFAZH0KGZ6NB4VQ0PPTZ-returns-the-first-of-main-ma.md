---
id: 01M0TREFAZH0KGZ6NB4VQ0PPTZ
anchor: fn trunk_name
created: 2026-08-24T20:44:38Z
branch: HEAD
norm: '1'
sig: 6388df923cf25623
body_hash: 1fe611ea9801b789
raw_hash: d852df978d3a005c
lines: 19-32
---

returns the first of main/master/trunk that exists as a ref, not the default branch, so a repo whose default is develop while a stale main survives gets rebased onto the wrong ref silently
