---
id: 01M0ESRPDEXQZCBMT3TB36B26X
anchor: '@file'
created: 2026-08-20T03:20:04Z
branch: main
norm: '1'
sig: e3b0c44298fc1c14
body_hash: 49d298450869b51d
raw_hash: d147626bbb7aa7ad
lines: 1-26
---

cannot run from a lane worktree: it does 'cp -r /w /build', and a linked worktree's .git is a file pointing at an absolute host path outside the container mount, so git inside fails. Run it from the main checkout.
