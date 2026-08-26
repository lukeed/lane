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
  git config commit.gpgsign false
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

echo "== 1. anchor discovery and qualification =="
setup
is "anchors human output is canonical and in source order" \
   "$("$LANE" anchors src/auth.rs)" \
   $'@file\t1-7\nfn verify\t1-3\nfn refresh\t5-7'
is "anchors json has exactly the contracted fields and types" \
   "$("$LANE" anchors src/auth.rs --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print(all(set(r)=={"anchor","start","end"} and type(r["anchor"]) is str and type(r["start"]) is int and type(r["end"]) is int for r in d))')" "True"
is "anchors json order and ranges match human output" \
   "$("$LANE" anchors --json src/auth.rs | python3 -c 'import json,sys; print("\n".join(f"{r['"'"'anchor'"'"']}\t{r['"'"'start'"'"']}-{r['"'"'end'"'"']}" for r in json.load(sys.stdin)))')" \
   "$("$LANE" anchors src/auth.rs)"
printf 'func verify() {}\n' > src/Auth.swift
is "an unknown extension reports only the file anchor" \
   "$("$LANE" anchors src/Auth.swift)" $'@file\t1-1'
is "unique shorthand prints its canonical anchor" \
   "$("$LANE" note add src/auth.rs -a verify "shorthand stays canonical")" \
   "noted -> src/auth.rs#fn verify"
"$LANE" audit > /dev/null
is "the promoted shorthand note stores its canonical anchor" \
   "$(grep -R -c '^anchor: fn verify$' .lane/memory/src/auth.rs | awk -F: '{n += $NF} END {print n + 0}')" "1"
cat > src/ambiguous.rs <<'EOF'
fn run() {}
const run: u8 = 1;
EOF
pending_lines() {
  if [ -f .git/lane/pending.jsonl ]; then
    awk 'NF { n++ } END { print n + 0 }' .git/lane/pending.jsonl
  else
    echo 0
  fi
}
BEFORE_AMBIGUOUS=$(pending_lines)
"$LANE" note add src/ambiguous.rs -a run "must choose a declaration" > /tmp/ambiguous.out 2>&1
AMBIGUOUS_STATUS=$?
is "ambiguous shorthand fails and lists both canonical choices" \
   "$([ "$AMBIGUOUS_STATUS" -ne 0 ] && grep -q '^  fn run$' /tmp/ambiguous.out && grep -q '^  const run$' /tmp/ambiguous.out && echo yes)" "yes"
is "an ambiguous note writes no pending record" "$(pending_lines)" "$BEFORE_AMBIGUOUS"

echo "== 2. new: warm cache arrives, tracked files from git, status clean =="
setup
is "ls json is an empty array before a lane exists" \
   "$("$LANE" ls --json | python3 -c 'import json,sys; print(json.load(sys.stdin) == [])')" "True"
"$LANE" new fix-login > /tmp/new.out 2>&1
LP="$TMP/repo/.lane/trees/fix-login"
LP_REAL=$(cd "$LP" && pwd -P)
REFLINK=$(grep -c 'reflink: yes' /tmp/new.out)
want() { [ "$REFLINK" = "1" ] && echo yes || echo no; }
is "lane exists" "$([ -d "$LP" ] && echo yes)" "yes"
is "warm dir present in lane iff reflink" \
   "$([ -f "$LP/node_modules/pkg/blob.bin" ] && echo yes || echo no)" "$(want)"
is "tracked file present" "$([ -f "$LP/src/auth.rs" ] && echo yes)" "yes"
is "lane status clean" "$(git -C "$LP" status --porcelain | wc -l | tr -d ' ')" "0"
is "ls prints the lane name once" \
   "$("$LANE" ls | awk '$1 == "fix-login" && $2 == "open" && $3 == "clean" && $4 == "0" { n++ } END { print n + 0 }')" "1"
is "ls json reports the exact clean open lane row" \
   "$("$LANE" ls --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print(int(d == [{"name":"fix-login","path":sys.argv[1],"branch":"fix-login","state":"open","dirty":False,"pending_notes":0}] and type(d[0]["dirty"]) is bool and type(d[0]["pending_notes"]) is int))' "$LP_REAL")" "1"
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
is "ls json reports the dirty spike row" \
   "$("$LANE" ls --json | python3 -c 'import json,sys; print(sum(r["name"] == "spike" and r["dirty"] is True for r in json.load(sys.stdin)))')" "1"
is "warm dir also carried iff reflink" \
   "$([ -f "$LP/node_modules/pkg/blob.bin" ] && echo yes || echo no)" "$(want)"

echo "== 4. note inside lane, merge lands memory on trunk =="
setup
"$LANE" new fix-login > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/fix-login"
cd "$LP"
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
"$LANE" note add src/auth.rs -a "fn verify" \
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
"$LANE" note add src/Editor.svelte -a "#script" "clear undo stack on doc swap" > /dev/null
"$LANE" note add src/Editor.svelte -a "#style" "auto not scroll; ios safari jank" > /dev/null
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
( cd "$TMP/repo/.lane/trees/thread-a" && "$LANE" note add src/auth.rs -a "fn verify" \
    "a: callers rely on false-on-expiry, not an error" > /dev/null && "$LANE" merge > /tmp/a.out 2>&1 )
is "lane a landed" "$?" "0"
( cd "$TMP/repo/.lane/trees/thread-b" && "$LANE" note add src/auth.rs -a "fn verify" \
    "b: token parse allocates; hot path, do not add regex" > /dev/null && "$LANE" merge > /tmp/b.out 2>&1 )
is "lane b landed after a, no conflict" "$?" "0"
cd "$TMP/repo"
is "both memories on trunk" \
   "$(git grep -l 'callers rely on false-on-expiry\|do not add regex' main -- .lane | wc -l | tr -d ' ')" "2"

echo "== 7. anchor deleted -> attic =="
sedi 's|pub fn refresh|pub fn rotate_token|' src/auth.rs
"$LANE" note add src/auth.rs -a "fn refresh" "rotation is idempotent upstream" > /dev/null
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

# Memory is the thing a lane carries that no branch can give back.
"$LANE" new notes > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/notes" \
  && "$LANE" note add src/auth.rs -a "fn verify" "only in the lane" > /dev/null )
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
  "$LANE" note add src/auth.rs -a "fn verify" "filler note number $i about verify" > /dev/null
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

"$LANE" note add src/auth.rs -a "fn verify" "a note the parent has not promoted" > /dev/null
"$LANE" new inherit > /dev/null 2>&1
is "a fresh lane does not inherit the parent's queue" \
   "$("$LANE" ls | grep -c 'inherit.*0 pending')" "1"
"$LANE" rm inherit --force > /dev/null 2>&1

echo "== 13. two branches writing memory merge without conflict =="
setup
"$LANE" note add src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed

git checkout -qb branch-a
"$LANE" note add src/auth.rs -a "fn verify" "a: alpha" > /dev/null
"$LANE" audit > /dev/null && git add -A && git commit -qm a
git checkout -q main && git checkout -qb branch-b
"$LANE" note add src/auth.rs -a "fn verify" "b: beta" > /dev/null
"$LANE" audit > /dev/null && git add -A && git commit -qm b
git merge -q --no-edit branch-a > /tmp/merge.out 2>&1
is "parallel memory merges without conflict" "$?" "0"
is "both notes survived" \
   "$(grep -rl 'a: alpha\|b: beta' .lane --include='*.md' | wc -l | tr -d ' ')" "2"

echo "== 14. audit is idempotent and a damaged note stays readable =="
setup
"$LANE" note add src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
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
"$LANE" note add src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
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
"$LANE" note add src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
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
"$LANE" note add src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
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
"$LANE" note add attic/f.txt -a "@file" "a repo may have its own attic" > /dev/null
"$LANE" audit > /dev/null
is "a user path named attic does not collide" \
   "$(find '.lane/memory/attic' -name '*.md' | wc -l | tr -d ' ')" "1"
git add -A && git commit -qm memory

"$LANE" new land > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/land" \
  && "$LANE" note add src/auth.rs -a "fn verify" "from the lane" > /dev/null \
  && "$LANE" merge > /dev/null 2>&1 )
cd "$TMP/repo"
is "merge leaves no shared landing record" \
   "$(find .lane -name 'log.jsonl' | wc -l | tr -d ' ')" "0"

echo "== 18. anchors we cannot resolve are kept, not discarded =="
setup
printf 'func verify(_ t: String) -> Bool {\n    return ok(t)\n}\n' > src/Auth.swift
git add -A && git commit -qm swift
"$LANE" note add src/Auth.swift -a "func verify" "swift: constant time" 2> /tmp/w.out
is "note on an unparsed language warns" "$(grep -c 'warning:' /tmp/w.out)" "1"
"$LANE" note add src/auth.rs -a "fn verfy" "typo anchor" > /dev/null 2>&1
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

is "new puts only the path on stdout" \
   "$(command lane new probe 2>/dev/null | wc -l | tr -d ' ')" "1"
is "enter puts only the path on stdout" \
   "$(command lane enter probe 2>/dev/null | wc -l | tr -d ' ')" "1"
is "switch is the same command as enter" \
   "$(command lane switch probe 2>/dev/null)" "$(command lane enter probe 2>/dev/null)"
is "exit prints the main worktree" \
   "$(command lane exit 2>/dev/null)" "$(cd "$TMP/repo" && pwd -P)"
command lane rm probe --force > /dev/null 2>&1

cd "$TMP/repo"
lane new hop > /dev/null 2>&1
cd "$TMP/repo"
lane enter hop
is "enter moves the shell into the lane" \
   "$PWD" "$(cd "$TMP/repo/.lane/trees/hop" && pwd -P)"
lane exit
is "exit moves the shell back to the main worktree" "$PWD" "$(cd "$TMP/repo" && pwd -P)"
lane enter nosuchlane > /dev/null 2>&1
is "a failed enter leaves the shell where it was" "$PWD" "$(cd "$TMP/repo" && pwd -P)"
lane rm hop --force > /dev/null 2>&1

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

echo "== 23b. a stale hook block is named, not left to run =="
setup
"$LANE" init > /tmp/init1.out 2>&1
is "init offers the hooks nobody installed" \
   "$(grep -c 'lane install hooks' /tmp/init1.out)" "1"
"$LANE" install hooks > /dev/null
"$LANE" init > /tmp/init2.out 2>&1
is "init calls a freshly installed hook current" \
   "$(grep -c 'commit hooks are current' /tmp/init2.out)" "1"
"$LANE" check > /dev/null 2>/tmp/check1.err
is "check says nothing about a current hook" \
   "$(wc -c < /tmp/check1.err | tr -d ' ')" "0"
# What an older release left behind: lane's own markers around a body it no longer ships.
sedi 's/lane capture HEAD/lane capture/' .git/hooks/post-commit
"$LANE" check > /dev/null 2>/tmp/check2.err
is "check names the stale hook" \
   "$(grep -c 'post-commit is out of date' /tmp/check2.err)" "1"
is "and names the recovery" \
   "$(grep -c 'lane install hooks' /tmp/check2.err)" "1"
"$LANE" check --json > /tmp/check2.out 2>/dev/null
is "the warning stays off the json document" \
   "$(python3 -c 'import json,sys; print(json.load(open("/tmp/check2.out")) == [])')" "True"
"$LANE" init > /tmp/init3.out 2>&1
is "init names the stale hook too" \
   "$(grep -c 'post-commit is out of date' /tmp/init3.out)" "1"
"$LANE" install hooks > /dev/null
"$LANE" check > /dev/null 2>/tmp/check3.err
is "installing again clears the warning" \
   "$(wc -c < /tmp/check3.err | tr -d ' ')" "0"
printf '#!/bin/sh\necho mine\n' > .git/hooks/post-commit
"$LANE" check > /dev/null 2>/tmp/check4.err
is "a hook that is not lane's is left alone" \
   "$(grep -c 'out of date' /tmp/check4.err)" "0"

echo "== 24. lane install skill =="
setup
"$LANE" install skill > /tmp/skill.out 2>&1
is "the skill lands in the harness-neutral tree" \
   "$([ -f .agents/skills/lane/SKILL.md ] && echo yes || echo no)" "yes"
is "the other spelling is our entry, not the directory a loader scans" \
   "$(readlink .claude/skills/lane)" "../../.agents/skills/lane"
is "so the directory it scans is real" \
   "$([ -L .claude/skills ] && echo link || echo dir)" "dir"
is "and it resolves to the same file" \
   "$([ -f .claude/skills/lane/SKILL.md ] && echo yes || echo no)" "yes"
is "it has frontmatter naming the skill" \
   "$(grep -c '^name: lane$' .agents/skills/lane/SKILL.md)" "1"
is "it teaches the Why trailer form" \
   "$(grep -c '^Why: src/auth.rs#fn verify' .agents/skills/lane/SKILL.md)" "1"
is "it teaches lane note with a path" \
   "$(grep -Fc 'lane note add src/auth.rs -a "fn verify" "must stay constant-time"' .agents/skills/lane/SKILL.md)" "1"
is "it teaches lane push, which is how a protected trunk is reached" \
   "$(grep -c 'lane push' .agents/skills/lane/SKILL.md)" "1"
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

echo "== 24b. the alias is one entry, never the directory =="
setup
mkdir -p .claude/skills/other && echo "someone else" > .claude/skills/other/SKILL.md
"$LANE" install skill > /dev/null 2>&1
is "a skill already installed there survives" \
   "$(grep -c 'someone else' .claude/skills/other/SKILL.md)" "1"
is "and ours arrives beside it" \
   "$(readlink .claude/skills/lane)" "../../.agents/skills/lane"

setup
"$LANE" install skill > /dev/null 2>&1
"$LANE" uninstall skill > /dev/null 2>&1
is "uninstall removes the skill" \
   "$([ -e .agents/skills/lane/SKILL.md ] && echo yes || echo no)" "no"
is "and the alias it created" \
   "$([ -L .claude/skills/lane ] && echo yes || echo no)" "no"
is "and the directory it emptied" \
   "$([ -d .claude/skills ] && echo yes || echo no)" "no"

setup
mkdir -p .claude/skills/other && echo x > .claude/skills/other/SKILL.md
"$LANE" install skill > /dev/null 2>&1
"$LANE" uninstall skill > /dev/null 2>&1
is "but never a directory holding someone else's skill" \
   "$([ -d .claude/skills/other ] && echo yes || echo no)" "yes"

setup
"$LANE" install skill > /dev/null 2>&1
rm .claude/skills/lane && ln -s ../../elsewhere .claude/skills/lane
"$LANE" uninstall skill > /dev/null 2>&1
is "an alias pointing somewhere else is not ours to remove" \
   "$(readlink .claude/skills/lane)" "../../elsewhere"

echo "== 24c. init repairs a protocol it wrote earlier =="
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
   "$(grep -c 'lane note add <path>' AGENTS.md)" "1"
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
"$LANE" note add src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
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
is "why json is an empty array before a note exists" \
   "$("$LANE" why --json | python3 -c 'import json,sys; print(json.load(sys.stdin) == [])')" "True"
"$LANE" note add src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
WHY_NOTE=$(find .lane/memory/src/auth.rs -type f -name '*.md' | head -n 1)
WHY_FULL_ID=$(basename "$WHY_NOTE" | cut -d- -f1)
WHY_ID=$(printf '%s' "$WHY_FULL_ID" | cut -c1-10)
WHY_CREATED=$(awk '/^created:/{print $2; exit}' "$WHY_NOTE")
WHY_DATE=$(awk '/^created:/{print substr($2, 1, 10); exit}' "$WHY_NOTE")
is "lane why uses the compact note format" \
  "$("$LANE" why src/auth.rs)" \
  "[fn verify]
  - $WHY_ID · $WHY_DATE
    must stay constant-time"
is "the whole-store view keeps paths unambiguous" \
  "$("$LANE" why | head -n 1)" "[src/auth.rs#fn verify]"
is "a directory reads every note beneath it" \
  "$("$LANE" why src/)" \
  "[src/auth.rs#fn verify]
  - $WHY_ID · $WHY_DATE
    must stay constant-time"
is "the repository root reads the whole store" \
  "$("$LANE" why . | head -n 1)" "[src/auth.rs#fn verify]"
is "a directory json carries the path of every match" \
  "$("$LANE" why src/ --json | python3 -c 'import json,sys; print([n["path"] for n in json.load(sys.stdin)])')" \
  "['src/auth.rs']"
is "a partial name matches no path" \
  "$("$LANE" why src/au)" "no context for src/au"
is "why json reports the exact full note row" \
  "$("$LANE" why src/auth.rs --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print(int(d == [{"id":sys.argv[1],"path":"src/auth.rs","anchor":"fn verify","created":sys.argv[2],"note":"must stay constant-time"}]))' "$WHY_FULL_ID" "$WHY_CREATED")" "1"
is "why json honors an anchor with no matches" \
  "$("$LANE" why src/auth.rs -a 'fn refresh' --json | python3 -c 'import json,sys; print(json.load(sys.stdin) == [])')" "True"
is "new notes do not track their branch" \
  "$(grep -R '^branch:' .lane/memory .lane/attic 2>/dev/null || true)" ""
git add -A .lane && git commit -qm "memory" > /dev/null
"$LANE" why src/auth.rs --json > /dev/null
is "lane why json leaves the tree clean" "$(git status --porcelain)" ""
"$LANE" why src/auth.rs > /dev/null
is "lane why leaves the tree clean" "$(git status --porcelain)" ""
"$LANE" why src/auth.rs > /dev/null
is "and is still clean when read twice" "$(git status --porcelain)" ""
"$LANE" audit > /dev/null
is "an audit that changes nothing writes nothing" "$(git status --porcelain)" ""

echo "== 30. drift survives a landing =="
setup
"$LANE" note add src/auth.rs -a "fn verify" "callers rely on the parsed shape" > /dev/null
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

echo "== 31. confirmation survives a landing =="
setup
"$LANE" new confirm > /dev/null 2>&1
cd "$TMP/repo/.lane/trees/confirm"
"$LANE" note add src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
"$LANE" audit > /dev/null
git add -A .lane && git commit -qm memory
sedi 's/parse(token).is_valid()/parse(token).is_valid() \&\& true/' src/auth.rs
git add src/auth.rs && git commit -qm drift
"$LANE" check --json > /tmp/holds-before.json
ID=$(python3 -c 'import json;print(json.load(open("/tmp/holds-before.json"))[0]["id"])')
is "note starts drifted" \
   "$(python3 -c 'import json;print(json.load(open("/tmp/holds-before.json"))[0]["tier"])')" "content-changed"
"$LANE" note confirm "$ID" > /tmp/holds.out 2>&1
is "confirm succeeds" "$?" "0"
is "confirm makes the note fresh" \
   "$("$LANE" check --json | python3 -c 'import json,sys;print(json.load(sys.stdin)[0]["tier"])')" "fresh"
is "confirm clears body drift" \
   "$("$LANE" check | awk '/^content-changed/{print $2}')" "0"
git add -A .lane && git commit -qm confirm
( "$LANE" merge > /tmp/holds-merge.out 2>&1 )
cd "$TMP/repo"
is "fresh state and change survive merge" \
   "$("$LANE" check | awk '/^fresh/{print $2}'):$(grep -c '&& true' src/auth.rs)" "1:1"

echo "== 31b. damaged frontmatter is never re-vouched =="
setup
"$LANE" note add src/auth.rs -a "fn verify" "must stay constant-time" > /dev/null
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
is "confirm refuses damaged frontmatter" \
   "$("$LANE" note confirm "$ID" 2>&1 | grep -c 'frontmatter is unreadable')" "1"
is "confirm leaves damaged frontmatter byte-identical" "$(cksum < "$F")" "$BEFORE"
BEFORE_PENDING=$(pending_lines)
"$LANE" note replace "$ID" "replacement must not lose its anchor" > /tmp/damaged-replace.out 2>&1
is "replace refuses damaged frontmatter without changing bytes or pending notes" \
   "$([ "$?" -eq 1 ] && grep -q 'frontmatter is unreadable' /tmp/damaged-replace.out \
      && [ "$(cksum < "$F")" = "$BEFORE" ] && [ "$(pending_lines)" = "$BEFORE_PENDING" ] \
      && echo yes)" "yes"

echo "== 31c. the explicit note lifecycle is guarded and reversible =="
setup
"$LANE" note -p src/auth.rs "legacy" > /tmp/legacy-note.out 2>&1
is "legacy lane note -p exits 2" "$?" "2"
"$LANE" holds 01M0A > /tmp/legacy-holds.out 2>&1
is "legacy lane holds exits 2" "$?" "2"

ADD_OUTPUT=$("$LANE" note add src/auth.rs "whole-file rule" < /dev/null 2> /tmp/add.err)
is "explicit-text add never prompts and stores the file anchor" \
   "$([ "$ADD_OUTPUT" = 'noted -> src/auth.rs#@file' ] \
      && [ ! -s /tmp/add.err ] \
      && python3 -c 'import json; r=json.loads(open(".git/lane/pending.jsonl").readline()); raise SystemExit(0 if r["anchor"]=="@file" else 1)' \
      && echo yes)" "yes"
BEFORE_PENDING=$(pending_lines)
"$LANE" note add src/auth.rs < /dev/null > /tmp/nonterminal.out 2>&1
is "missing text over non-terminal stdin fails without appending" \
   "$([ "$?" -eq 1 ] && grep -q 'pass text explicitly' /tmp/nonterminal.out \
      && [ "$(pending_lines)" = "$BEFORE_PENDING" ] && echo yes)" "yes"

"$LANE" audit > /dev/null
OLD_FILE=$(find .lane/memory/src/auth.rs -name '*.md' | head -1)
OLD_ID=$(basename "$OLD_FILE" | cut -d- -f1)
"$LANE" note edit NOT-A-NOTE < /dev/null > /tmp/nonterminal-edit.out 2>&1
is "note edit refuses non-terminal use with direct-command guidance" \
   "$([ "$?" -eq 1 ] && grep -q 'requires stdin and stderr terminals' /tmp/nonterminal-edit.out \
      && grep -q 'lane note <action>' /tmp/nonterminal-edit.out && echo yes)" "yes"
"$LANE" note replace "$OLD_ID" "replacement rule" > /dev/null
is "replace inherits the predecessor path" \
   "$(python3 -c 'import json; print(json.loads(open(".git/lane/pending.jsonl").readline())["path"])')" \
   "src/auth.rs"
is "replace inherits the predecessor anchor" \
   "$(python3 -c 'import json; print(json.loads(open(".git/lane/pending.jsonl").readline())["anchor"])')" \
   "@file"
is "the predecessor stays live before replacement promotion" \
   "$([ -f "$OLD_FILE" ] && [ ! -d .lane/attic/src/auth.rs ] && echo yes)" "yes"
"$LANE" note replace "$OLD_ID" "duplicate replacement" > /tmp/duplicate-replace.out 2>&1
is "a second pending replacement is refused" \
   "$([ "$?" -eq 1 ] && grep -q 'already has a pending replacement' /tmp/duplicate-replace.out && echo yes)" \
   "yes"
"$LANE" audit > /dev/null
SUCCESSOR_FILE=$(find .lane/memory/src/auth.rs -name '*.md' | head -1)
SUCCESSOR_ID=$(basename "$SUCCESSOR_FILE" | cut -d- -f1)
is "promotion creates the successor and retires the predecessor" \
   "$([ -f "$SUCCESSOR_FILE" ] && grep -q 'replacement rule' "$SUCCESSOR_FILE" \
      && find .lane/attic/src/auth.rs -name "$OLD_ID-*" | grep -q . && echo yes)" "yes"

SUCCESSOR_BYTES=$(cksum < "$SUCCESSOR_FILE")
"$LANE" note retire "$SUCCESSOR_ID" > /dev/null
RETIRED_FILE=$(find .lane/attic/src/auth.rs -name "$SUCCESSOR_ID-*" | head -1)
is "explicit retire moves exact bytes to the attic" \
   "$([ ! -f "$SUCCESSOR_FILE" ] && [ "$(cksum < "$RETIRED_FILE")" = "$SUCCESSOR_BYTES" ] && echo yes)" \
   "yes"
"$LANE" note restore "$SUCCESSOR_ID" > /dev/null
is "restore moves the same bytes back to live memory" \
   "$([ -f "$SUCCESSOR_FILE" ] && [ ! -f "$RETIRED_FILE" ] \
      && [ "$(cksum < "$SUCCESSOR_FILE")" = "$SUCCESSOR_BYTES" ] && echo yes)" "yes"
"$LANE" note replace "$SUCCESSOR_ID" "guarded replacement" > /dev/null
"$LANE" note retire "$SUCCESSOR_ID" > /tmp/guarded-retire.out 2>&1
is "retire is refused while a replacement is pending" \
   "$([ "$?" -eq 1 ] && [ -f "$SUCCESSOR_FILE" ] \
      && grep -q 'pending replacement' /tmp/guarded-retire.out && echo yes)" "yes"

setup
"$LANE" note add src/auth.rs -a "fn verify" "pinned rule" > /dev/null
"$LANE" audit > /dev/null
PIN_FILE=$(find .lane/memory/src/auth.rs -name '*.md' | head -1)
PIN_ID=$(basename "$PIN_FILE" | cut -d- -f1)
"$LANE" note pin "$PIN_ID" > /dev/null
PIN_HASH=$(cksum < "$PIN_FILE")
printf 'pub fn refresh() {}\n' > src/auth.rs
"$LANE" audit > /dev/null
is "pin writes true and protects a missing anchor from eviction" \
   "$([ -f "$PIN_FILE" ] && grep -q '^pinned: true$' "$PIN_FILE" \
      && [ ! -d .lane/attic/src/auth.rs ] && echo yes)" "yes"
"$LANE" note pin "$PIN_ID" > /dev/null
is "pin is idempotent" "$(cksum < "$PIN_FILE")" "$PIN_HASH"
"$LANE" note unpin "$PIN_ID" > /dev/null
is "unpin removes the serialized field" "$(grep -c '^pinned:' "$PIN_FILE")" "0"

setup
"$LANE" note add src/auth.rs -a "fn verify" "confirmed rule" > /dev/null
"$LANE" audit > /dev/null
CONFIRM_FILE=$(find .lane/memory/src/auth.rs -name '*.md' | head -1)
CONFIRM_ID=$(basename "$CONFIRM_FILE" | cut -d- -f1)
sedi 's/parse(token).is_valid()/parse(token).is_valid() \&\& true/' src/auth.rs
"$LANE" note confirm "$CONFIRM_ID" > /dev/null
is "confirm makes a drifted note fresh" \
   "$("$LANE" check --json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["tier"])')" \
   "fresh"

echo "== 32. a pushed lane merges elsewhere, then prunes =="
setup
remote_setup
"$LANE" new feat > /dev/null 2>&1
BEFORE=$(git rev-parse main)
( cd "$TMP/repo/.lane/trees/feat" \
  && sedi 's/parse(token).is_valid()/parse(token).is_valid() \&\& fresh()/' src/auth.rs \
  && "$LANE" note add src/auth.rs -a "fn verify" "the freshness check is not optional" > /dev/null \
  && git add -A && git commit -qm work > /dev/null \
  && "$LANE" push > /tmp/push.out 2>&1 )
cd "$TMP/repo"
is "push leaves trunk where it was" "$(git rev-parse main)" "$BEFORE"
is "push keeps the lane" "$([ -d .lane/trees/feat ] && echo yes || echo no)" "yes"
is "push reaches origin" "$(git --git-dir="$TMP/origin.git" branch --list feat | wc -l | tr -d ' ')" "1"
is "the lane is pushed, not landed" "$("$LANE" ls | grep -c 'feat .*pushed')" "1"
is "ls json reports the pushed state" \
   "$("$LANE" ls --json | python3 -c 'import json,sys; print([r["state"] for r in json.load(sys.stdin) if r["name"] == "feat"][0])')" "pushed"

# The merge happens where lane is not: squash, the button that hides every SHA.
git merge -q --squash feat && git commit -qm "squash feat" > /dev/null
is "ls sees the landing through a squash" "$("$LANE" ls | grep -c 'feat .*landed')" "1"
is "ls json reports the landed state" \
   "$("$LANE" ls --json | python3 -c 'import json,sys; print([r["state"] for r in json.load(sys.stdin) if r["name"] == "feat"][0])')" "landed"
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
  && "$LANE" note add src/auth.rs -a "fn verify" "a: callers rely on false-on-expiry" > /dev/null \
  && "$LANE" push > /dev/null 2>&1 )
( cd "$TMP/repo/.lane/trees/b" \
  && "$LANE" note add src/auth.rs -a "fn refresh" "b: rotation is not idempotent" > /dev/null \
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
  && "$LANE" note add src/auth.rs -a "fn verify" "rb: the note" > /dev/null \
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
  && "$LANE" note add src/auth.rs -a "fn verify" "pushed" > /dev/null \
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
  && "$LANE" note add src/auth.rs -a "fn verify" "first time round" > /dev/null \
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
  && "$LANE" note add src/auth.rs -a "fn verify" "n" > /dev/null \
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
git -C "$TMP/other" config commit.gpgsign false
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


echo "== 40. a retired upstream proves a landing no patch can =="
setup
remote_setup
printf 'one\ntwo\nthree\nfour\nfive\n' > src/ctx.txt
git add -A && git commit -qm ctx
git push -q origin main
"$LANE" new drifted > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/drifted"
( cd "$LP" && sedi 's/^four$/four-lane/' src/ctx.txt && git add -A && git commit -qm "lane edits four" \
  && "$LANE" push > /dev/null 2>&1 )
# main moves first, touching only a context line of the lane's own hunk, then takes the
# lane's change on top of it. That is enough to give the two diffs different patch ids.
sedi 's/^two$/two-main/' src/ctx.txt && git add -A && git commit -qm "main edits two"
sedi 's/^four$/four-lane/' src/ctx.txt && git add -A && git commit -qm "squash drifted"
is "the patch probe alone cannot see the landing" "$("$LANE" ls | grep -c 'drifted .*pushed')" "1"
# The remote retires the branch on its own side, exactly as a merged pull request does.
git --git-dir="$TMP/origin.git" branch -q -D drifted
is "ls does not fetch, so it still reads the cached ref" "$("$LANE" ls | grep -c 'drifted .*pushed')" "1"
git fetch -q --prune
is "a retired upstream reads as landed" "$("$LANE" ls | grep -c 'drifted .*landed')" "1"
is "prune collects it" "$("$LANE" prune 2>&1 | grep -c 'removed drifted')" "1"
is "and the lane is gone" "$([ -d .lane/trees/drifted ] && echo yes || echo no)" "no"

echo "== 41. work added after a landing is counted, not guessed =="
setup
remote_setup
"$LANE" new kept > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/kept"
( cd "$LP" && echo one > src/one.rs && git add -A && git commit -qm one && "$LANE" push > /dev/null 2>&1 )
git merge -q --squash kept > /dev/null && git commit -qm "squash kept" > /dev/null
git --git-dir="$TMP/origin.git" branch -q -D kept && git fetch -q --prune
# The branch keeps committing after its own pull request merged.
( cd "$LP" && echo two > src/two.rs && git add -A && git commit -qm two )
is "prune counts what arrived after the landing" \
   "$("$LANE" prune 2>&1 | grep -c 'kept: 1 commit(s) after landing')" "1"
is "it does not fall back to the uncountable message" \
   "$("$LANE" prune 2>&1 | grep -c 'kept: commits main does not have')" "0"
is "and the lane survives" "$([ -d .lane/trees/kept ] && echo yes || echo no)" "yes"

echo "== 42. an open pull request is never collected =="
setup
remote_setup
"$LANE" new open-pr > /dev/null 2>&1
( cd "$TMP/repo/.lane/trees/open-pr" && echo x > src/x.rs && git add -A && git commit -qm x \
  && "$LANE" push > /dev/null 2>&1 )
is "prune keeps a lane whose branch is still on the remote" \
   "$("$LANE" prune 2>&1 | grep -c 'skipped open-pr: commits main does not have')" "1"
is "the lane survives" "$([ -d .lane/trees/open-pr ] && echo yes || echo no)" "yes"
git remote set-url origin "$TMP/gone.git"
is "a failed fetch warns instead of stopping prune" \
   "$("$LANE" prune 2>&1 | grep -c 'warning: fetch failed')" "1"
is "and the open lane is still kept" \
   "$("$LANE" prune 2>&1 | grep -c 'skipped open-pr')" "1"


echo "== 43. a landing with no recorded tip says so =="
setup
remote_setup
"$LANE" new legacy > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/legacy"
( cd "$LP" && echo one > src/one.rs && git add -A && git commit -qm one && "$LANE" push > /dev/null 2>&1 )
MARK="$TMP/repo/.git/worktrees/legacy/lane/landed"
is "a landing records the tip alongside the id and stamp" "$(wc -w < "$MARK" | tr -d ' ')" "3"
# Roll the marker back to the two-field shape a lane written before tips would carry.
cut -d' ' -f1,2 "$MARK" > "$MARK.old" && mv "$MARK.old" "$MARK"
git --git-dir="$TMP/origin.git" branch -q -D legacy && git fetch -q --prune
is "ls still reads the retired upstream as landed" "$("$LANE" ls | grep -c 'legacy .*landed')" "1"
is "prune names the gap instead of the trunk" \
   "$("$LANE" prune 2>&1 | grep -c 'skipped legacy: landed, later commits unknown')" "1"
is "and never claims main lacks work it has" \
   "$("$LANE" prune 2>&1 | grep -c 'legacy: commits main does not have')" "0"
is "the lane survives" "$([ -d .lane/trees/legacy ] && echo yes || echo no)" "yes"
is "force is the way out" "$("$LANE" rm legacy --force 2>&1 | grep -c 'removed lane legacy')" "1"


echo "== 45. prune names a lane that landed with no record =="
setup
remote_setup
"$LANE" new handlanded > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/handlanded"
( cd "$LP" && echo one > src/one.rs && git add -A && git commit -qm one > /dev/null \
  && git push -q -u origin handlanded 2>/dev/null )
git merge -q --squash handlanded > /dev/null && git commit -qm "squash handlanded" > /dev/null
is "a hand push leaves no landing record" \
   "$([ -e "$TMP/repo/.git/worktrees/handlanded/lane/landed" ] && echo yes || echo no)" "no"
is "so prune passes over it while the branch is still on the remote" \
   "$("$LANE" prune 2>&1 | grep -c 'handlanded')" "0"
git --git-dir="$TMP/origin.git" branch -q -D handlanded
"$LANE" prune > /tmp/prune45.out 2>&1
is "once the remote retires it, prune names it" \
   "$(grep -c 'skipped handlanded: landed outside lane' /tmp/prune45.out)" "1"
is "and never calls the repository clean" \
   "$(grep -c 'no landed lanes' /tmp/prune45.out)" "0"
is "it points at both ways out" \
   "$(grep -c 'lane push <name>' /tmp/prune45.out)" "1"
is "and it does not remove what it cannot account for" \
   "$([ -d .lane/trees/handlanded ] && echo yes || echo no)" "yes"
echo "== 46. a push completes even when the rebase cannot =="
setup
remote_setup
"$LANE" install hooks > /dev/null
printf 'one\ntwo\nthree\n' > src/shared.rs && git add -A && git commit -qm shared
"$LANE" new stale > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/stale"
( cd "$LP" && printf 'one\nLANE\nthree\n' > src/shared.rs \
  && git commit -qam "fix: lane edits two" -m "Why: src/shared.rs#@file | this finding must survive" > /dev/null )
printf 'one\nTRUNK\nthree\n' > src/shared.rs && git commit -qam "trunk edits two too" > /dev/null
is "the finding is queued" \
   "$(cd "$LP" && grep -c 'must survive' "$(git rev-parse --git-path lane/pending.jsonl)")" "1"
"$LANE" push stale > /tmp/stalepush.out 2>&1
is "the push succeeds despite the conflict" "$?" "0"
is "and says where it pushed from" \
   "$(grep -c 'pushing from where this lane forked' /tmp/stalepush.out)" "1"
is "the queue is drained, not stranded" \
   "$(cd "$LP" && [ -e "$(git rev-parse --git-path lane/pending.jsonl)" ] && echo kept || echo drained)" "drained"
is "the note reached the remote with the branch" \
   "$(git --git-dir="$TMP/origin.git" ls-tree -r --name-only stale | grep -c '^\.lane/memory/src/shared.rs/')" "1"
is "the lane is left on its branch, not mid-rebase" \
   "$(cd "$LP" && git rev-parse --abbrev-ref HEAD)" "stale"
is "and nothing is conflicted in it" \
   "$(cd "$LP" && git status --porcelain | grep -c '^UU')" "0"

echo "== 47. a merge still refuses a conflict only you can resolve =="
setup
remote_setup
printf 'one\ntwo\nthree\n' > src/shared.rs && git add -A && git commit -qm shared
"$LANE" new mustfix > /dev/null 2>&1
LP="$TMP/repo/.lane/trees/mustfix"
( cd "$LP" && printf 'one\nLANE\nthree\n' > src/shared.rs && git commit -qam "lane edits two" > /dev/null )
printf 'one\nTRUNK\nthree\n' > src/shared.rs && git commit -qam "trunk edits two too" > /dev/null
"$LANE" merge mustfix > /tmp/mustfix.out 2>&1
is "merge refuses" "$?" "1"
is "and says what to do about it" \
   "$(grep -c 'resolve it, then run the command again' /tmp/mustfix.out)" "1"
is "trunk is not advanced" \
   "$(git log --oneline -1 --format=%s)" "trunk edits two too"
is "and the lane still exists" \
   "$([ -d .lane/trees/mustfix ] && echo yes || echo no)" "yes"

echo
echo "passed: $pass   failed: $fail"
[ "$fail" -eq 0 ]
