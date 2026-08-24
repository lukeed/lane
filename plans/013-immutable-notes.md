# Plan 013: Make note files immutable, and put everything that changes in per-writer files

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition occurs,
> stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 43e404f..HEAD -- crates/lane/src/`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none. No migration: nothing is released and no store exists to carry
  forward.
- **Category**: tech-debt
- **Planned at**: commit `43e404f`, 2026-08-19
- **Supersedes**: plan 009 entirely, and the design plan 003 patched around. 003 stays
  landed; this removes the class rather than the symptom.

## Why this matters

A note file mixes two kinds of data with opposite merge requirements:

| field | changes | written by |
|---|---|---|
| `id` `anchor` `created` `branch` `supersedes`, body | never | the author, once |
| `sig` `body_hash` `lines` `status` `checked` `reviewed` `verdict` `evicted` | most audits | every branch |

`merge=union` is correct for the first and cannot be correct for the second. One file is
being asked to be both append-only and mutable, and every symptom follows: duplicate
frontmatter keys, an audit that rewrites every note, and `.reads.jsonl` with the identical
shape of problem. Plans 003 and 009 are one bug wearing two hats.

Plan 003 landed a patch — skip the write when nothing changed, keep a damaged note inert.
That bounds the blast radius. It does not remove the class: a review verdict, an eviction,
or a `holds` refresh still mutates a shared file on two branches at once.

## The design

```
.context/
  -/<path>/<ulid>-<slug>.md        immutable note; `-` reserves the mirrored tree
  attic/<path>/<ulid>-<slug>.md    the same file, retired, byte-identical
  state/<branch>.json              per-branch cache: fingerprints and read counts
  log/<branch>.jsonl               per-branch record: verdicts and evictions, append-only
```

Three rules produce the whole thing.

**1. A note file is written once and never rewritten.** Unique ULID filenames, no
mutation, so a merge never has to merge one. `pinned` is the single exception; see below.

**2. Everything that changes is per-writer.** One branch, one file, so two branches never
touch the same bytes. `state/` is a disposable cache and may be deleted at any time;
`log/` is a durable record and may not — a verdict cost a model call, and "nothing is
deleted, the attic keeps it inspectable" means keeping the reason. Both are conflict-free
for the same reason; the split between them is durability, not merge behaviour.

**3. `lane done` rolls a lane up into its target.** Per-writer files exist so concurrent
lanes cannot collide while they are alive. When a lane lands, its state and log fold into
the trunk's and its own files are deleted, so nothing accumulates. This is safe for the
same reason the memory commit is: `done` rebases before it audits, so lanes serialise.

### `-` reserves the mirrored tree

Everything under `.context/-/` and `.context/attic/` mirrors user paths; every other name
under `.context/` is ours. Without the reserved directory, a repo with its own root-level
`attic/`, `log/` or `state/` collides with ours. `attic/` needs no `-` of its own — no
name of ours is a sibling of a user path inside it.

### Only the log stays union-merged

`.gitattributes` keeps exactly one rule:

```
.context/log/*.jsonl merge=union
```

That file is genuinely append-only, which is what union merge is for. Notes never conflict
because they are never modified. A `state/` conflict is resolvable by deleting the file,
because it is a cache. A conflict on a note file means two people disagreed about
`pinned`, and that should be loud.

### `pinned` stays in the note file

It is the one field a human edits, deliberately and rarely. The merge behaviour is
acceptable and worth writing down: both branches pin → identical content, no conflict; one
pins and one leaves alone → git takes the pin; one pins and one unpins → a real
disagreement, surfaced as a real conflict. That last case is only loud once `merge=union`
stops covering note files, which this plan does.

### The fingerprint has to survive a grammar upgrade

`sig` and `body_hash` are sha256 of the *normalized* span — comments stripped via
tree-sitter, whitespace collapsed — truncated to 8 bytes. They are not git oids and must
not become them; a blob oid covers the whole file, which throws away the per-anchor
granularity the tool exists for.

Normalization depends on the grammar, so **upgrading a tree-sitter crate can move every
hash**. Today that costs one noisy audit and the fingerprints refresh in place. Under this
plan the creation fingerprint lives in an immutable note and can never be rewritten, so a
version marker alone would only detect the problem, not resolve it.

The note therefore carries two fingerprints:

- `sig` / `body_hash` — normalized, so comment and whitespace churn is not drift
- `raw_hash` — the span's bytes, unnormalized, which does not depend on comment
  classification at all

plus `norm`, the normalization version they were taken under. `check` then has three
cases rather than a guess:

| `norm` | `raw_hash` | outcome |
|---|---|---|
| matches | — | compare normalized hashes; today's behaviour |
| differs | matches | the bytes are identical, so drift is impossible — adopt the new normalized hash silently |
| differs | differs | something moved and we cannot tell what — adopt, and **report it** |

The second row is the common case: most grammar upgrades change how comments are
classified, not where a function starts and ends, so most notes are provably unchanged and
need no attention.

The third row is the honest residue. Span *extents* also come from the grammar, so an
upgrade that moves a node boundary changes `raw_hash` even though the code did not, and
that is indistinguishable from a real edit made in the same window. Those notes are
re-baselined and counted in the audit output — never silently. The alternative, storing the
span text itself so any future normalizer could be applied to both sides, is exact but
embeds a whole file in the note for an `@file` anchor. Not worth it for a once-a-year event.

### On the hash function

sha256 stays. blake3 was measured at these span sizes and is **slower** here — 368-byte
spans on aarch64: 904 MB/s for sha256 against 378 MB/s for blake3, because its tree
parallelism needs kilobytes to pay for its per-call setup, and this machine has SHA
extensions. A whole audit hashes well under a megabyte, so the number is irrelevant either
way; it just removes any argument for a new dependency.

fxhash and rustc-hash are disqualified on a different axis: both are in-memory hashmap
hashes with no stability guarantee across versions. Committing one to disk would mean
fingerprints changing when a dependency is bumped — the exact problem `norm` exists to
handle, self-inflicted, for no gain.

## Current state

- `crates/lane/src/note.rs` — `Meta` carries all fifteen fields; `path_from_location`
  already derives a path from a note's location.
- `crates/lane/src/store.rs` — `CONTEXT_DIR`, `ATTIC`, `READS`, `bump_reads`,
  `read_counts`, `evict`, `Checker`, the tier constants.
- `crates/lane/src/audit.rs` — `run` mutates `status`/`checked`/`sig`/`body_hash`/`lines`
  and writes; `apply_review` mutates `reviewed`/`verdict`/`status` and writes; both call
  `store::evict`, which sets `evicted` before moving the file.
- `crates/lane/src/cli.rs` — `init` writes two `merge=union` rules; `done` fast-forwards
  and removes the lane; `rm` discards one.
- `crates/lane/src/syntax.rs` — `hashes` and `sha`.

Conventions: one-line comments, `anyhow::Result`, `#[cfg(test)] mod tests` at file end.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | baseline + 7 |
| Lint / format | `cargo clippy --all-targets` / `cargo fmt --all --check` | clean |
| End to end | `./scripts/test.sh` | `failed: 0`, baseline + 7 |

At `43e404f` the baselines are 28 and 50.

## Scope

**In scope**: `crates/lane/src/note.rs`, `store.rs`, `audit.rs`, `cli.rs`, `syntax.rs`,
`worktree.rs`, `scripts/test.sh`, `README.md`, `USAGE.md`.

**Out of scope**: rename following (plan 014); the anchor grammar; tier semantics, ranking
weights and the budget; `lane done`'s rebase-then-audit ordering; deleting plan 003's
`unreadable`/`raw` machinery.

## Steps

### Step 1: Move the tree under `-` and unhide our directories

In `store.rs`, replace the path constants:

```rust
pub const CONTEXT_DIR: &str = ".context";
pub const NOTES: &str = "-";        // reserved: everything under it mirrors user paths
pub const ATTIC: &str = "attic";
pub const STATE: &str = "state";
pub const LOG: &str = "log";
```

`note_dir` becomes `root/.context/-/<path>`; `evict`'s destination becomes
`root/.context/attic/<path>`. Update `load_notes`' skip rule, which currently tests for a
leading `.attic` component, and `path_from_location`, which must now skip the `-` or
`attic` component after `.context`.

**Verify**: `cargo test` — the `path_from_location` unit test from plan 003 must be updated
to the new shape and pass.

### Step 2: Split `Meta` in two

Reduce `Meta` to what is written once:

```rust
pub struct Meta {
    pub id: String,
    pub anchor: String,
    pub created: String,
    pub branch: String,
    pub norm: String,       // normalization version these fingerprints were taken under
    pub sig: String,        // at creation; the fallback baseline
    pub body_hash: String,
    pub raw_hash: String,   // unnormalized, so a grammar upgrade can be resolved not guessed
    pub lines: String,
    pub supersedes: String,
    pub pinned: bool,
}
```

Add `Note::path(&self) -> String` returning `path_from_location(file)`, and make every
`meta.path` reader use it.

Add `pub const NORM_VERSION: &str = "1";` next to `sha` in `syntax.rs`, with a one-line
comment saying to bump it whenever a grammar upgrade or a change to `strip_comments` or
`normalize` can move an existing hash. Extend `Source::hashes` to return a third value,
`sha(&self.span_text(span))` — the unnormalized span — and update its two call sites.

**Verify**: `grep -c 'meta.path' crates/lane/src/` → `0`.

### Step 3: A per-branch state file

```rust
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct NoteState {
    pub sig: String,
    pub body_hash: String,
    pub lines: String,
    pub raw_hash: String,
    pub status: String,
    pub checked: String,
    pub norm: String,
    pub reads: u32,
}
```

`load_state(root)` reads every `.context/state/*.json` and keeps, per id, the entry with
the newest `checked`. Ties resolve either way — the value is derived, so a wrong pick costs
one recheck.

`save_state` writes only `state/<slug(current_branch())>.json`, sorted keys for a stable
diff, dropping ids no longer in the live store, and **skips the write when the content is
unchanged** so a no-op audit produces no diff here either.

`bump_reads` and `read_counts` move onto this file; `.reads.jsonl` goes away.

**Verify**: a unit test writing two branch files with different `checked` and asserting
`load_state` takes the newer.

### Step 4: `check` reads the baseline from state, then from the note

The baseline is the newest state entry for that id, falling back to the note's creation
fingerprint. A fresh clone with no `state/` is therefore correct, just noisier on its first
audit.

If the baseline's `norm` differs from `NORM_VERSION`, fall back to `raw_hash` per the table
above: equal means the span's bytes are unchanged and the new normalized hashes are adopted
silently; unequal means adopt but flag, and `audit::report` prints
`re-baselined N note(s) after a normalization change` so it is never silent.

**Verify**: three unit tests — a stale creation fingerprint with a matching state entry
reports `fresh`; an old `norm` with an unchanged `raw_hash` re-baselines silently; an old
`norm` with a changed `raw_hash` re-baselines and is counted.

### Step 5: Audit writes state and log, never notes

In `audit.rs`:

- the main loop accumulates a `HashMap<String, NoteState>` and must not call `note.write`
- `apply_review` records the verdict into the log and the refreshed status into the state
  map; its `superseded` branch still creates a **new** note file, which is a create
- `store::evict` becomes a pure file move plus one appended log line:
  `{"at":...,"kind":"evict","id":...,"path":...,"anchor":...,"reason":...}`
- each verdict appends `{"at":...,"kind":"verdict","id":...,"verdict":...,"reason":...,"replacement":...}`
- `save_state` runs once at the end

**Verify**: after an audit that finds drift, `git status --porcelain -- .context` lists
changes only under `state/`, `log/`, `attic/`, and any newly created note.

### Step 6: Roll up at `done`, discard at `rm`, and GC orphaned caches

- `cli::done`, after the audit and before the memory commit: append `log/<lane>.jsonl` to
  `log/<trunk>.jsonl`, merge `state/<lane>.json` into `state/<trunk>.json` taking the newer
  `checked` per id and summing `reads`, then delete both lane files. This is the rollup
  that keeps the store from accumulating a file per lane forever.
- `cli::rm` deletes `state/<lane>.json` and `log/<lane>.jsonl`. The work was discarded, so
  its record goes with it.
- `audit::run` deletes any `state/*.json` whose branch has no ref
  (`git show-ref --verify --quiet refs/heads/<name>`). Safe because it is a cache. **Never
  do this for `log/`** — that is a record; an orphaned log is folded by `done` or left alone.

**Verify**: after `lane done`, `.context/state/` and `.context/log/` each hold exactly one
file, named for the trunk.

### Step 7: One merge rule, and only one

`cli::init` writes `.context/log/*.jsonl merge=union` and nothing else.

Section 12 of `scripts/test.sh` asserts two rules — change it to one in the same commit.

**Verify**: `lane init` in a fresh repo → `grep -c 'merge=union' .gitattributes` → `1`.

### Step 8: Cover it

Add to `scripts/test.sh` before the summary. Seven assertions:

```bash
echo "== N. notes are immutable; state and log are per-branch and roll up =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
N=$(find .context/- -name '*.md' | head -1)
BEFORE=$(cksum < "$N")
sedi 's|    parse(token).is_valid()|    let p = parse(token);\n    p.is_valid()|' src/auth.rs
"$LANE" audit > /dev/null
is "a drifted note file is not rewritten" "$(cksum < "$N")" "$BEFORE"
is "the fingerprint moved into state" "$(find .context/state -name '*.json' | wc -l | tr -d ' ')" "1"
is "the note carries no path field" "$(grep -c '^path:' "$N")" "0"
is "init writes exactly one merge rule" "$(grep -c 'merge=union' .gitattributes)" "1"
is "a user path named attic does not collide" \
   "$(mkdir -p attic && echo x > attic/f.txt && git add -A && git commit -qm a > /dev/null && \
      "$LANE" note -p attic/f.txt -a "@file" "user attic" > /dev/null && "$LANE" audit > /dev/null && \
      find .context/-/attic -name '*.md' | wc -l | tr -d ' ')" "1"
git add -A && git commit -qm memory
"$LANE" new land > /dev/null 2>&1
( cd "$TMP/.lanes-repo/land" && "$LANE" note -p src/auth.rs -a "fn verify" "from the lane" > /dev/null \
  && "$LANE" done > /dev/null 2>&1 )
cd "$TMP/repo"
is "done rolls the lane's state up" "$(find .context/state -name '*.json' | wc -l | tr -d ' ')" "1"
is "and its log too" "$(find .context/log -name '*.jsonl' | wc -l | tr -d ' ')" "1"
```

Confirm the first, third and fifth fail against the current code before changing it.

**Verify**: `./scripts/test.sh` → `failed: 0`, baseline + 7.

### Step 9: Update the docs

`README.md`'s Layout and Memory sections and `USAGE.md`'s Layout section both show the old
tree and describe two union rules. Rewrite both to the four-directory shape, say that notes
are never modified after they are written, and say that `-` reserves the mirrored tree so a
repo may have its own `attic/`.

**Verify**: `grep -c 'reads.jsonl' README.md USAGE.md` → `0` for both.

## Done criteria

- [ ] `cargo test` passes, baseline + 7; `./scripts/test.sh` passes, baseline + 7
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] An audit that finds drift modifies no existing `.md` file
- [ ] `grep -c 'pub path' crates/lane/src/note.rs` → `0`
- [ ] `lane init` writes exactly one `merge=union` rule, for `log/*.jsonl`
- [ ] After `lane done`, `state/` and `log/` hold one file each, named for the trunk
- [ ] A repo with its own root-level `attic/` gets notes at `.context/-/attic/`
- [ ] Bumping `NORM_VERSION` by hand and re-auditing an unchanged tree re-baselines
      silently and reports nothing; doing it after editing a span reports the count
- [ ] `plans/README.md` rows for 013 and 009 updated

## STOP conditions

- Reconciling two state files needs a rule more complicated than "newest `checked` wins".
  It should not; the value is derived. If it does, the state is carrying something that is
  not derived and belongs in the note.
- The rollup in step 6 conflicts when two lanes land back to back. It should not — `done`
  rebases before it audits, so lanes serialise. If it does, report the sequence.
- Removing `merge=union` from note files produces a conflict in `scripts/test.sh` sections 6
  or 13, which land two branches of memory. Those should be pure adds. A conflict means
  something is still being mutated — report which file.
- GC of orphaned `state/` files deletes one belonging to a branch that exists only on a
  remote. It is a cache, so this is survivable, but report it.

## Maintenance notes

- The invariant to defend in review: **`.context/` holds immutable notes and per-writer
  files. Nothing else.** A new field that changes over time goes in `state/`; a decision
  worth keeping goes in `log/`; a fact that is true forever goes in the note.
- `NORM_VERSION` must be bumped by any grammar upgrade or any change to `strip_comments`
  or `normalize` that can move an existing hash. Under immutability this is not optional:
  the creation fingerprint can never be rewritten, so the version plus `raw_hash` is the
  only way to tell "not comparable" from "drifted". A dependency bump to a `tree-sitter-*`
  crate without a bump here should fail review.
- Filenames stay `<ulid>-<slug>.md`. The ULID gives uniqueness and creation-order sort; the
  slug is what makes `ls` and `grep -rl` on `.context/-/` legible, which is the affordance
  the whole design is sold on. It is immutable derived data, so it cannot drift out of
  sync the way `path:` could. `<ulid>.md` was considered and rejected: it saves nothing and
  turns a directory listing into five opaque identifiers.
- The rollup at `done` is what makes per-writer files cheap. If a future command lands a
  lane by some other route, it owes the same rollup, or accumulation comes back.
- Deferred: an orphaned `log/<branch>.jsonl` from a branch deleted by hand is left in
  place. It is small, append-only, union-merged, and harmless. Folding it would be a
  cross-writer write, which is the thing this design avoids.
