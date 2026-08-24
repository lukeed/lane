---
id: 01M0V0CJJ6TDCGYRRGQSVH5101
anchor: '@file'
created: 2026-08-24T23:06:16Z
norm: '1'
sig: 70acf00586aa7b90
body_hash: 9aef453ab469f78d
raw_hash: eec213dc7e417838
lines: 1-68
supersedes: 01M0RP813WRK018HGY8ATHW0XT
---

pico-args ships eq-separator and short-space-opt off by default, so an unfeatured dependency makes --base=main fail because --base has no associated value; both features are named in the manifest on purpose
