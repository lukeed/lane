---
id: 01M0RP8055DHXBS165696WAHB3
anchor: struct Cli
created: 2026-08-24T01:30:16Z
branch: usage-migration
norm: '1'
sig: 6b6db5d128fddaa6
body_hash: ea773d26a9c4aad7
raw_hash: a00b0f26f8b230e9
lines: 24-27
---

usage treats an unknown flag as a value by default; unknown_flags = "error" on the root restores clap's rejection and reaches every subcommand
