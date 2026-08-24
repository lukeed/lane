---
id: 01M0TMPTRSMCPHAKVYK4FMJJ3K
anchor: fn byte_range
created: 2026-08-24T19:14:32Z
branch: perf-audit
norm: '1'
sig: 651739647cf1a22b
body_hash: 465d8737e0d68b54
raw_hash: 2fdd1141ea465d06
lines: 272-290
---

span_text feeds the note fingerprints, so a one-byte shift here silently marks every note in every repo as drifted. the line index was proved against the old scan over CRLF, multi-byte UTF-8 and inverted spans rather than against fresh expectations, because a test written next to the change encodes the change's own blind spot
