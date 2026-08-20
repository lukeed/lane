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
REFLINK=$(grep -c 'reflink: yes' /tmp/new.out)
want() { [ "$REFLINK" = "1" ] && echo yes || echo no; }
is "lane exists" "$([ -d "$LP" ] && echo yes)" "yes"
is "warm dir present in lane iff reflink" \
   "$([ -f "$LP/node_modules/pkg/blob.bin" ] && echo yes || echo no)" "$(want)"
is "tracked file present" "$([ -f "$LP/src/auth.rs" ] && echo yes)" "yes"
is "lane status clean" "$(git -C "$LP" status --porcelain | wc -l | tr -d ' ')" "0"
is "reflink verdict reported" "$(grep -c 'reflink:' /tmp/new.out)" "1"
is "tracked files not re-cloned" \
   "$(grep -o 'cloned' /tmp/new.out | head -1)" "cloned"

echo "== 3. dirty mode carries dirty state without rewriting files =="
setup
echo "// scratch work" >> src/auth.rs
"$LANE" new spike --dirty > /tmp/dirty.out 2>&1
LP="$TMP/.lanes-repo/spike"
REFLINK=$(grep -c 'reflink: yes' /tmp/dirty.out)
want() { [ "$REFLINK" = "1" ] && echo yes || echo no; }
# --dirty honours the flag on any filesystem: reflinked whole-tree with it, copied without.
is "dirty change carried" \
   "$(grep -q 'scratch work' "$LP/src/auth.rs" && echo yes || echo no)" "yes"
is "dirty mode reports what it carried" \
   "$(grep -q 'carried' /tmp/dirty.out && echo yes || echo no)" "yes"
is "exactly one modified file, not the whole tree" \
   "$(git -C "$LP" status --porcelain | grep -c '^ M')" "1"
is "warm dir also carried iff reflink" \
   "$([ -f "$LP/node_modules/pkg/blob.bin" ] && echo yes || echo no)" "$(want)"

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
   "$(git show main --name-only --format= | grep -c '^\.context/-/src/auth\.rs/')" "1"
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
is "evicted to attic" "$(find .context/attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "1"

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
   "$(grep -rh 'supersedes:' .context/-/src/sync.rs/*.md | wc -l | tr -d ' ')" "1"
is "superseded original in attic" \
   "$(grep -rl 'caller must not hold' .context/attic | wc -l | tr -d ' ')" "1"
is "contradicted note quarantined" \
   "$(grep -rl 'never retries' .context/attic | wc -l | tr -d ' ')" "1"
is "contradicted removed from live store" \
   "$(grep -rl 'never retries' .context --include='*.md' | grep -vc attic)" "0"
is "the log records the reason" \
   "$(grep -rh 'contradicted' .context/log/*.jsonl | wc -l | tr -d ' ')" "2"

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
is "gitattributes has one union rule, for the log" \
   "$(grep -c 'log/\*.jsonl merge=union' .gitattributes)" "1"
is "AGENTS.md has the protocol" "$(grep -c 'Context memory' AGENTS.md)" "1"
is "init does not touch .gitignore" "$(grep -c 'pending.jsonl' .gitignore)" "0"

for i in 1 2 3 4 5; do
  "$LANE" note -p src/auth.rs -a "fn verify" "filler note number $i about verify" > /dev/null
done
"$LANE" audit > /dev/null
"$LANE" why src/auth.rs -a "fn verify" > /dev/null
"$LANE" audit --max-notes 2 --json > /tmp/budget.json
is "budget caps the anchor at 2 notes" \
   "$(find .context/-/src/auth.rs -name '*.md' | wc -l | tr -d ' ')" "2"
is "eviction reason is recorded" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/budget.json"));print(d["evicted"][0]["reason"])')" \
   "budget"
is "evicted notes are recoverable from the attic" \
   "$(find .context/attic -name '*.md' | wc -l | tr -d ' ')" "3"

"$LANE" note -p src/auth.rs -a "fn verify" "a note the parent has not promoted" > /dev/null
"$LANE" new inherit > /dev/null 2>&1
is "a fresh lane does not inherit the parent's queue" \
   "$("$LANE" ls | grep -c 'inherit.*0 pending')" "1"
"$LANE" rm inherit --force > /dev/null 2>&1

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

echo "== 14. audit is idempotent and a damaged note stays readable =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
"$LANE" audit > /dev/null
is "a no-op audit writes nothing" \
   "$(git status --porcelain -- .context | wc -l | tr -d ' ')" "0"

F=$(find .context -name '*.md' -not -path '*attic*' | head -1)
python3 - "$F" <<'DUP'
import io, sys
p = sys.argv[1]
s = io.open(p, encoding="utf-8").read()
io.open(p, "w", encoding="utf-8").write(s.replace("checked:", "checked: 2099-01-01T00:00:00Z\nchecked:", 1))
DUP
BEFORE=$(cksum < "$F")
is "a duplicated key does not hide the note" \
   "$("$LANE" why src/auth.rs 2>/dev/null | grep -c 'constant time')" "1"
"$LANE" audit > /dev/null 2>&1
is "and does not evict it" \
   "$(find .context/attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "0"
is "and the damaged file is left alone" "$(cksum < "$F")" "$BEFORE"

echo "== 15. a renamed file keeps its memory =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
git mv src/auth.rs src/token.rs && git commit -qm rename
"$LANE" audit > /tmp/mv.out 2>&1
is "audit reports the move" "$(grep -c 'moved   src/auth.rs -> src/token.rs' /tmp/mv.out)" "1"
is "the note followed the file" \
   "$("$LANE" why src/token.rs 2>/dev/null | grep -c 'constant-time')" "1"
is "nothing was evicted" \
   "$(find .context/attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "0"
is "the old directory is gone" \
   "$([ -d .context/-/src/auth.rs ] && echo yes || echo no)" "no"
git add -A && git commit -qm memory
git rm -q src/token.rs && git commit -qm delete
"$LANE" audit > /dev/null 2>&1
is "a genuine deletion still evicts" \
   "$(find .context/attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "1"

echo "== 16. a lane that renames a file lands its memory on the new path =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
"$LANE" new refactor > /dev/null 2>&1
( cd "$TMP/.lanes-repo/refactor" \
  && git mv src/auth.rs src/token.rs && git commit -qm rename > /dev/null \
  && "$LANE" done > /tmp/lanemv.out 2>&1 )
cd "$TMP/repo"
is "done reports the move" "$(grep -c '^  moved   ' /tmp/lanemv.out)" "1"
is "trunk has the note on the new path" \
   "$("$LANE" why src/token.rs 2>/dev/null | grep -c 'constant-time')" "1"

echo "== 17. notes are immutable; state and log are per-branch and roll up =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
N=$(find .context/- -name '*.md' | head -1)
BEFORE=$(cksum < "$N")
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
"$LANE" audit > /dev/null
is "a drifted note file is not rewritten" "$(cksum < "$N")" "$BEFORE"
is "the note carries no path field" "$(grep -c '^path:' "$N")" "0"
is "the fingerprint lives in state" \
   "$(find .context/state -name '*.json' | wc -l | tr -d ' ')" "1"
is "state records the drift" \
   "$(python3 -c 'import json,glob;d=json.load(open(glob.glob(".context/state/*.json")[0]));print(list(d.values())[0]["status"])')" \
   "body-drift"

mkdir -p attic && echo "user content" > attic/f.txt
git add -A && git commit -qm user-attic
"$LANE" note -p attic/f.txt -a "@file" "a repo may have its own attic" > /dev/null
"$LANE" audit > /dev/null
is "a user path named attic does not collide" \
   "$(find '.context/-/attic' -name '*.md' | wc -l | tr -d ' ')" "1"
git add -A && git commit -qm memory

"$LANE" new land > /dev/null 2>&1
( cd "$TMP/.lanes-repo/land" \
  && "$LANE" note -p src/auth.rs -a "fn verify" "from the lane" > /dev/null \
  && "$LANE" done > /dev/null 2>&1 )
cd "$TMP/repo"
is "done rolls the lane's state into trunk's" \
   "$(find .context/state -name '*.json' | wc -l | tr -d ' ')" "1"
is "the lane's own state file is gone" \
   "$([ -f .context/state/land.json ] && echo yes || echo no)" "no"

echo "== 18. anchors we cannot resolve are kept, not discarded =="
setup
printf 'func verify(_ t: String) -> Bool {\n    return ok(t)\n}\n' > src/Auth.swift
git add -A && git commit -qm swift
"$LANE" note -p src/Auth.swift -a "func verify" "swift: constant time" 2> /tmp/w.out
is "note on an unparsed language warns" "$(grep -c 'warning:' /tmp/w.out)" "1"
"$LANE" note -p src/auth.rs -a "fn verfy" "typo anchor" > /dev/null 2>&1
"$LANE" audit > /dev/null
"$LANE" audit > /dev/null
is "the unparsed note survives" \
   "$(grep -rl 'swift: constant time' .context --include='*.md' | grep -vc attic)" "1"
is "check reports it as unverifiable" \
   "$("$LANE" check | awk '/^unverifiable/{print $2}')" "1"
is "a typo in a language we DO parse still evicts" \
   "$(grep -rl 'typo anchor' .context/attic | wc -l | tr -d ' ')" "1"

echo "== 19. shell integration survives failure and survives done =="
setup
PATH="$(dirname "$LANE"):$PATH"
eval "$(command lane shellenv)"

is "new --cd puts only the path on stdout" \
   "$(command lane new probe --cd 2>/dev/null | wc -l | tr -d ' ')" "1"
command lane rm probe --force > /dev/null 2>&1

cd "$TMP/repo"
lane new dup > /dev/null 2>&1
lane new dup > /dev/null 2>&1
is "a failed new leaves the shell where it was" \
   "$PWD" "$(cd "$TMP/.lanes-repo/dup" && pwd -P)"
cd "$TMP/repo"

lane new land > /dev/null 2>&1
echo "fn x() {}" > src/x.rs && git add -A && git commit -qm x > /dev/null
lane done > /dev/null 2>&1
is "done lands the shell in the main worktree" "$PWD" "$(cd "$TMP/repo" && pwd -P)"
is "that directory exists" "$([ -d "$PWD" ] && echo yes || echo no)" "yes"

echo "== 20. a lane carries what git ignores =="
setup
mkdir -p packages/a/node_modules packages/b/node_modules
echo cache > packages/a/node_modules/dep
echo cache > packages/b/node_modules/dep
echo "export const a = 1" > packages/a/index.ts
echo "SECRET=1" > .env
printf 'node_modules/\n.env\n' > .gitignore
git add -A && git commit -qm monorepo

"$LANE" new carry > /tmp/carry.out 2>&1
LP="$TMP/.lanes-repo/carry"
REFLINK=$(grep -c 'reflink: yes' /tmp/carry.out)
want() { [ "$REFLINK" = "1" ] && echo yes || echo no; }
is "a nested node_modules is carried iff reflink" \
   "$([ -f "$LP/packages/a/node_modules/dep" ] && echo yes || echo no)" "$(want)"
is "an ignored file is carried iff reflink" \
   "$([ -f "$LP/.env" ] && echo yes || echo no)" "$(want)"
is "tracked files always come from git" \
   "$([ -f "$LP/packages/a/index.ts" ] && echo yes || echo no)" "yes"
"$LANE" rm carry --force > /dev/null 2>&1

echo "// scratch" >> src/auth.rs
"$LANE" new clean > /tmp/clean.out 2>&1
is "a dirty tree without --dirty warns and names the recovery" \
   "$(grep -c 'lane rm clean && lane new clean --dirty' /tmp/clean.out)" "1"
"$LANE" rm clean --force > /dev/null 2>&1
"$LANE" new carried --dirty > /tmp/carried.out 2>&1
REFLINK=$(grep -c 'reflink: yes' /tmp/carried.out)
want() { [ "$REFLINK" = "1" ] && echo yes || echo no; }
is "--dirty carries the change" \
   "$(grep -c 'scratch' "$TMP/.lanes-repo/carried/src/auth.rs")" "1"

echo "== 21. done refuses before it writes =="
setup
"$LANE" new spike > /dev/null 2>&1
LP="$TMP/.lanes-repo/spike"
( cd "$LP" && printf 'pub fn verify() {\n    lane version\n}\n' > src/auth.rs \
  && git commit -qam "lane work" > /dev/null )
printf 'pub fn verify() {\n    my version\n}\n' > src/auth.rs   # dirty in main, same file
BEFORE=$(git -C "$LP" rev-parse HEAD)
( cd "$LP" && "$LANE" done > /tmp/blocked.out 2>&1 )
is "done refuses" "$(grep -c '^error:' /tmp/blocked.out)" "1"
is "and names the file" "$(grep -c 'src/auth.rs' /tmp/blocked.out)" "1"
is "nothing was committed" "$(git -C "$LP" rev-parse HEAD)" "$BEFORE"
git checkout -- src/auth.rs
( cd "$LP" && "$LANE" done > /dev/null 2>&1 )

echo "== 22. decisions are captured from commit trailers =="
setup
"$LANE" install hooks > /dev/null
git commit -q --allow-empty -m "make verify constant-time

Why: src/auth.rs#fn verify | early return leaks token length"
is "the trailer became a pending note" \
   "$(grep -c 'early return leaks' .git/lane/pending.jsonl)" "1"
"$LANE" audit > /dev/null
is "and promotes like any other note" \
   "$("$LANE" why src/auth.rs | grep -c 'early return leaks')" "1"

git commit -q --allow-empty -m "tidy imports"
is "a commit with no trailer records nothing" \
   "$([ -f .git/lane/pending.jsonl ] && echo yes || echo no)" "no"

git commit -q --allow-empty -m "refactor the parser

Why: refactor the parser" 2> /tmp/cap.out
is "a pasted subject is refused" "$(grep -c 'warning:' /tmp/cap.out)" "1"
is "and records nothing" "$([ -f .git/lane/pending.jsonl ] && echo yes || echo no)" "no"

git commit -q --allow-empty -m "note it twice

Why: src/auth.rs#fn verify | early return leaks token length"
"$LANE" audit > /dev/null
is "an identical note is not duplicated" \
   "$(grep -rl 'early return leaks' .context/- --include='*.md' | wc -l | tr -d ' ')" "1"

echo "== 23. lane install skill =="
setup
"$LANE" install skill > /tmp/skill.out 2>&1
is "the skill lands at the conventional path" \
   "$([ -f .agents/skills/lane/SKILL.md ] && echo yes || echo no)" "yes"
is "it has frontmatter naming the skill" \
   "$(grep -c '^name: lane$' .agents/skills/lane/SKILL.md)" "1"
is "it teaches the Why trailer form" \
   "$(grep -c '^Why: src/auth.rs#fn verify' .agents/skills/lane/SKILL.md)" "1"
is "it teaches lane note with a path" \
   "$(grep -c 'lane note -p ' .agents/skills/lane/SKILL.md)" "1"
"$LANE" install skill > /tmp/skill2.out 2>&1
is "installing twice is a no-op" "$?" "0"
echo "edited by hand" >> .agents/skills/lane/SKILL.md
"$LANE" install skill > /tmp/skill3.out 2>&1
is "an edited skill is not clobbered" "$?" "1"

"$LANE" new skillhome > /dev/null 2>&1
( cd "$TMP/.lanes-repo/skillhome" && "$LANE" install skill > /dev/null 2>&1 )
is "the skill installs into the worktree you ran it from" \
   "$([ -f "$TMP/.lanes-repo/skillhome/.agents/skills/lane/SKILL.md" ] && echo yes || echo no)" "yes"
"$LANE" rm skillhome --force > /dev/null 2>&1

echo "== 24. init repairs a protocol it wrote earlier =="
setup
is "init writes the marked protocol" \
   "$(grep -c 'lane:protocol' AGENTS.md)" "2"
printf '# AGENTS\n\nSome existing house rules for this project.\n' > AGENTS.md
"$LANE" init > /dev/null 2>&1
is "init adds a protocol to unrelated content" \
   "$(grep -c 'lane:protocol' AGENTS.md)" "2"
is "and preserves the unrelated content" \
   "$(grep -c 'Some existing house rules' AGENTS.md)" "1"
cat > AGENTS.md <<'AGENTSEOF'
# AGENTS

## Context memory

- Before editing a file, read `.context/-/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note -a <anchor> "..."`.
- Do not edit `.context/` by hand; `lane done` manages it.
AGENTSEOF
"$LANE" init > /dev/null 2>&1
is "a legacy protocol is upgraded" \
   "$(grep -c 'lane note -p <path>' AGENTS.md)" "1"
is "an upgraded legacy protocol ends with a newline" \
   "$([ -z "$(tail -c 1 AGENTS.md)" ] && echo yes || echo no)" "yes"
printf '# AGENTS\n\n## Context memory\n\n- my own notes, do not touch\n' > AGENTS.md
BEFORE=$(cat AGENTS.md)
"$LANE" init > /dev/null 2>&1
is "an edited protocol is refused, not overwritten" "$(cat AGENTS.md)" "$BEFORE"
is "and the bullet the user wrote is still there" \
   "$(grep -c 'my own notes' AGENTS.md)" "1"

echo
echo "passed: $pass   failed: $fail"
[ "$fail" -eq 0 ]
