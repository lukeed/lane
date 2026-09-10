---
id: 01M268SCNPBZJVVGY1M00P6TPX
anchor: fn primary_worktree
created: 2026-09-10T18:20:26Z
norm: '1'
sig: 39f8121dda410eb6
body_hash: 7905f3853e717c2b
raw_hash: 2d64f3cb3514333e
vouched: 2026-09-10T18:54:09Z
lines: 104-130
---

An included core.bare=true can make git worktree list mark the common repository bare while rev-parse --is-bare-repository returns false. Query the effective core.bare boolean with git config so includes and config.worktree control primary-root selection.
