#!/usr/bin/env bash
# End-to-end test. The interesting assertions are 4 (granularity) and 7
# (two branches writing memory for the same file must merge clean).
set -euo pipefail

CTX="$(cd "$(dirname "$0")" && pwd)/ctx"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"

pass=0; fail=0
ok()   { pass=$((pass+1)); echo "  ok   - $1"; }
bad()  { fail=$((fail+1)); echo "  FAIL - $1"; }
is()   { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (want '$3', got '$2')"; fi; }

git init -q .
git config user.email t@t.t; git config user.name t
git config merge.union.driver 'git merge-file -p %A %O %B > %A' 2>/dev/null || true

mkdir -p src
cat > src/auth.rs <<'EOF'
pub fn verify(token: &str) -> bool {
    let parsed = parse(token);
    parsed.is_valid()
}

pub fn refresh(token: &str) -> String {
    rotate(token)
}
EOF

cat > src/Editor.svelte <<'EOF'
<script>
  let undoStack = [];
  function push(op) { undoStack.push(op); }
</script>

<div class="viewport">{undoStack.length}</div>

<style>
  .viewport { overflow: auto; }
</style>
EOF

git add -A && git commit -qm init

echo "== 1. init =="
"$CTX" init > /dev/null
is "gitattributes has union rule" "$(grep -c 'merge=union' .gitattributes)" "2"
is "AGENTS.md has protocol" "$(grep -c 'Context memory' AGENTS.md)" "1"

echo "== 2. note + audit promotes =="
"$CTX" note -p src/auth.rs -a "fn verify" \
  "must be constant-time; early return leaks token length" > /dev/null
"$CTX" note -p src/Editor.svelte -a "#script" \
  "undo stack must be cleared on doc swap or ops replay onto wrong doc" > /dev/null
"$CTX" note -p src/Editor.svelte -a "#style" \
  "overflow:auto here not scroll, scroll causes jank on ios safari" > /dev/null
"$CTX" audit > /dev/null
is "three notes on disk" "$(find .context -name '*.md' | wc -l | tr -d ' ')" "3"
is "all fresh" "$("$CTX" check --json | grep -c '"tier": "fresh"')" "3"

echo "== 3. comment churn is not drift =="
sed -i 's|    let parsed = parse(token);|    // NOTE: see RFC\n    let parsed = parse(token);|' src/auth.rs
is "comment-only edit stays fresh" \
   "$("$CTX" check --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print([x["tier"] for x in d if x["anchor"]=="fn verify"][0])')" \
   "fresh"

echo "== 4. GRANULARITY: editing #script must not stale #style =="
sed -i 's|  let undoStack = \[\];|  let undoStack = [];\n  let cursor = 0;|' src/Editor.svelte
"$CTX" check --json > /tmp/c.json
is "#script drifts" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/c.json"));print([x["tier"] for x in d if x["anchor"]=="#script"][0])')" \
   "body-drift"
is "#style unaffected" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/c.json"));print([x["tier"] for x in d if x["anchor"]=="#style"][0])')" \
   "fresh"

echo "== 5. signature change is distinguished from body drift =="
sed -i 's|pub fn verify(token: &str) -> bool {|pub fn verify(token: \&str, now: u64) -> bool {|' src/auth.rs
is "sig change detected" \
   "$("$CTX" check --json | python3 -c 'import json,sys;d=json.load(sys.stdin);print([x["tier"] for x in d if x["anchor"]=="fn verify"][0])')" \
   "signature-changed"

echo "== 6. anchor deleted -> evicted to attic =="
"$CTX" audit > /dev/null   # clears the flag by refreshing fingerprints
sed -i 's|pub fn refresh|pub fn rotate_token|' src/auth.rs
"$CTX" note -p src/auth.rs -a "fn refresh" "rotation is idempotent upstream" > /dev/null
"$CTX" audit > /dev/null
is "missing anchor evicted" "$(find .context/.attic -name '*.md' | wc -l | tr -d ' ')" "1"
is "attic note is recoverable" "$(grep -rc 'idempotent upstream' .context/.attic | cut -d: -f2)" "1"

echo "== 7. PARALLEL: two branches, same file, no conflict =="
git add -A && git commit -qm base
git checkout -qb thread-a
"$CTX" note -p src/Editor.svelte -a "#script" "a: batches ops per frame" > /dev/null
"$CTX" audit > /dev/null && git add -A && git commit -qm a
git checkout -q master 2>/dev/null || git checkout -q main
git checkout -qb thread-b
"$CTX" note -p src/Editor.svelte -a "#script" "b: cursor resets on remote patch" > /dev/null
"$CTX" audit > /dev/null && git add -A && git commit -qm b

if git merge -q --no-edit thread-a 2>/tmp/merge.err; then
  ok "parallel memory merges without conflict"
else
  bad "merge conflicted: $(cat /tmp/merge.err)"
fi
is "both notes survived" "$(grep -rl 'batches ops per frame\|cursor resets' .context --include='*.md' | wc -l | tr -d ' ')" "2"

echo "== 8. budget evicts least-read first =="
for i in 1 2 3 4; do
  "$CTX" note -p src/auth.rs -a "fn verify" "filler note number $i about verify" > /dev/null
done
"$CTX" audit > /dev/null
"$CTX" why src/auth.rs -a "fn verify" > /dev/null   # bumps reads on survivors
"$CTX" audit --max-notes 2 --json > /tmp/a.json
is "budget capped to 2" \
   "$(find .context/src/auth.rs -name '*.md' | wc -l | tr -d ' ')" "2"
is "eviction reason recorded" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/a.json"));print(d["evicted"][0]["reason"])')" \
   "budget"

echo
echo "passed: $pass   failed: $fail"
[ "$fail" -eq 0 ]
