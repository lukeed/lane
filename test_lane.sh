#!/usr/bin/env bash
# End-to-end suite for `lane`. Section 6 is the one that matters: two lanes opened
# from the same trunk, both writing memory about the same anchor, must both land.
# The clone layer and anchor resolution are covered by `cargo test`.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
FAKE="$ROOT/tests/fake-reviewer"
cargo build --quiet --manifest-path "$ROOT/crates/lane/Cargo.toml" || exit 1
TARGET=$(cargo metadata --no-deps --format-version 1 --manifest-path "$ROOT/crates/lane/Cargo.toml" \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
LANE="$TARGET/debug/lane"
[ -x "$LANE" ] || { echo "no binary at $LANE"; exit 1; }

TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); echo "  ok   - $1"; }
bad() { fail=$((fail+1)); echo "  FAIL - $1"; }
is()  { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (want '$3', got '$2')"; fi; }
# BSD sed wants an argument to -i, GNU sed refuses one; do the rename ourselves.
sedi() { local expr="$1"; shift; for f in "$@"; do sed "$expr" "$f" > "$f.sedi" && mv "$f.sedi" "$f"; done; }

setup() {
  cd "$TMP" && rm -rf repo .lanes-repo && mkdir repo && cd repo
  git init -qb main . && git config user.email t@t.t && git config user.name t
  mkdir -p src node_modules/pkg
  cat > src/auth.rs <<'EOF'
pub fn verify(token: &str) -> bool {
    parse(token).is_valid()
}

pub fn refresh(token: &str) -> String {
    rotate(token)
}
EOF
  head -c 2000000 /dev/urandom > node_modules/pkg/blob.bin
  printf 'node_modules/\n' > .gitignore
  git add -A && git commit -qm init
  "$LANE" init > /dev/null
  git add -A && git commit -qm "lane init"
}

echo "== 2. new: warm cache arrives, tracked files from git, status clean =="
setup
"$LANE" new fix-login > /tmp/new.out 2>&1
LP="$TMP/.lanes-repo/fix-login"
is "lane exists" "$([ -d "$LP" ] && echo yes)" "yes"
is "warm dir present in lane" "$([ -f "$LP/node_modules/pkg/blob.bin" ] && echo yes)" "yes"
is "tracked file present" "$([ -f "$LP/src/auth.rs" ] && echo yes)" "yes"
is "lane status clean" "$(git -C "$LP" status --porcelain | wc -l | tr -d ' ')" "0"
is "reflink verdict reported" "$(grep -c 'reflink:' /tmp/new.out)" "1"
is "tracked files not re-cloned" \
   "$(grep -o 'cloned' /tmp/new.out | head -1)" "cloned"

echo "== 3. fork mode carries dirty state without rewriting files =="
setup
echo "// scratch work" >> src/auth.rs
"$LANE" new spike --fork > /tmp/fork.out 2>&1
LP="$TMP/.lanes-repo/spike"
is "dirty change carried into fork" "$(grep -c 'scratch work' "$LP/src/auth.rs")" "1"
is "fork reports carried changes" "$(grep -c 'carried' /tmp/fork.out)" "1"
is "index rebuilt: exactly one modified file" \
   "$(git -C "$LP" status --porcelain | grep -c '^ M')" "1"
is "warm dir also carried" "$([ -f "$LP/node_modules/pkg/blob.bin" ] && echo yes)" "yes"

echo "== 4. note inside lane, done lands memory on trunk =="
setup
"$LANE" new fix-login > /dev/null 2>&1
LP="$TMP/.lanes-repo/fix-login"
cd "$LP"
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
"$LANE" note -p src/auth.rs -a "fn verify" \
  "must be constant-time; early return leaks token length" > /dev/null
git add -A && git commit -qm "refactor verify"
"$LANE" done > /tmp/done.out 2>&1
cd "$TMP/repo"
is "trunk advanced" "$(git log --oneline main | grep -c 'refactor verify')" "1"
is "memory committed to trunk" \
   "$(git show main --name-only --format= | grep -c '^\.context/src/auth\.rs/')" "1"
is "note readable from trunk" \
   "$("$LANE" why src/auth.rs | grep -c 'constant-time')" "1"
is "lane removed" "$([ -d "$TMP/.lanes-repo/fix-login" ] && echo yes || echo no)" "no"
is "branch deleted" "$(git branch --list fix-login | wc -l | tr -d ' ')" "0"

echo "== 5. staleness granularity survives the unified flow =="
cat > src/Editor.svelte <<'EOF'
<script>
  let undoStack = [];
</script>
<style>
  .viewport { overflow: auto; }
</style>
EOF
git add -A && git commit -qm sfc
"$LANE" note -p src/Editor.svelte -a "#script" "clear undo stack on doc swap" > /dev/null
"$LANE" note -p src/Editor.svelte -a "#style" "auto not scroll; ios safari jank" > /dev/null
"$LANE" audit > /dev/null
sedi 's|  let undoStack = \[\];|  let undoStack = [];\n  let cursor = 0;|' src/Editor.svelte
"$LANE" check --json > /tmp/c.json
is "#script drifts" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/c.json"));print([x["tier"] for x in d if x["anchor"]=="#script"][0])')" \
   "body-drift"
is "#style unaffected" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/c.json"));print([x["tier"] for x in d if x["anchor"]=="#style"][0])')" \
   "fresh"
git add -A && git commit -qm sfc2

echo "== 6. PARALLEL: two lanes, same anchor, both land clean =="
"$LANE" new thread-a > /dev/null 2>&1
"$LANE" new thread-b > /dev/null 2>&1
( cd "$TMP/.lanes-repo/thread-a" && "$LANE" note -p src/auth.rs -a "fn verify" \
    "a: callers rely on false-on-expiry, not an error" > /dev/null && "$LANE" done > /tmp/a.out 2>&1 )
is "lane a landed" "$?" "0"
( cd "$TMP/.lanes-repo/thread-b" && "$LANE" note -p src/auth.rs -a "fn verify" \
    "b: token parse allocates; hot path, do not add regex" > /dev/null && "$LANE" done > /tmp/b.out 2>&1 )
is "lane b landed after a, no conflict" "$?" "0"
cd "$TMP/repo"
is "both memories on trunk" \
   "$(git grep -l 'callers rely on false-on-expiry\|do not add regex' main -- .context | wc -l | tr -d ' ')" "2"

echo "== 7. anchor deleted -> attic =="
sedi 's|pub fn refresh|pub fn rotate_token|' src/auth.rs
"$LANE" note -p src/auth.rs -a "fn refresh" "rotation is idempotent upstream" > /dev/null
"$LANE" audit > /dev/null
is "evicted to attic" "$(find .context/.attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "1"

echo "== 8. model-in-the-loop review =="
setup
cat > src/sync.rs <<'EOF'
pub fn reconnect(session: &Session) -> Result<()> {
    let lock = session.lock()?;
    dial(lock)
}

pub fn send(msg: Msg) -> Result<()> {
    transmit(msg)
}
EOF
git add -A && git commit -qm sync
"$LANE" note -p src/auth.rs -a "fn verify" "must be constant-time; early return leaks length" > /dev/null
"$LANE" note -p src/sync.rs -a "fn reconnect" "caller must not hold the session lock" > /dev/null
"$LANE" note -p src/sync.rs -a "fn send" "never retries; upstream is not idempotent" > /dev/null
"$LANE" audit > /dev/null

# drift all three spans
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
sedi 's|    let lock = session.lock()?;|    let lock = session.assume_locked();|' src/sync.rs
sedi 's|    transmit(msg)|    backoff_retry(\|\| transmit(msg))|' src/sync.rs

"$LANE" audit --review cmd --review-cmd "$FAKE" --json > /tmp/r.json
V=/tmp/r.json
is "reviewer reported" "$(python3 -c 'import json;print(json.load(open("/tmp/r.json"))["reviewer"].split("(")[0])')" "cmd"
is "three verdicts returned" "$(python3 -c 'import json;print(len(json.load(open("/tmp/r.json"))["verdicts"]))')" "3"
is "holds keeps note fresh" \
   "$("$LANE" check --json | python3 -c 'import json,sys;d=json.load(sys.stdin);print([x["tier"] for x in d if x["anchor"]=="fn verify"][0])')" \
   "fresh"
is "superseded wrote a replacement note" \
   "$(grep -rl 'session lock is now taken by the caller' .context --include='*.md' | grep -vc attic)" "1"
is "replacement records supersedes" \
   "$(grep -rh 'supersedes:' .context/src/sync.rs/*.md | wc -l | tr -d ' ')" "1"
is "superseded original in attic" \
   "$(grep -rl 'caller must not hold' .context/.attic | wc -l | tr -d ' ')" "1"
is "contradicted note quarantined" \
   "$(grep -rl 'never retries' .context/.attic | wc -l | tr -d ' ')" "1"
is "contradicted removed from live store" \
   "$(grep -rl 'never retries' .context --include='*.md' | grep -vc attic)" "0"
is "attic records the reason" \
   "$(grep -rh 'evicted:.*contradicted' .context/.attic | wc -l | tr -d ' ')" "1"

echo "== 9. review is off by default (no key, no cmd) =="
env -u ANTHROPIC_API_KEY -u LANE_REVIEW_CMD "$LANE" audit --json > /tmp/n.json
is "defaults to no reviewer" "$(python3 -c 'import json;print(json.load(open("/tmp/n.json"))["reviewer"])')" "none"

echo "== 10. malformed model output is survivable =="
is "garbage response yields no verdicts" \
   "$("$LANE" audit --review cmd --review-cmd "echo 'sorry, I cannot'" --json | python3 -c 'import json,sys;print(len(json.load(sys.stdin)["verdicts"]))')" \
   "0"

echo "== 11. rm will not discard commits trunk does not have =="
setup
"$LANE" new scrap > /dev/null 2>&1
( cd "$TMP/.lanes-repo/scrap" && echo "fn work() {}" > src/work.rs \
  && git add -A && git commit -qm "unlanded work" > /dev/null )
SHA=$(git rev-parse scrap)
"$LANE" rm scrap > /tmp/rm.out 2>&1
is "rm exits non-zero when it kept a branch" "$?" "1"
is "rm says the branch was kept" "$(grep -c 'kept branch scrap' /tmp/rm.out)" "1"
is "unlanded branch survives" "$(git branch --list scrap | wc -l | tr -d ' ')" "1"
is "unlanded commit still reachable" "$(git rev-parse scrap)" "$SHA"
is "worktree is gone either way" \
   "$([ -d "$TMP/.lanes-repo/scrap" ] && echo yes || echo no)" "no"

"$LANE" new scrap2 > /dev/null 2>&1
( cd "$TMP/.lanes-repo/scrap2" && echo "fn work() {}" > src/work2.rs \
  && git add -A && git commit -qm "throwaway" > /dev/null )
"$LANE" rm scrap2 --force > /dev/null 2>&1
is "--force discards the branch" "$(git branch --list scrap2 | wc -l | tr -d ' ')" "0"

echo "== 12. init scaffolding and the per-anchor budget =="
setup
is "gitattributes has both union rules" "$(grep -c 'merge=union' .gitattributes)" "2"
is "AGENTS.md has the protocol" "$(grep -c 'Context memory' AGENTS.md)" "1"
is "pending notes are ignored" "$(grep -c '.wt/pending.jsonl' .gitignore)" "1"

for i in 1 2 3 4 5; do
  "$LANE" note -p src/auth.rs -a "fn verify" "filler note number $i about verify" > /dev/null
done
"$LANE" audit > /dev/null
"$LANE" why src/auth.rs -a "fn verify" > /dev/null
"$LANE" audit --max-notes 2 --json > /tmp/budget.json
is "budget caps the anchor at 2 notes" \
   "$(find .context/src/auth.rs -name '*.md' | wc -l | tr -d ' ')" "2"
is "eviction reason is recorded" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/budget.json"));print(d["evicted"][0]["reason"])')" \
   "budget"
is "evicted notes are recoverable from the attic" \
   "$(find .context/.attic -name '*.md' | wc -l | tr -d ' ')" "3"

echo "== 13. two branches writing memory merge without conflict =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed

git checkout -qb branch-a
"$LANE" note -p src/auth.rs -a "fn verify" "a: alpha" > /dev/null
"$LANE" audit > /dev/null && git add -A && git commit -qm a
git checkout -q main && git checkout -qb branch-b
"$LANE" note -p src/auth.rs -a "fn verify" "b: beta" > /dev/null
"$LANE" audit > /dev/null && git add -A && git commit -qm b
git merge -q --no-edit branch-a > /tmp/merge.out 2>&1
is "parallel memory merges without conflict" "$?" "0"
is "both notes survived" \
   "$(grep -rl 'a: alpha\|b: beta' .context --include='*.md' | wc -l | tr -d ' ')" "2"

echo
echo "passed: $pass   failed: $fail"
[ "$fail" -eq 0 ]
