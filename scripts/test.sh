#!/usr/bin/env bash
# End-to-end suite for `lane`. Section 6 is the one that matters: two lanes opened
# from the same trunk, both writing memory about the same anchor, must both land.
# The clone layer and anchor resolution are covered by `cargo test`.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
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
  cd "$TMP" && rm -rf repo && mkdir repo && cd repo
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

remote_setup() {
  rm -rf "$TMP/origin.git"
  git init --bare -q "$TMP/origin.git"
  git remote add origin "$TMP/origin.git"
}

echo "== 2. new: warm cache arrives, tracked files from git, status clean =="
setup
"$LANE" new fix-login > /tmp/new.out 2>&1
LP="$TMP/repo/.lane/trees/fix-login"
REFLINK=$(grep -c 'reflink: yes' /tmp/new.out)
want() { [ "$REFLINK" = "1" ] && echo yes || echo no; }
is "lane exists" "$([ -d "$LP" ] && echo yes)" "yes"
is "warm dir present in lane iff reflink" \
   "$([ -f "$LP/node_modules/pkg/blob.bin" ] && echo yes || echo no)" "$(want)"
is "tracked file present" "$([ -f "$LP/src/auth.rs" ] && echo yes)" "yes"
is "lane status clean" "$(git -C "$LP" status --porcelain | wc -l | tr -d ' ')" "0"
is "ls prints the lane name once" \
   "$("$LANE" ls | awk '$1 == "fix-login" && $2 == "open" && $3 == "clean" && $4 == "0" { n++ } END { print n + 0 }')" "1"
is "reflink verdict reported" "$(grep -c 'reflink:' /tmp/new.out)" "1"
is "tracked files not re-cloned" \
   "$(grep -o 'cloned' /tmp/new.out | head -1)" "cloned"

echo "== 3. dirty mode carries dirty state without rewriting files =="
setup
echo "// scratch work" >> src/auth.rs
"$LANE" new spike --dirty > /tmp/dirty.out 2>&1
LP="$TMP/repo/.lane/trees/spike"
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

echo "== 4. note inside lane, merge lands memory on trunk =="
setup
"$LANE" new fix-login > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/fix-login"
cd "$LP"
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
"$LANE" note -p src/auth.rs -a "fn verify" \
  "must be constant-time; early return leaks token length" > /dev/null
git add -A && git commit -qm "refactor verify"
"$LANE" merge > /tmp/merge.out 2>&1
cd "$TMP/repo"
is "trunk advanced" "$(git log --oneline main | grep -c 'refactor verify')" "1"
is "memory committed to trunk" \
   "$(git show main --name-only --format= | grep -c '^\.lane/memory/src/auth\.rs/')" "1"
is "note readable from trunk" \
   "$("$LANE" why src/auth.rs | grep -c 'constant-time')" "1"
is "lane removed" "$([ -d "$TMP/repo/.lane/trees/fix-login" ] && echo yes || echo no)" "no"
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
   "content-changed"
is "#style unaffected" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/c.json"));print([x["tier"] for x in d if x["anchor"]=="#style"][0])')" \
   "fresh"
git add -A && git commit -qm sfc2

echo "== 6. PARALLEL: two lanes, same anchor, both land clean =="
"$LANE" new thread-a > /dev/null 2>&1
"$LANE" new thread-b > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/thread-a" && "$LANE" note -p src/auth.rs -a "fn verify" \
    "a: callers rely on false-on-expiry, not an error" > /dev/null && "$LANE" merge > /tmp/a.out 2>&1 )
is "lane a landed" "$?" "0"
( cd "$TMP/repo/.lane/trees/thread-b" && "$LANE" note -p src/auth.rs -a "fn verify" \
    "b: token parse allocates; hot path, do not add regex" > /dev/null && "$LANE" merge > /tmp/b.out 2>&1 )
is "lane b landed after a, no conflict" "$?" "0"
cd "$TMP/repo"
is "both memories on trunk" \
   "$(git grep -l 'callers rely on false-on-expiry\|do not add regex' main -- .lane | wc -l | tr -d ' ')" "2"

echo "== 7. anchor deleted -> attic =="
sedi 's|pub fn refresh|pub fn rotate_token|' src/auth.rs
"$LANE" note -p src/auth.rs -a "fn refresh" "rotation is idempotent upstream" > /dev/null
"$LANE" audit > /dev/null
is "evicted to attic" "$(find .lane/attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "1"

echo "== 11. rm refuses before it destroys, and --force means everything =="
setup
"$LANE" new scrap > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/scrap" && echo "fn work() {}" > src/work.rs \
  && git add -A && git commit -qm "unlanded work" > /dev/null )
SHA=$(git rev-parse scrap)
"$LANE" rm scrap > /tmp/rm.out 2>&1
is "rm exits non-zero when it kept the lane" "$?" "1"
is "rm names the commits at stake" \
   "$(grep -c 'kept lane scrap: commits main does not have' /tmp/rm.out)" "1"
is "rm names the way through" "$(grep -c 'lane rm scrap --force' /tmp/rm.out)" "1"
is "unlanded branch survives" "$(git branch --list scrap | wc -l | tr -d ' ')" "1"
is "unlanded commit still reachable" "$(git rev-parse scrap)" "$SHA"
# The refusal is the whole point: a --force offered after the worktree already went
# would be offering to recover work that is not there any more.
is "the refused lane is still on disk" \
   "$([ -f "$TMP/repo/.lane/trees/scrap/src/work.rs" ] && echo yes || echo no)" "yes"
"$LANE" rm scrap --force > /tmp/force.out 2>&1
is "--force exits zero" "$?" "0"
is "--force discards the branch" "$(git branch --list scrap | wc -l | tr -d ' ')" "0"
is "--force discards the worktree" \
   "$([ -d "$TMP/repo/.lane/trees/scrap" ] && echo yes || echo no)" "no"

# Memory is the thing a lane holds that no branch can give back.
"$LANE" new notes > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/notes" \
  && "$LANE" note -p src/auth.rs -a "fn verify" "only in the lane" > /dev/null )
"$LANE" rm notes > /tmp/notes.out 2>&1
is "rm counts pending notes as loss" \
   "$(grep -c 'kept lane notes: 1 pending note(s)' /tmp/notes.out)" "1"
is "the lane with notes survives" \
   "$([ -d "$TMP/repo/.lane/trees/notes" ] && echo yes || echo no)" "yes"
"$LANE" rm notes --force > /dev/null 2>&1
is "--force discards the notes with it" \
   "$([ -d "$TMP/repo/.lane/trees/notes" ] && echo yes || echo no)" "no"

"$LANE" new edited > /dev/null 2>&1
echo "// scratch" >> "$TMP/repo/.lane/trees/edited/src/auth.rs"
"$LANE" rm edited > /tmp/edited.out 2>&1
is "rm counts uncommitted work as loss" \
   "$(grep -c 'kept lane edited: 1 uncommitted change(s)' /tmp/edited.out)" "1"
"$LANE" rm edited --force > /dev/null 2>&1
is "--force takes the edit too" \
   "$([ -d "$TMP/repo/.lane/trees/edited" ] && echo yes || echo no)" "no"

# A clean lane holding nothing trunk lacks costs nothing, so no --force is asked for.
"$LANE" new empty > /dev/null 2>&1
"$LANE" rm empty > /tmp/empty.out 2>&1
is "rm takes a lane with nothing at stake" "$?" "0"
is "and says so" "$(grep -c 'removed lane empty' /tmp/empty.out)" "1"

# The squash merge git branch -d always refuses. Lane compares patches, not ancestry.
"$LANE" new squashed > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/squashed" && echo "fn s() {}" > src/s.rs \
  && git add -A && git commit -qm "squash me" > /dev/null )
git merge --squash squashed > /dev/null 2>&1 && git commit -qm "squashed (#1)"
is "git itself refuses the squashed branch" \
   "$(git branch -d squashed > /dev/null 2>&1 && echo deleted || echo refused)" "refused"
"$LANE" rm squashed > /tmp/squash.out 2>&1
is "rm sees the squash and needs no --force" "$?" "0"
is "the squashed branch is gone" "$(git branch --list squashed | wc -l | tr -d ' ')" "0"

# The other half of the same question: a rebase merge replays each commit on its own,
# so a branch of several is landed by patch while no single collapsed diff matches.
"$LANE" new replayed > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/replayed" \
  && echo "fn one() {}" > src/one.rs && git add -A && git commit -qm one > /dev/null \
  && echo "fn two() {}" > src/two.rs && git add -A && git commit -qm two > /dev/null )
git cherry-pick "$(git merge-base main replayed)..replayed" > /dev/null 2>&1
echo "// moved on" >> src/auth.rs && git add -A && git commit -qm "trunk moved" > /dev/null
is "git itself refuses the replayed branch" \
   "$(git branch -d replayed > /dev/null 2>&1 && echo deleted || echo refused)" "refused"
"$LANE" rm replayed > /tmp/replay.out 2>&1
is "rm sees a multi-commit rebase merge" "$?" "0"
is "the replayed branch is gone" "$(git branch --list replayed | wc -l | tr -d ' ')" "0"

"$LANE" rm ghost > /tmp/ghost.out 2>&1
is "rm rejects a name that is neither lane nor branch" "$?" "1"
is "rm says so plainly" "$(grep -c 'no lane ghost' /tmp/ghost.out)" "1"

# A worktree deleted by hand leaves a branch git will not remove a worktree for.
"$LANE" new gone > /dev/null 2>&1
rm -rf "$TMP/repo/.lane/trees/gone"
"$LANE" rm gone --force > /tmp/gone.out 2>&1
is "rm cleans up after a hand-deleted worktree" "$?" "0"
is "with no git fatal" "$(grep -c 'not a working tree' /tmp/gone.out)" "0"
is "and the branch goes with it" "$(git branch --list gone | wc -l | tr -d ' ')" "0"

echo "== 12. init scaffolding and the per-anchor budget =="
setup
is "init creates no merge driver" \
   "$([ -e .gitattributes ] && echo yes || echo no)" "no"
is "AGENTS.md has the protocol" "$(grep -c 'Context memory' AGENTS.md)" "1"
is "init does not touch .gitignore" "$(grep -c 'pending.jsonl' .gitignore)" "0"

for i in 1 2 3 4 5; do
  "$LANE" note -p src/auth.rs -a "fn verify" "filler note number $i about verify" > /dev/null
done
"$LANE" audit > /dev/null
"$LANE" why src/auth.rs -a "fn verify" > /dev/null
"$LANE" audit --max-notes 2 --json > /tmp/budget.json
is "budget caps the anchor at 2 notes" \
   "$(find .lane/memory/src/auth.rs -name '*.md' | wc -l | tr -d ' ')" "2"
is "eviction reason is recorded" \
   "$(python3 -c 'import json;d=json.load(open("/tmp/budget.json"));print(d["evicted"][0]["reason"])')" \
   "budget"
is "evicted notes are recoverable from the attic" \
   "$(find .lane/attic -name '*.md' | wc -l | tr -d ' ')" "3"

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
   "$(grep -rl 'a: alpha\|b: beta' .lane --include='*.md' | wc -l | tr -d ' ')" "2"

echo "== 14. audit is idempotent and a damaged note stays readable =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
"$LANE" audit > /dev/null
is "a no-op audit writes nothing" \
   "$(git status --porcelain -- .lane | wc -l | tr -d ' ')" "0"

F=$(find .lane -name '*.md' -not -path '*attic*' | head -1)
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
   "$(find .lane/attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "0"
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
   "$(find .lane/attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "0"
is "the old directory is gone" \
   "$([ -d .lane/memory/src/auth.rs ] && echo yes || echo no)" "no"
git add -A && git commit -qm memory
git rm -q src/token.rs && git commit -qm delete
"$LANE" audit > /dev/null 2>&1
is "a genuine deletion still evicts" \
   "$(find .lane/attic -name '*.md' 2>/dev/null | wc -l | tr -d ' ')" "1"

echo "== 16. a lane that renames a file lands its memory on the new path =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
"$LANE" new refactor > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/refactor" \
  && git mv src/auth.rs src/token.rs && git commit -qm rename > /dev/null \
  && "$LANE" merge > /tmp/lanemv.out 2>&1 )
cd "$TMP/repo"
is "merge reports the move" "$(grep -c '^  moved   ' /tmp/lanemv.out)" "1"
is "trunk has the note on the new path" \
   "$("$LANE" why src/token.rs 2>/dev/null | grep -c 'constant-time')" "1"

echo "== 17. notes are immutable and nothing derived is committed =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
N=$(find .lane/memory -name '*.md' | head -1)
BEFORE=$(cksum < "$N")
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
"$LANE" audit > /dev/null
is "a drifted note file is not rewritten" "$(cksum < "$N")" "$BEFORE"
is "the note carries no path field" "$(grep -c '^path:' "$N")" "0"
is "the baseline is the note's own frontmatter" "$(grep -c '^body_hash:' "$N")" "1"
is "nothing derived is written" \
   "$([ -e .lane/branch ] && echo yes || echo no)" "no"
is "an audit that finds drift creates no shared log" \
   "$(find .lane -name 'log.jsonl' | wc -l | tr -d ' ')" "0"

mkdir -p attic && echo "user content" > attic/f.txt
git add -A && git commit -qm user-attic
"$LANE" note -p attic/f.txt -a "@file" "a repo may have its own attic" > /dev/null
"$LANE" audit > /dev/null
is "a user path named attic does not collide" \
   "$(find '.lane/memory/attic' -name '*.md' | wc -l | tr -d ' ')" "1"
git add -A && git commit -qm memory

"$LANE" new land > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/land" \
  && "$LANE" note -p src/auth.rs -a "fn verify" "from the lane" > /dev/null \
  && "$LANE" merge > /dev/null 2>&1 )
cd "$TMP/repo"
is "merge leaves no shared landing record" \
   "$(find .lane -name 'log.jsonl' | wc -l | tr -d ' ')" "0"

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
   "$(grep -rl 'swift: constant time' .lane --include='*.md' | grep -vc attic)" "1"
is "check reports it as unverifiable" \
   "$("$LANE" check | awk '/^unverifiable/{print $2}')" "1"
is "a typo in a language we DO parse still evicts" \
   "$(grep -rl 'typo anchor' .lane/attic | wc -l | tr -d ' ')" "1"

echo "== 19. shell integration survives failure and survives merge =="
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
   "$PWD" "$(cd "$TMP/repo/.lane/trees/dup" && pwd -P)"
cd "$TMP/repo"

lane new land > /dev/null 2>&1
echo "fn x() {}" > src/x.rs && git add -A && git commit -qm x > /dev/null
lane merge > /dev/null 2>&1
is "merge lands the shell in the main worktree" "$PWD" "$(cd "$TMP/repo" && pwd -P)"
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
LP="$TMP/repo/.lane/trees/carry"
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
   "$(grep -c 'scratch' "$TMP/repo/.lane/trees/carried/src/auth.rs")" "1"

echo "== 21. merge refuses before it writes =="
setup
"$LANE" new spike > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/spike"
( cd "$LP" && printf 'pub fn verify() {\n    lane version\n}\n' > src/auth.rs \
  && git commit -qam "lane work" > /dev/null )
printf 'pub fn verify() {\n    my version\n}\n' > src/auth.rs   # dirty in main, same file
BEFORE=$(git -C "$LP" rev-parse HEAD)
( cd "$LP" && "$LANE" merge > /tmp/blocked.out 2>&1 )
is "merge refuses" "$(grep -c '^error:' /tmp/blocked.out)" "1"
is "and names the file" "$(grep -c 'src/auth.rs' /tmp/blocked.out)" "1"
is "nothing was committed" "$(git -C "$LP" rev-parse HEAD)" "$BEFORE"
git checkout -- src/auth.rs
( cd "$LP" && "$LANE" merge > /dev/null 2>&1 )

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
   "$(grep -rl 'early return leaks' .lane/memory --include='*.md' | wc -l | tr -d ' ')" "1"

setup
"$LANE" install hooks > /dev/null
git switch -qc replay
printf 'pub fn replayed() {}\n' > src/replayed.rs
git add src/replayed.rs
git commit -q -m "record replay decision

Why: src/replayed.rs | replaying the commit is not a new decision"
is "a replay candidate is captured once" \
   "$(grep -c 'not a new decision' .git/lane/pending.jsonl)" "1"
git switch -q main
printf 'pub fn moved_base() {}\n' > src/base.rs
git add src/base.rs && git commit -qm "move base"
git switch -q replay
git rebase main > /tmp/replay.out 2>&1
is "the decision-bearing commit rebases cleanly" "$?" "0"
is "a replayed commit is not captured again" \
   "$(grep -c 'not a new decision' .git/lane/pending.jsonl)" "1"

setup
"$LANE" install hooks > /dev/null
git commit -q --allow-empty -m "silent commit

Why: src/auth.rs#fn verify | a trailer that should warn when lane is missing" 2>/tmp/nolane.err
# re-run the hook with lane off PATH, to exercise the branch
( PATH=/usr/bin:/bin sh .git/hooks/post-commit ) 2>/tmp/nolane2.err
is "a dropped trailer warns" "$(grep -c 'not on PATH' /tmp/nolane2.err)" "1"
is "and names the recovery" "$(grep -c 'lane capture HEAD' /tmp/nolane2.err)" "1"
git commit -q --allow-empty -m "no trailer here"
( PATH=/usr/bin:/bin sh .git/hooks/post-commit ) 2>/tmp/nolane3.err
is "a commit without a trailer stays silent" "$(wc -c < /tmp/nolane3.err | tr -d ' ')" "0"

echo "== 23. hooks can be upgraded and really removed =="
setup
"$LANE" install hooks > /dev/null
printf '#!/bin/sh\n# lane: capture Why trailers\ncommand -v lane >/dev/null 2>&1 && lane capture HEAD || true\n' > .git/hooks/post-commit
"$LANE" install hooks > /dev/null 2>&1
is "a legacy hook is upgraded" \
   "$(grep -c 'not on PATH' .git/hooks/post-commit)" "1"
printf '#!/bin/sh\necho mine\n# lane: capture Why trailers\ncommand -v lane >/dev/null 2>&1 && lane capture HEAD || true\n' > .git/hooks/post-commit
"$LANE" uninstall hooks > /dev/null 2>&1
is "uninstall keeps the user's own lines" \
   "$(grep -c 'echo mine' .git/hooks/post-commit)" "1"
is "and really removes lane's block" \
   "$(grep -c 'lane capture HEAD' .git/hooks/post-commit)" "0"
rm .git/hooks/post-commit
"$LANE" install hooks > /dev/null 2>&1
"$LANE" uninstall hooks > /dev/null 2>&1
is "a hook that was only lane's is deleted" \
   "$([ -f .git/hooks/post-commit ] && echo yes || echo no)" "no"

echo "== 24. lane install skill =="
setup
"$LANE" install skill > /tmp/skill.out 2>&1
is "the skill lands at the conventional path" \
   "$([ -f .agents/skills/lane/SKILL.md ] && echo yes || echo no)" "yes"
is "it has frontmatter naming the skill" \
   "$(grep -c '^name: lane$' .agents/skills/lane/SKILL.md)" "1"
is "it teaches the Why trailer form" \
   "$(grep -c '^Why: src/auth.rs#fn verify' .agents/skills/lane/SKILL.md)" "1"
is "it teaches lane note with a path" \
   "$(grep -Fc 'lane note -p src/auth.rs -a "fn verify" "must stay constant-time"' .agents/skills/lane/SKILL.md)" "1"
"$LANE" install skill > /tmp/skill2.out 2>&1
is "installing twice is a no-op" "$?" "0"
echo "edited by hand" >> .agents/skills/lane/SKILL.md
"$LANE" install skill > /tmp/skill3.out 2>&1
is "an edited skill is not clobbered" "$?" "1"

"$LANE" new skillhome > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/skillhome" && "$LANE" install skill > /dev/null 2>&1 )
is "the skill installs into the worktree you ran it from" \
   "$([ -f "$TMP/repo/.lane/trees/skillhome/.agents/skills/lane/SKILL.md" ] && echo yes || echo no)" "yes"
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

- Before editing a file, read `.lane/memory/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note -a <anchor> "..."`.
- Do not edit `.lane/` by hand; `lane done` manages it.
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

echo "== 25. a lane lives inside the repo and survives a move =="
setup
RELATIVE_PATHS=$(git worktree add -h 2>&1 | grep -c 'relative-paths')
"$LANE" new moved > /dev/null 2>&1
is "the lane is inside the repo" \
   "$([ -d .lane/trees/moved ] && echo yes || echo no)" "yes"
is "its gitdir pointer is relative when git supports it" \
   "$(grep -c '^gitdir: \.\.' .lane/trees/moved/.git)" "$RELATIVE_PATHS"
is "the main worktree stays clean" "$(git status --porcelain)" ""
( cd "$TMP" && mv repo moved-repo )
MOVED_BRANCH=$(cd "$TMP/moved-repo/.lane/trees/moved" \
  && git rev-parse --abbrev-ref HEAD 2>/dev/null || true)
is "git still works after a move when relative paths are supported" \
   "$MOVED_BRANCH" "$([ "$RELATIVE_PATHS" = "1" ] && echo moved || true)"
( cd "$TMP" && mv moved-repo repo )

echo "== 26. dirty lanes do not carry sibling lanes =="
setup
"$LANE" new first > /dev/null 2>&1
"$LANE" new second > /dev/null 2>&1
echo "// scratch" >> src/auth.rs
"$LANE" new dirty-third --dirty > /dev/null 2>&1
is "a dirty lane contains no other lanes" \
   "$([ -e .lane/trees/dirty-third/.lane/trees ] && echo yes || echo no)" "no"

echo "== 27. unresolved drift stays flagged =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
sedi 's/parse(token).is_valid()/parse(token).is_valid() \&\& true/' src/auth.rs
"$LANE" audit > /dev/null
is "drift survives an audit" \
   "$("$LANE" check --json | python3 -c 'import json,sys; print(sum(x["tier"] == "content-changed" for x in json.load(sys.stdin)))')" "1"
"$LANE" audit > /dev/null
is "and is re-reported by the next audit" \
   "$("$LANE" check --json | python3 -c 'import json,sys; print(sum(x["tier"] == "content-changed" for x in json.load(sys.stdin)))')" "1"
echo "== 28. landings are serialized and marked =="
setup
"$LANE" new solo > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/solo" && echo "work" > src/new.rs && git add -A && git commit -qm "add work" )
( cd "$TMP/repo/.lane/trees/solo" && "$LANE" merge > /dev/null 2>&1 )
is "a landing with no memory change needs no sync commit" \
  "$(git log -1 --format=%s | grep -c '^add work$')" "1"
is "history stayed linear" "$(git log -1 --format=%P | wc -w | tr -d ' ')" "1"

"$LANE" new sq > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/sq" && echo "a" > src/a.rs && git add -A && git commit -qm "one" \
  && echo "b" > src/b.rs && git add -A && git commit -qm "two" )
BEFORE=$(git rev-list --count HEAD)
( cd "$TMP/repo/.lane/trees/sq" && "$LANE" merge --squash > /dev/null 2>&1 )
is "squash lands exactly one commit" \
  "$(( $(git rev-list --count HEAD) - BEFORE ))" "1"
is "and names it merged" \
  "$(git log -1 --format=%s | grep -c '^lane: merged sq$')" "1"
is "and removed the branch" "$(git branch --list sq | wc -l | tr -d ' ')" "0"

echo "== 29. reading context does not modify the tree =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
git add -A .lane && git commit -qm "memory" > /dev/null
"$LANE" why src/auth.rs > /dev/null
is "lane why leaves the tree clean" "$(git status --porcelain)" ""
"$LANE" why src/auth.rs > /dev/null
is "and is still clean when read twice" "$(git status --porcelain)" ""
"$LANE" audit > /dev/null
is "an audit that changes nothing writes nothing" "$(git status --porcelain)" ""

echo "== 30. drift survives a landing =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "callers rely on the parsed shape" > /dev/null
"$LANE" audit > /dev/null
git add -A .lane && git commit -qm memory > /dev/null
"$LANE" new carry > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/carry" \
  && sed 's/parse(token).is_valid()/parse(token).is_valid() \&\& true/' src/auth.rs > t \
  && mv t src/auth.rs && git add -A && git commit -qm "change the span" > /dev/null \
  && "$LANE" audit > /dev/null )
is "a lane preserves the baseline it compared against" \
   "$(cd "$TMP/repo/.lane/trees/carry" && "$LANE" check --json \
      | python3 -c "import json,sys; n=json.load(sys.stdin)[0]; print(int(n['tier']=='content-changed'))")" "1"
( cd "$TMP/repo/.lane/trees/carry" && "$LANE" merge > /dev/null 2>&1 )
is "and the drift survives the landing" \
   "$("$LANE" check --json | python3 -c "import json,sys; print(sum(1 for n in json.load(sys.stdin) if n['tier']=='content-changed'))")" "1"
is "and is still reported by a later audit" \
   "$("$LANE" audit > /dev/null; "$LANE" check --json | python3 -c "import json,sys; print(sum(1 for n in json.load(sys.stdin) if n['tier']=='content-changed'))")" "1"

echo "== 31. holds survives a landing =="
setup
"$LANE" new holds > /dev/null 2>&1
cd "$TMP/repo/.lane/trees/holds"
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
git add -A .lane && git commit -qm memory
sedi 's/parse(token).is_valid()/parse(token).is_valid() \&\& true/' src/auth.rs
git add src/auth.rs && git commit -qm drift
"$LANE" check --json > /tmp/holds-before.json
ID=$(python3 -c 'import json;print(json.load(open("/tmp/holds-before.json"))[0]["id"])')
is "note starts drifted" \
   "$(python3 -c 'import json;print(json.load(open("/tmp/holds-before.json"))[0]["tier"])')" "content-changed"
"$LANE" holds "$ID" > /tmp/holds.out 2>&1
is "holds succeeds" "$?" "0"
is "holds makes the note fresh" \
   "$("$LANE" check --json | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["tier"])')" "fresh"
is "holds clears body drift" \
   "$("$LANE" check | awk '/^content-changed/{print $2}')" "0"
git add -A .lane && git commit -qm holds
( "$LANE" merge > /tmp/holds-merge.out 2>&1 )
cd "$TMP/repo"
is "fresh state and change survive merge" \
   "$("$LANE" check | awk '/^fresh/{print $2}'):$(grep -c '&& true' src/auth.rs)" "1:1"

echo "== 31b. damaged frontmatter is never re-vouched =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
F=$(find .lane/memory -name '*.md' | head -1)
ID=$(basename "$F" | cut -d- -f1)
python3 - "$F" <<'DUP'
import io, sys
p = sys.argv[1]
s = io.open(p, encoding="utf-8").read()
io.open(p, "w", encoding="utf-8").write(s.replace("created:", "created: broken\ncreated:", 1))
DUP
BEFORE=$(cksum < "$F")
is "holds refuses damaged frontmatter" \
   "$("$LANE" holds "$ID" 2>&1 | grep -c 'frontmatter is unreadable')" "1"
is "holds leaves damaged frontmatter byte-identical" "$(cksum < "$F")" "$BEFORE"

echo "== 32. a pushed lane merges elsewhere, then prunes =="
setup
remote_setup
"$LANE" new feat > /dev/null 2>&1
BEFORE=$(git rev-parse main)
( cd "$TMP/repo/.lane/trees/feat" \
  && sedi 's/parse(token).is_valid()/parse(token).is_valid() \&\& fresh()/' src/auth.rs \
  && "$LANE" note -p src/auth.rs -a "fn verify" "the freshness check is not optional" > /dev/null \
  && git add -A && git commit -qm work > /dev/null \
  && "$LANE" push > /tmp/push.out 2>&1 )
cd "$TMP/repo"
is "push leaves trunk where it was" "$(git rev-parse main)" "$BEFORE"
is "push keeps the lane" "$([ -d .lane/trees/feat ] && echo yes || echo no)" "yes"
is "push reaches origin" "$(git --git-dir="$TMP/origin.git" branch --list feat | wc -l | tr -d ' ')" "1"
is "the lane is pushed, not landed" "$("$LANE" ls | grep -c 'feat .*pushed')" "1"

# The merge happens where lane is not: squash, the button that hides every SHA.
git merge -q --squash feat && git commit -qm "squash feat" > /dev/null
is "ls sees the landing through a squash" "$("$LANE" ls | grep -c 'feat .*landed')" "1"
is "the note came with it" \
   "$(find .lane/memory/src/auth.rs -name '*.md' | wc -l | tr -d ' ')" "1"

echo "x" >> .lane/trees/feat/src/auth.rs
is "prune skips a dirty lane" "$("$LANE" prune 2>&1 | grep -c 'skipped feat')" "1"
git -C .lane/trees/feat checkout -- src/auth.rs
is "prune removes it once clean" "$("$LANE" prune 2>&1 | grep -c 'removed feat')" "1"
is "and the lane is gone" "$([ -d .lane/trees/feat ] && echo yes || echo no)" "no"

echo "== 33. two pushed lanes from one base merge in either order =="
setup
remote_setup
"$LANE" new a > /dev/null 2>&1
"$LANE" new b > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/a" \
  && "$LANE" note -p src/auth.rs -a "fn verify" "a: callers rely on false-on-expiry" > /dev/null \
  && "$LANE" push > /dev/null 2>&1 )
( cd "$TMP/repo/.lane/trees/b" \
  && "$LANE" note -p src/auth.rs -a "fn refresh" "b: rotation is not idempotent" > /dev/null \
  && "$LANE" push > /dev/null 2>&1 )
cd "$TMP/repo"
# Neither rebased onto the other: this is what two open pull requests look like.
git merge -q --no-edit a > /dev/null 2>&1
MERGED_B=$(git merge --no-edit b > /tmp/merge-b.out 2>&1; echo $?)
is "the second merges without conflict" "$MERGED_B" "0"
is "both notes survive" \
   "$(find .lane/memory/src/auth.rs -name '*.md' | wc -l | tr -d ' ')" "2"
is "both prepared lanes remain individually prunable" \
   "$("$LANE" ls | grep -c '^[ab] .*landed')" "2"
is "prune collects both" "$("$LANE" prune 2>&1 | grep -c '^removed')" "2"

echo "== 34. a lane can adopt a branch that already exists =="
setup
git branch review-me
"$LANE" new review-me > /dev/null 2>&1
is "the lane is on the existing branch" \
   "$(git -C .lane/trees/review-me rev-parse --abbrev-ref HEAD)" "review-me"
is "no second branch was made" "$(git branch --list 'review-me*' | wc -l | tr -d ' ')" "1"
"$LANE" rm review-me --force > /dev/null 2>&1
git branch -D review-me > /dev/null 2>&1
git branch taken
is "--base is refused for an existing branch" \
   "$("$LANE" new taken --base main 2>&1 | grep -c '^error:')" "1"

echo "== 35. a rebase merge is seen, and unpushed work is not pruned =="
setup
remote_setup
"$LANE" new rb > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/rb" \
  && "$LANE" note -p src/auth.rs -a "fn verify" "rb: the note" > /dev/null \
  && "$LANE" push > /dev/null 2>&1 )
# A rebase merge replays onto a trunk that moved, so every SHA differs and `-d` refuses.
echo "// unrelated" >> src/auth.rs && git add -A && git commit -qm "trunk moved"
git cherry-pick "$(git merge-base main rb)..rb" > /dev/null 2>&1
is "the branch is no longer an ancestor" \
   "$(git merge-base --is-ancestor rb main && echo yes || echo no)" "no"
is "git itself refuses the branch" \
   "$(git branch -d rb > /dev/null 2>&1 && echo deleted || echo refused)" "refused"
is "prune sees through the replay" "$("$LANE" prune 2>&1 | grep -c 'removed rb')" "1"

setup
remote_setup
"$LANE" new after > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/after" \
  && "$LANE" note -p src/auth.rs -a "fn verify" "pushed" > /dev/null \
  && "$LANE" push > /dev/null 2>&1 )
git merge -q --squash after && git commit -qm "squash after" > /dev/null
# Work that arrived after the pull request merged never reached trunk.
( cd "$TMP/repo/.lane/trees/after" \
  && echo "// later" >> src/auth.rs && git add -A && git commit -qm later > /dev/null )
is "prune refuses work trunk does not have" \
   "$("$LANE" prune 2>&1 | grep -c 'skipped after: commits main does not have')" "1"
is "and leaves the lane in place" \
   "$([ -d .lane/trees/after ] && echo yes || echo no)" "yes"

echo "== 36. a landing marks the lane, not the name =="
setup
remote_setup
"$LANE" new fix > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/fix" \
  && "$LANE" note -p src/auth.rs -a "fn verify" "first time round" > /dev/null \
  && "$LANE" merge > /dev/null 2>&1 )
is "landing leaves no committed marker" \
   "$(find .lane -name 'log.jsonl' | wc -l | tr -d ' ')" "0"
# `fix` twice in a week is normal, and the second one has landed nothing.
"$LANE" new fix > /dev/null 2>&1
is "a reused name is not landed" "$("$LANE" ls | grep -c 'fix .*open')" "1"
is "prune leaves it alone" "$("$LANE" prune 2>&1 | grep -c 'no landed lanes')" "1"
is "and it is still on disk" "$([ -d .lane/trees/fix ] && echo yes || echo no)" "yes"

echo "== 37. nothing removes the directory you are standing in =="
setup
"$LANE" new here > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/here" \
  && "$LANE" note -p src/auth.rs -a "fn verify" "n" > /dev/null \
  && "$LANE" push > /dev/null 2>&1 )
git merge -q --squash here && git commit -qm "squash here" > /dev/null
is "prune refuses from inside the lane" \
   "$(cd "$TMP/repo/.lane/trees/here" && "$LANE" prune 2>&1 | grep -c 'cd out first')" "1"
is "rm refuses from inside the lane" \
   "$(cd "$TMP/repo/.lane/trees/here" && "$LANE" rm here --force 2>&1 | grep -c 'cd out first')" "1"
is "the lane is untouched" "$([ -d .lane/trees/here ] && echo yes || echo no)" "yes"
is "and prune still works from the root" "$("$LANE" prune 2>&1 | grep -c 'removed here')" "1"

echo "== 38. push re-pushes safely and reports its state =="
setup
remote_setup
"$LANE" new feat > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/feat"
( cd "$LP" && echo "one" > src/push.rs && git add -A && git commit -qm one && "$LANE" push > /dev/null )
is "push records its base" "$(git config --get lane.feat.base)" "main"
is "ls calls an exact remote tip pushed" "$("$LANE" ls | grep -c 'feat .*pushed')" "1"
( cd "$LP" && echo "two" >> src/push.rs && git add -A && git commit -qm two )
is "a local commit returns ls to open" "$("$LANE" ls | grep -c 'feat .*open')" "1"
( cd "$LP" && "$LANE" push > /dev/null )
echo "base" > src/base.rs && git add -A && git commit -qm base
( cd "$LP" && "$LANE" push > /dev/null )
is "a re-push carries a moved base" "$(git -C "$LP" merge-base --is-ancestor main HEAD && echo yes || echo no)" "yes"
git clone -q --branch feat "$TMP/origin.git" "$TMP/other"
git -C "$TMP/other" config user.email t@t.t
git -C "$TMP/other" config user.name t
echo "remote" > "$TMP/other/remote.rs"
git -C "$TMP/other" add -A && git -C "$TMP/other" commit -qm remote && git -C "$TMP/other" push -q
( cd "$LP" && echo "local" > src/local.rs && git add -A && git commit -qm local && "$LANE" push > /tmp/lease.out 2>&1 )
is "the lease refuses a remote commit" "$?" "1"
is "the lease names the failed push" "$(grep -c 'force-with-lease' /tmp/lease.out)" "1"

echo "== 39. merge warns only for orphaned pull requests =="
setup
remote_setup
"$LANE" new feat > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/feat"
( cd "$LP" && echo "lane" > src/lane.rs && git add -A && git commit -qm lane && "$LANE" push > /dev/null )
REMOTE_TIP=$(git --git-dir="$TMP/origin.git" rev-parse refs/heads/feat)
git --git-dir="$TMP/origin.git" update-ref refs/pull/7/head "$REMOTE_TIP"
echo "base" > src/base.rs && git add -A && git commit -qm base
( cd "$LP" && "$LANE" merge > /tmp/orphan.out 2>&1 )
is "merge warns after rewriting the pushed tip" "$(grep -c '^warning: pushed pull request' /tmp/orphan.out)" "1"
is "the orphan warning names pull request 7" "$(grep -c 'pull request #7' /tmp/orphan.out)" "1"

setup
remote_setup
"$LANE" new clean-pr > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/clean-pr"
( cd "$LP" && echo "lane" > src/lane.rs && git add -A && git commit -qm lane && "$LANE" push > /dev/null )
( cd "$LP" && "$LANE" merge > /tmp/clean-pr.out 2>&1 )
is "merge stays silent when the pushed tip remains reachable" "$(grep -c '^warning: pushed pull request' /tmp/clean-pr.out)" "0"

echo
echo "passed: $pass   failed: $fail"
[ "$fail" -eq 0 ]
