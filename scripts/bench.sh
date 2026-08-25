#!/usr/bin/env bash
# Times `lane new` and `lane rm` against a synthetic repository.
#
# The cost lane pays is one syscall per ignored file, so the file COUNT is the variable
# that matters and the byte total is not. Run it on the filesystem you care about: ext4
# has no reflink at all, and the fallback copy it forces is a different measurement.
#
#   scripts/bench.sh <lane-binary> <workdir> [files] [runs] [synthetic|cargo]
#
# `cargo` builds this workspace inside the repository under test, so the ignored tree is a
# real target/ — a few large archives among many small ones — rather than uniform stubs.
set -uo pipefail

LANE="${1:?usage: bench.sh <lane-binary> <workdir> [files] [runs]}"
WORK="${2:?usage: bench.sh <lane-binary> <workdir> [files] [runs]}"
FILES="${3:-20000}"
RUNS="${4:-3}"
MODE="${5:-synthetic}"
SOURCE="$(cd "$(dirname "$0")/.." && pwd)"
command -v "$LANE" >/dev/null 2>&1 || [ -x "$LANE" ] || { echo "no binary at $LANE" >&2; exit 1; }
LANE="$(cd "$(dirname "$LANE")" && pwd)/$(basename "$LANE")"

# %R alone, so the builtin reports wall seconds and nothing else. BSD date has no %N and
# GNU time is not installed everywhere; this is the one timer both shells always have.
TIMEFORMAT='%R'

# Noise on a shared runner only ever adds, so the minimum is the honest reading — the same
# argument the extent-sharing test makes.
lower() { awk -v a="$1" -v b="$2" 'BEGIN{print (b < a) ? b : a}'; }

# `lane rm` returns before the unlinking does; waiting keeps it out of the next reading.
settle() {
  local i
  for i in $(seq 1 120); do
    [ -z "$(ls -A "$1/.lane/trees/.trash" 2>/dev/null)" ] && return 0
    sleep 1
  done
}

# Two ignored trees, the shape a real checkout has: a build directory of many small files,
# and a dependency directory holding symlinks among them.
build_tree() {
  local root="$1" count="$2" i dir
  for i in $(seq 1 "$count"); do
    dir="$root/$((i / 100))"
    [ -d "$dir" ] || mkdir -p "$dir"
    printf 'artifact %d\n' "$i" > "$dir/unit-$i.o"
  done
}

REPO="$WORK/repo"
rm -rf "$REPO"; mkdir -p "$REPO"; cd "$REPO" || exit 1

if [ "$MODE" = cargo ]; then
  # Tracked files only, then build: target/ has to be made here, on the filesystem under
  # test, or it is not the thing being measured.
  git -C "$SOURCE" archive HEAD | tar -x -C "$REPO"
  git init -qb main .
  git config user.email bench@example.com
  git config user.name bench
  git add -A && git commit -qm base
  cargo build --release --all-targets --quiet 2>/dev/null || cargo build --release --all-targets 2>&1 | tail -5
else
  git init -qb main .
  git config user.email bench@example.com
  git config user.name bench
  mkdir -p src
  for i in $(seq 1 20); do printf 'pub fn f%d() {}\n' "$i" > "src/m$i.rs"; done
  printf 'target/\nnode_modules/\n' > .gitignore
  git add -A && git commit -qm base

  build_tree target $((FILES * 2 / 3))
  build_tree node_modules $((FILES - FILES * 2 / 3))
  ln -sf ../src/m1.rs node_modules/relative-link
  ln -sf "$REPO/src/m2.rs" node_modules/absolute-link
fi

"$LANE" init >/dev/null 2>&1
git add -A >/dev/null 2>&1 && git commit -qm lane >/dev/null 2>&1

echo "kernel=$(uname -sr | tr ' ' '-')"
# -T is GNU; on BSD read the type out of mount instead.
fs=$(df -PT . 2>/dev/null | awk 'NR==2{print $2}')
[ -n "$fs" ] || fs=$(mount | awk -v m="$(df -P . | awk 'NR==2{print $NF}')" '$3==m{print $4}' | tr -d '(,' | head -1)
echo "filesystem=${fs:-unknown}"
echo "mode=$MODE"
echo "ignored_files=$(find target node_modules -type f 2>/dev/null | wc -l | tr -d ' ')"
echo "ignored_bytes=$(du -sk target node_modules 2>/dev/null | awk '{s+=$1} END{printf "%.0fMiB", s/1024}')"

"$LANE" new probe > /tmp/bench-probe.out 2>&1
echo "reflink=$(awk '/reflink:/{print $2; exit}' /tmp/bench-probe.out)"
echo "clone_stats=$(awk '/cloned/{$1=$1; print; exit}' /tmp/bench-probe.out)"
"$LANE" rm probe --force >/dev/null 2>&1
settle "$REPO"

# One untimed pair first: the first read of a cold tree is measuring the page cache.
"$LANE" new warm >/dev/null 2>&1
"$LANE" rm warm --force >/dev/null 2>&1
settle "$REPO"

# Both timings come from one loop: a lane must be removed before the next can be made, and
# timing `new` against a name that already exists measures the refusal instead.
# A command that fails returns fast, and a fast number is what this script is looking for.
# Check the status or a broken lane reads as a quick one.
run() {
  local out; out=$("$@" 2>&1)
  [ $? -eq 0 ] || { echo "FAILED: $* -> $out" >&2; exit 1; }
}

new_best=""; rm_best=""
for _ in $(seq 1 "$RUNS"); do
  t=$( { time run "$LANE" new bench; } 2>&1 )
  new_best=$(lower "${new_best:-$t}" "$t")
  t=$( { time run "$LANE" rm bench --force; } 2>&1 )
  rm_best=$(lower "${rm_best:-$t}" "$t")
  settle "$REPO"
done

echo "lane_new=$new_best"
echo "lane_rm=$rm_best"
