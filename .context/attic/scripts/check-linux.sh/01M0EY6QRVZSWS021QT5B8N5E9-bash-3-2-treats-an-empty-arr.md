---
id: 01M0EY6QRVZSWS021QT5B8N5E9
anchor: podman run
created: 2026-08-20T06:30:09Z
branch: fix-linux-gate
norm: '1'
---

Bash 3.2 treats an empty array expansion as unbound under nounset, while a main checkout intentionally has no common-directory mount.
