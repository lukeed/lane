#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

cargo install --path "$ROOT/crates/lane" --force --locked

printf '\nRefresh the shell integration in this terminal with:\n'
printf '  eval "$(lane shellenv)"\n'
