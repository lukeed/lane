---
id: 01M0X4BS4J4D1RF4M6ER7YJFVA
anchor: fn clone_dir_tree
created: 2026-08-25T18:52:37Z
norm: '1'
sig: 49f77fc8c56e183b
body_hash: c6719fd7528fb92c
raw_hash: b54a55bc17356e6d
vouched: 2026-09-14T19:19:16Z
lines: 197-241
---

a directory clone copies symlinks verbatim, so an absolute link into the source survives pointing at the source; the fixup walk reads dirents and costs a fraction of cloning each file
