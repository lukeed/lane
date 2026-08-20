#!/usr/bin/env bash
# The gates as ubuntu-latest runs them: Linux, and a filesystem without reflink.
#
# Both matter. cfg(target_os = "linux") code is never type-checked on a macOS host, and
# the podman VM's own filesystem DOES support reflink — so fixtures go on tmpfs, which
# does not, or the no-reflink half of the code silently never runs.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
podman machine start >/dev/null 2>&1 || true

podman run --rm --tmpfs /scratch:size=2g -v "$ROOT":/w:ro -w /w docker.io/library/rust:1-slim sh -c '
  set -e
  apt-get update -qq >/dev/null 2>&1
  apt-get install -y -qq gcc git python3 >/dev/null 2>&1
  rustup component add clippy rustfmt >/dev/null 2>&1
  # A linked worktree .git points outside the container; the gates do not need it because test_lane.sh creates its own repos.
  mkdir -p /build && cp -a /w/. /build/ && rm -rf /build/.git && cd /build
  git config --global user.email ci@lane.test
  git config --global user.name ci
  git config --global init.defaultBranch main
  rustc --version
  cargo fmt --all --check
  cargo clippy --all-targets -- -D warnings
  TMPDIR=/scratch cargo test
  TMPDIR=/scratch ./test_lane.sh
'
