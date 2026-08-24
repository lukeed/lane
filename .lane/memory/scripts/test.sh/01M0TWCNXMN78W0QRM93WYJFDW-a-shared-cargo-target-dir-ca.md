---
id: 01M0TWCNXMN78W0QRM93WYJFDW
anchor: '@file'
created: 2026-08-24T21:56:26Z
branch: scripts-and-readme
norm: '1'
sig: e3b0c44298fc1c14
body_hash: c2c499ae45095bd2
raw_hash: 0fabad15014070b4
lines: 1-884
---

a shared CARGO_TARGET_DIR can serve another worktree’s lane binary for the same package; Cargo may print Finished without recompiling, so run cargo clean -p lane before trusting this suite in a lane
