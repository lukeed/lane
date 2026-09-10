---
id: 01M268SCNPBZJVVGY1M00P6TPX
anchor: fn primary_worktree
created: 2026-09-10T18:20:26Z
norm: '1'
sig: 39f8121dda410eb6
body_hash: 105b0ad86172d2c6
raw_hash: a9cc464364efc7b8
lines: 104-130
---

An included core.bare=true can make git worktree list mark the common repository bare while rev-parse --is-bare-repository returns false. Query the effective core.bare boolean with git config so includes and config.worktree control primary-root selection.
