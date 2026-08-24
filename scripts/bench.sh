#!/usr/bin/env bash
# Benchmark harness. Builds one deterministic repo and times every command against it,
# so a number means the same thing on Monday as it did on Friday.
#
# The fixture is generated, never the lane repo itself: the live repo's note count and
# file sizes drift with every commit, which is exactly what a baseline must not do.
#
#   scripts/bench.sh                          time the current build
#   scripts/bench.sh --out before.json        record a baseline
#   scripts/bench.sh --compare before.json    time again and print the deltas
#   scripts/bench.sh --bin path/to/lane       time a binary built elsewhere
#   scripts/bench.sh --fixture DIR            build the fixture there and stop, for profiling
#   scripts/bench.sh --against old-lane       A/B both binaries in one run, immune to drift
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUNS=${RUNS:-100}
WARMUP=${WARMUP:-15}
BIN=""
OUT=""
BASELINE=""
FILTER=""
FIXTURE_ONLY=""
AGAINST=""

while [ $# -gt 0 ]; do
  case "$1" in
    --bin)     BIN="$2"; shift 2 ;;
    --out)     OUT="$2"; shift 2 ;;
    --compare) BASELINE="$2"; shift 2 ;;
    --runs)    RUNS="$2"; shift 2 ;;
    --only)    FILTER="$2"; shift 2 ;;
    --fixture) FIXTURE_ONLY="$2"; shift 2 ;;
    --against) AGAINST="$2"; shift 2 ;;
    -h|--help) sed -n '2,12p' "$0" | sed 's/^# \?//'; exit 0 ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

command -v hyperfine >/dev/null || { echo "bench needs hyperfine: brew install hyperfine" >&2; exit 1; }
command -v jq >/dev/null || { echo "bench needs jq" >&2; exit 1; }

if [ -z "$BIN" ]; then
  cargo build --release --quiet --manifest-path "$ROOT/crates/lane/Cargo.toml" || exit 1
  TARGET=$(cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/crates/lane/Cargo.toml" \
    | jq -r .target_directory)
  BIN="$TARGET/release/lane"
fi
[ -x "$BIN" ] || { echo "no binary at $BIN" >&2; exit 1; }
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"

if [ -n "$FIXTURE_ONLY" ]; then
  TMP="$FIXTURE_ONLY"; mkdir -p "$TMP"; rm -rf "${TMP:?}/repo"
else
  TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
fi

# ---------------------------------------------------------------------------
# Fixture: 18 files across 5 grammars, ~180 KB, 30 notes, 3 lanes. Sized to sit
# near this repo's own shape so a win here is a win in practice.
# ---------------------------------------------------------------------------
build_fixture() {
  REPO="$TMP/repo"
  mkdir -p "$REPO/src" "$REPO/scripts" "$REPO/docs"
  cd "$REPO" || exit 1
  git init -qb main . && git config user.email b@b.b && git config user.name b

  python3 - "$REPO" <<'PY'
import sys, pathlib
root = pathlib.Path(sys.argv[1])

def rust(n, decls):
    out = ["//! generated fixture module\n"]
    for i in range(decls):
        out.append(f"""
pub struct Item{i} {{
    pub id: usize,
    pub name: String,
}}

pub fn handle_{i}(item: &Item{i}) -> usize {{
    let mut total = item.id;
    for step in 0..8 {{
        total = total.wrapping_add(step * item.name.len());
    }}
    total
}}
""")
    return "".join(out)

def bash(decls):
    out = ["#!/usr/bin/env bash\nset -euo pipefail\n"]
    for i in range(decls):
        out.append(f"""
run_step_{i}() {{
  local input="$1"
  if [ -z "$input" ]; then return 1; fi
  printf '%s\\n' "$input"
}}
""")
    return "".join(out)

def markdown(sections):
    out = ["# Fixture\n"]
    for i in range(sections):
        out.append(f"\n## Section {i}\n\nProse for section {i}. " * 3 + "\n")
    return "".join(out)

def typescript(decls):
    out = ["export type Id = string;\n"]
    for i in range(decls):
        out.append(f"""
export function transform{i}(input: Id): Id {{
  const parts = input.split('-');
  return parts.map((p) => p.trim()).join('/');
}}
""")
    return "".join(out)

# Eight rust files, deliberately uneven: one big file carries most anchors, which is
# the shape that exposes per-note work repeated over one tree.
for i, decls in enumerate([220, 130, 90, 65, 50, 40, 30, 20]):
    (root / "src" / f"mod_{i}.rs").write_text(rust(i, decls))
for i, decls in enumerate([170, 70, 35]):
    (root / "scripts" / f"task_{i}.sh").write_text(bash(decls))
for i in range(3):
    (root / "docs" / f"page_{i}.md").write_text(markdown(60))
for i, decls in enumerate([110, 55]):
    (root / "src" / f"client_{i}.ts").write_text(typescript(decls))
(root / "Cargo.toml").write_text('[package]\nname = "fixture"\nversion = "0.1.0"\n')
(root / "README.md").write_text(markdown(4))
PY

  git add -A && git commit -qm init >/dev/null
  "$BIN" init >/dev/null 2>&1
  git add -A && git commit -qm "lane init" >/dev/null

  # 30 notes, weighted onto mod_0.rs so one tree carries many anchors.
  for i in 0 1 2 3 4 5 6 7 8 9 10 11; do
    "$BIN" note -p src/mod_0.rs -a "fn handle_$i" "fixture note about handle_$i" >/dev/null 2>&1
  done
  for i in 0 1 2 3 4 5; do
    "$BIN" note -p src/mod_1.rs -a "struct Item$i" "fixture note about Item$i" >/dev/null 2>&1
  done
  for i in 1 2 3 4 5 6 7; do
    "$BIN" note -p "src/mod_$i.rs" -a "fn handle_0" "entry point for module $i" >/dev/null 2>&1
  done
  for i in 0 1 2; do
    "$BIN" note -p "scripts/task_$i.sh" -a "run_step_0" "first step of task $i" >/dev/null 2>&1
  done
  "$BIN" note -p src/client_0.ts -a "function transform0" "id shape is dash separated" >/dev/null 2>&1
  "$BIN" note -p docs/page_0.md -a "@file" "page zero is the entry doc" >/dev/null 2>&1
  "$BIN" audit >/dev/null 2>&1
  git add -A && git commit -qm "notes" >/dev/null

  # Three lanes, so `ls` and `sweep` have something to walk.
  for name in alpha beta gamma; do "$BIN" new "$name" >/dev/null 2>&1; done

  NOTES=$(find .lane/memory -name '*.md' 2>/dev/null | wc -l | tr -d ' ')
  BYTES=$(find src scripts docs -type f -exec cat {} + | wc -c | tr -d ' ')
  echo "fixture: $NOTES notes, $BYTES bytes of source, 3 lanes" >&2
}

build_fixture
cd "$REPO" || exit 1

if [ -n "$FIXTURE_ONLY" ]; then
  echo "fixture kept at $REPO" >&2
  exit 0
fi

# name|args. Read-only unless marked; `new` gets a prepare step to stay honest.
CASES=$(cat <<'CASES'
version|--version
ls|ls
check|check
check-json|check --json
why-hot|why src/mod_0.rs
why-cold|why src/mod_7.rs
audit|audit
sweep|sweep --dry-run
shellenv|shellenv
CASES
)

# `new` mutates, so each run starts from a lane that is not there yet. Under -N there is
# no shell to chain the reset, so it lives in a file.
cat > "$TMP/reset-lane" <<RESET
#!/usr/bin/env bash
rm -rf "$REPO/.lane/trees/benchlane"
git -C "$REPO" worktree prune
git -C "$REPO" branch -D benchlane >/dev/null 2>&1
exit 0
RESET
chmod +x "$TMP/reset-lane"

RESULTS="$TMP/results.json"
echo '{}' > "$RESULTS"

# A quiet machine is not a given. --against times both binaries inside ONE hyperfine
# invocation per case, so load that drifts between runs hits both sides equally; the
# earlier --compare reads a file recorded minutes ago and cannot do that.
run_ab() {
  local name="$1" args="$2" prepare="${3:-}"
  [ -n "$FILTER" ] && [[ "$name" != *"$FILTER"* ]] && return 0
  local json="$TMP/ab-$name.json"
  local hf=(-N --warmup "$WARMUP" --runs "$RUNS" --style none --export-json "$json")
  [ -n "$prepare" ] && hf+=(--prepare "$prepare")
  if ! hyperfine "${hf[@]}" "$AGAINST $args" "$BIN $args" >/dev/null 2>&1; then
    echo "  !! $name failed to run" >&2
    return 0
  fi
  local before after delta
  before=$(jq -r '.results[0].mean * 1000' "$json")
  after=$(jq -r '.results[1].mean * 1000' "$json")
  delta=$(python3 -c "print(f'{(($after-$before)/$before*100):+.1f}%')")
  printf '  %-12s %9.1f %9.1f %9s\n' "$name" "$before" "$after" "$delta" >&2
}

run_case() {
  local name="$1" cmd="$2" prepare="${3:-}"
  [ -n "$FILTER" ] && [[ "$name" != *"$FILTER"* ]] && return 0
  local json="$TMP/$name.json"
  local args=(-N --warmup "$WARMUP" --runs "$RUNS" --style none --export-json "$json")
  [ -n "$prepare" ] && args+=(--prepare "$prepare")
  if ! hyperfine "${args[@]}" "$cmd" >/dev/null 2>&1; then
    echo "  !! $name failed to run" >&2
    return 0
  fi
  local mean stddev user
  mean=$(jq -r '.results[0].mean * 1000' "$json")
  stddev=$(jq -r '.results[0].stddev * 1000' "$json")
  user=$(jq -r '.results[0].user * 1000' "$json")
  jq --arg n "$name" --argjson m "$mean" --argjson s "$stddev" --argjson u "$user" \
    '.[$n] = {mean: $m, stddev: $s, user: $u}' "$RESULTS" > "$RESULTS.tmp" && mv "$RESULTS.tmp" "$RESULTS"
  printf '  %-12s %8.1f ms  ± %5.1f   user %6.1f ms\n' "$name" "$mean" "$stddev" "$user" >&2
}

echo "" >&2
echo "binary: $BIN ($(stat -f%z "$BIN" 2>/dev/null || stat -c%s "$BIN") bytes)" >&2
echo "runs:   $RUNS (warmup $WARMUP)" >&2
echo "" >&2

if [ -n "$AGAINST" ]; then
  printf '  %-12s %9s %9s %9s\n' "case" "before" "after" "delta" >&2
  printf '  %-12s %9s %9s %9s\n' "----" "------" "-----" "-----" >&2
  while IFS='|' read -r name args; do
    [ -z "$name" ] && continue
    run_ab "$name" "$args"
  done <<< "$CASES"
  run_ab "new" "new benchlane" "$TMP/reset-lane"
  echo "" >&2
  exit 0
fi

while IFS='|' read -r name args; do
  [ -z "$name" ] && continue
  run_case "$name" "$BIN $args"
done <<< "$CASES"

run_case "new" "$BIN new benchlane" "$TMP/reset-lane"

echo "" >&2

if [ -n "$OUT" ]; then
  cp "$RESULTS" "$OUT"
  echo "wrote $OUT" >&2
fi

if [ -n "$BASELINE" ]; then
  echo "" >&2
  printf '  %-12s %10s %10s %9s\n' "case" "before" "after" "delta" >&2
  printf '  %-12s %10s %10s %9s\n' "----" "------" "-----" "-----" >&2
  jq -r --slurpfile base "$BASELINE" '
    . as $after
    | $base[0] as $before
    | ($before | keys_unsorted) as $names
    | $names[]
    | select($after[.] != null)
    | [., $before[.].mean, $after[.].mean] | @tsv
  ' "$RESULTS" | while IFS=$'\t' read -r name b a; do
    delta=$(python3 -c "print(f'{(($a-$b)/$b*100):+.1f}%')" 2>/dev/null || echo "n/a")
    printf '  %-12s %9.1f %9.1f %9s\n' "$name" "$b" "$a" "$delta" >&2
  done
  echo "" >&2
fi
