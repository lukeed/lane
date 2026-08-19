# Plan 003: Stop rewriting unchanged notes, so a merge cannot destroy one

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving on. If a STOP condition
> occurs, stop and report. Update this plan's row in `plans/README.md` when done.
>
> **Drift check (run first)**: `git diff --stat 6dc6647..HEAD -- crates/lane/src/audit.rs crates/lane/src/note.rs crates/lane/src/store.rs`

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `6dc6647`, 2026-08-18
- **Supersedes**: the version of this plan written against the Python implementation.
  The rewrite changed the failure mode from silently wrong to loudly destructive.

## Why this matters

Every `lane audit` stamps `checked: <now>` into every note and rewrites the file,
whether or not anything changed. Two branches that both audit therefore produce a
competing one-line edit to the same line of the same file, and `.gitattributes` says
`merge=union`, so git keeps both:

```
status: fresh
checked: 2026-08-19T20:00:00Z
checked: 2026-08-19T10:00:00Z
```

Under the Python implementation the frontmatter parser took the last line and carried on
with a possibly-wrong fingerprint. The Rust rewrite parses frontmatter with
`serde_yaml_ng`, which **rejects duplicate keys**. Verified end to end on a note with a
duplicated `checked:`:

```
$ lane check
warning: .../01M0...-seed-note.md has unreadable frontmatter: duplicate field `checked`
anchor-missing     1
$ lane why src/auth.rs
no context for src/auth.rs
$ lane audit
  evict   #  (anchor missing)
```

So a merged note is now invisible to `lane why`, and the next audit moves it to the
attic under an eviction line with no path and no anchor. It is recoverable, but the
user is not told which note they lost. That is worse than the behaviour this plan was
originally written against, which is why it is P1 rather than P2.

`lane done` still dodges it by rebasing before auditing. `git pull --rebase` on trunk —
which `USAGE.md` tells users to run — does not.

## Current state

- `crates/lane/src/audit.rs` — the loop in `run()` sets `note.meta.checked = now_iso()`
  and then writes unconditionally:

```rust
        note.meta.status = res.tier.into();
        note.meta.checked = now_iso();
        ...
        if let Some(file) = note.file.clone() {
            note.write(&file)?;
        }
```

- `crates/lane/src/note.rs` — `parse()` falls back to a whole-file body with an empty
  `Meta` when YAML fails, which is what produces the empty `#` in the eviction line.
  `Note::render()` serializes `Meta` through `serde_yaml_ng`.
- `crates/lane/src/cli.rs` — `init()` writes the `merge=union` rules.

Nothing reads `meta.checked`. Verify with
`grep -rn 'checked' crates/lane/src/ | grep -v 'checked:'` before relying on it.

Conventions: one-line comments, `anyhow::Result`, tests as `#[cfg(test)] mod tests` at
the end of the file.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Unit + integration | `cargo test` | all pass, baseline + 3 |
| Lint | `cargo clippy --all-targets` | zero warnings |
| Format | `cargo fmt --all --check` | exit 0 |
| End to end | `./test_lane.sh` | `failed: 0`, baseline + 4 |

## Scope

**In scope**: `crates/lane/src/audit.rs`, `note.rs`, `test_lane.sh`.

**Out of scope**:
- The `merge=union` rule itself. It is the design, and this plan removes the spurious
  diffs it was being asked to resolve rather than replacing it. A custom merge driver
  would need installing in every clone, which is the coordination the design refuses.
- `promote_pending` and `evict`. Both write for real reasons.
- The read ledger, which has the same class of problem — plan 009.

## Steps

### Step 1: Keep the bytes a note was parsed from

In `note.rs`, add `pub raw: String` to `Note`, set it from the file contents in `parse()`
(both the success and fallback paths), and default it to `String::new()` in `Note::new`.

**Verify**: `cargo test` → baseline, all pass.

### Step 2: Write only when the render actually changed

Redefine `checked:` as "when this note last changed" — nothing reads it, so the meaning
is free, and what it buys is a byte-identical render across audits of a stable store.

In `audit.rs`, replace the stamp-and-write with:

```rust
        let before = note.render();
        note.meta.status = res.tier.into();
        // ... the existing drift branch, unchanged ...
        // An audit that learned nothing must leave no trace: stamping every note every
        // run gave two branches a competing edit to the same line.
        if note.render() != before {
            note.meta.checked = now_iso();
        }
        if let Some(file) = note.file.clone()
            && note.render() != note.raw
        {
            note.write(&file)?;
        }
```

The second condition also rewrites a note whose on-disk text is not what `render()`
produces, which heals a file carrying merge damage.

**Verify**: in a scratch repo with one note, `lane audit` twice, then
`git status --porcelain -- .context` → empty.

### Step 3: Name the note when its frontmatter is unreadable

`parse()`'s fallback leaves `Meta::default()`, so the path and anchor are empty and the
eviction line reads `evict   #  (anchor missing)`. Recover what the filename already
tells us: notes are written as `<ulid>-<slug>.md` under `.context/<path>/`.

In `parse()`'s fallback, set `id` from the filename stem up to the first `-`, and `path`
from the note file's directory relative to `.context/`. Leave `anchor` empty — it is not
recoverable — and leave the body as the raw text so nothing is lost.

**Verify**: a note with a duplicated key produces an eviction line naming its real path,
not `#`.

### Step 4: Cover it

Add to `test_lane.sh` before the summary, modelled on section 13:

```bash
echo "== N. audit is idempotent and a merged note stays readable =="
setup
"$LANE" note -p src/auth.rs -a "fn verify" "seed: constant time" > /dev/null
"$LANE" audit > /dev/null
git add -A && git commit -qm seed
"$LANE" audit > /dev/null
is "a no-op audit writes nothing" \
   "$(git status --porcelain -- .context | wc -l | tr -d ' ')" "0"

F=$(find .context -name '*.md' -not -path '*attic*' | head -1)
python3 - "$F" <<'PY'
import io, sys
p = sys.argv[1]; s = io.open(p, encoding="utf-8").read()
io.open(p, "w", encoding="utf-8").write(s.replace("checked:", "checked: 2099-01-01T00:00:00Z\nchecked:", 1))
PY
is "a duplicated key does not hide the note" \
   "$("$LANE" why src/auth.rs 2>/dev/null | grep -c 'constant time')" "1"
is "and the eviction line names it" \
   "$("$LANE" audit 2>/dev/null | grep -c 'evict   #')" "0"
```

Also add a `#[cfg(test)]` case in `audit.rs` or `note.rs` asserting that `render()` of a
parsed-then-unmodified note equals its `raw`.

Confirm all three shell assertions fail against the current code first.

**Verify**: `./test_lane.sh` → `failed: 0`, baseline + 4.

## Done criteria

- [ ] `cargo test` passes, baseline + 3; `./test_lane.sh` passes, baseline + 4
- [ ] `cargo clippy --all-targets` → zero warnings; `cargo fmt --all --check` → exit 0
- [ ] Two consecutive `lane audit` runs leave `git status --porcelain -- .context` empty
- [ ] A note with a duplicated frontmatter key is still listed by `lane why`
- [ ] `grep -c 'meta.checked = now_iso()' crates/lane/src/audit.rs` → `1`, inside the
      changed-render branch
- [ ] `plans/README.md` row updated

## STOP conditions

- Making writes conditional breaks any existing assertion. Sections 4, 7, 8 and 12 all
  depend on notes being written during audit for real reasons.
- You find code that reads `meta.checked` and depends on it meaning "last audited".
- The duplicate-key assertion passes before you change anything — then it is not
  reproducing the bug and the rest of the plan has no gate.
- Recovering `path` from the note's directory turns out to be ambiguous for a note whose
  file was moved by hand. Report rather than guessing.

## Maintenance notes

- The invariant to protect: **an audit that learned nothing writes nothing.** Any field
  stamped unconditionally reintroduces this. If something must change every run, it does
  not belong in the note file.
- Resolving duplicate keys rather than rejecting them was considered and rejected: with
  a real YAML parser the ambiguity is a parse error, and the honest fix is to stop
  producing the ambiguity. If duplicates ever arrive from somewhere else, revisit.
- Deferred: `.reads.jsonl` has the same shape of problem. Plan 009.
