# Plan 036: Make the note lifecycle explicit behind one command family

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving to the next step. If anything in
> the "STOP conditions" section occurs, stop and report — do not improvise. When done,
> update this plan's status row in `plans/README.md`, unless a reviewer told you they
> maintain the index.
>
> **Dependency check (run first)**: plans 034 and 035 must be DONE. Confirm that
> `lane why --json` and `lane anchors --json <path>` both work before editing.
>
> **Drift check**:
> `git diff --stat ae4343c..HEAD -- AGENTS.md readme.md crates/lane/src/args.rs crates/lane/src/audit.rs crates/lane/src/cli.rs crates/lane/src/help.rs crates/lane/src/lib.rs crates/lane/src/store.rs crates/lane/tests/args.rs crates/lane-tour/src/scenes.rs scripts/test.sh crates/lane/assets/skill.md www/src/data/commands.ts www/src/pages/index.astro www/src/pages/memory.md www/src/pages/usage.md www/src/scripts/acts.ts`
>
> The completed plans 034 and 035 account for expected changes in shared parser, CLI,
> help, tests, skill, and website files. Compare the live tree with both plans' contracts.
> Any other behavioral mismatch is a STOP condition.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `plans/035-discover-and-qualify-anchors.md` (and transitively plan 034)
- **Category**: direction
- **Planned at**: commit `ae4343c`, 2026-08-24

## Why this matters

Today `lane note` can create or supersede a note, `lane holds` confirms drift, audit can
retire a note, and pinning or restoration require editing `.lane/` by hand. The lifecycle
exists in storage but not as one coherent interface. Put every intentional note mutation
under `lane note`, give retire/restore/pin/unpin first-class verbs, rename the opaque
`holds` judgment to `confirm`, and use plan 035's anchor candidates for a deliberate
interactive creation path.

Lane is not released yet, so this plan performs one clean break. It removes the old
`lane note -p ... [--supersedes]` and `lane holds <id>` spellings instead of carrying
aliases that would double the parser, help, tests, and documentation indefinitely.

## Command contract

The complete public family after this plan is:

```text
lane note add [OPTIONS] <PATH> [TEXT]
lane note replace [OPTIONS] <ID> [TEXT]
lane note confirm <ID>
lane note retire <ID>
lane note restore <ID>
lane note pin <ID>
lane note unpin <ID>
```

`lane note --help` lists the seven verbs. Every `<ID>` accepts the same unambiguous prefix
that `lane why` and `lane check` print. Mutation success lines always include the resolved
full id when an id exists:

```text
noted -> src/auth.rs#fn verify
replacement queued -> 01M0... src/auth.rs#fn verify
confirmed -> 01M0...
retired -> 01M0...
restored -> 01M0...
pinned -> 01M0...
unpinned -> 01M0...
```

### Add

`lane note add <PATH> [TEXT]` makes the path positional. `-a/--anchor` remains optional.

- If `TEXT` is supplied, the command never prompts. An omitted anchor deterministically
  means `@file`, even when a TTY is attached. This is the script/agent-safe path.
- If `TEXT` is omitted, omission is an explicit request for interaction. Both stdin and
  stderr must be terminals; otherwise fail and say to pass text explicitly.
- In interactive mode, an omitted anchor opens the selector described below; an explicit
  anchor is qualified by plan 035 and skips the selector. Then prompt once for note text.
- Empty or whitespace-only interactive text is refused. The existing pending-note and
  delayed-fingerprint behavior is unchanged.

### Replace

`lane note replace <ID> [TEXT]` queues a successor through the existing `supersedes`
field. It inherits the live predecessor's path and anchor. Optional `-p/--path` and
`-a/--anchor` override either value. Text omission prompts only for text; replacement does
not open the anchor selector unless the operator explicitly chose a new path/anchor via
options.

The predecessor stays live until audit promotes the successor. Refuse a second pending
replacement for the same full id, and refuse `retire` while a pending replacement names
that id; either case would otherwise make `promote_pending` fail midway.

### Confirm, retire, restore, pin, and unpin

| Verb | Valid source state | Exact effect |
|---|---|---|
| `confirm` | live | Recompute and write the current fingerprint; the existing `lane holds` behavior. |
| `retire` | live | Pure rename from `.lane/memory/<path>/...` to `.lane/attic/<path>/...`. |
| `restore` | retired | Exact inverse pure rename back to `.lane/memory/<path>/...`. |
| `pin` | live | Set `pinned: true` in that note's frontmatter; no write if already true. |
| `unpin` | live | Set `pinned: false`; serialization omits the false field; no write if already false. |

Retire and restore never rewrite note bytes. Restore warns, but succeeds, when the source
file or anchor no longer resolves; the explicit recovery must not be silently undone by a
preflight. The next audit may retire an unpinned missing anchor again, which the warning
must say. Pin/unpin and confirm refuse unreadable frontmatter.

## Interactive selector contract

Use plan 035's `Source::anchors()` result; do not inspect the tree-sitter tree in terminal
code. Prompts go to stderr so stdout retains the single machine-capturable result line.

For multiple candidates:

```text
Anchor for src/auth.rs:
  1. @file (1-8)
  2. fn verify (1-4)
  3. fn refresh (6-8)
Choose [1-3]:
Note:
```

- `@file` is first because plan 035 guarantees that order.
- A sole candidate is selected without showing a menu; the command goes straight to
  `Note:`.
- Accept only a base-10 number in range. On invalid input, print one concise error and
  prompt again. EOF is an error, not an empty note.
- Read a single line of note text, trim its line ending and surrounding whitespace, and
  reject empty text. Multi-line editing and `$EDITOR` integration are out of scope.
- Do not prompt merely because a TTY exists. Missing `TEXT` is the only interaction signal.

Put pure input/output helpers in a new `crates/lane/src/prompt.rs` module parameterized by
`BufRead` and `Write`, so unit tests can drive menus without a pseudo-terminal. Keep
terminal detection and real stdin/stderr locking in `cli.rs`. Add no prompt dependency.

## Current state

### Parser and dispatch

At `crates/lane/src/args.rs:70-75`, `NoteArgs` requires text/path and already carries
`supersedes`. `Parsed` has `Note(NoteArgs)` and a separate `Holds { id }`. `parse_note`
defaults a missing anchor to `@file`. At `crates/lane/src/cli.rs:51-70`, those variants
dispatch to separate `note(...)` and `holds(...)` functions.

Plan 034 will have added JSON flags to `Ls` and `WhyArgs`; plan 035 will have added the
top-level `Anchors` variant. Preserve all of those shapes while replacing only the note
and holds variants.

### Storage transitions already present

- `store::PendingNote` has `text`, `path`, `anchor`, `at`, and optional `supersedes`.
- `store::promote_pending` creates the successor, then calls `supersede`, which moves the
  predecessor into the attic.
- `store::resolve_id` accepts a live note's full id or unambiguous prefix but never searches
  the attic.
- `store::evict` is already the required pure memory-to-attic rename.
- `note::Meta` already has `pinned: bool`; `audit::eviction_key` and missing-anchor logic
  already honor it.
- `audit::holds` checks the current span and delegates to `store::confirm`, which rewrites
  the baseline and `vouched` timestamp.

Do not add a state database or lifecycle enum. Live versus retired remains encoded by the
note's directory, and `Note::path()` already derives the source path correctly from either
directory.

### Historical word that must remain

`store::fold_legacy_log` recognizes persisted legacy records whose JSON `kind` is
`"holds"`. That string is historical data, not a command spelling. Renaming the public
command must **not** change or remove the legacy `kind == "holds"` branch.

### Documentation blast radius

The old syntax appears in `AGENTS.md`, `readme.md`, built-in help, the embedded skill,
roughly 35 shell-suite calls, the interactive tour, the website command data, home page,
usage and memory pages, and scripted terminal scenes. This is one intentional breaking
migration: every shipped surface moves together, while historical prose about a lane
"holding work" remains ordinary English and must not be mechanically replaced.

## Commands you will need

Record initial test and assertion counts before editing.

| Purpose | Command | Expected on success |
|---|---|---|
| Dependencies | `cargo test --workspace` | plans 034/035 tests pass |
| Parser | `cargo test -p lane --test args` | exit 0; baseline + 7 tests |
| Prompt/storage | `cargo test -p lane --lib` | exit 0; baseline + 12 tests |
| Workspace | `cargo test --workspace` | exit 0; all pass |
| End to end | `./scripts/test.sh` | exit 0; `failed: 0`, baseline + 16 assertions |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, no warnings |
| Format | `cargo fmt --all --check` | exit 0, no diff |
| Website | `cd www && bun run build` | exit 0; Astro check/build pass |

## Scope

**In scope** — the only implementation and documentation files to modify:

- `crates/lane/src/args.rs`
- `crates/lane/src/cli.rs`
- `crates/lane/src/help.rs`
- `crates/lane/src/audit.rs`
- `crates/lane/src/store.rs`
- `crates/lane/src/lib.rs`
- `crates/lane/src/prompt.rs` (new)
- `crates/lane/tests/args.rs`
- `crates/lane-tour/src/scenes.rs`
- `scripts/test.sh`
- `AGENTS.md`
- `crates/lane/assets/skill.md`
- `readme.md`
- `www/src/data/commands.ts`
- `www/src/pages/index.astro`
- `www/src/pages/memory.md`
- `www/src/pages/usage.md`
- `www/src/scripts/acts.ts`

Administrative exception: update only plan 036's row in `plans/README.md` after every
done criterion passes.

**Out of scope**:

- Compatibility aliases for `lane note -p`, `--supersedes`, or top-level `lane holds`.
- `lane note show`, list/search/browse commands, an attic JSON API, or `why --retired`.
  Retired notes remain ordinary tracked Markdown and restore accepts the ids printed before
  or during retirement.
- Editing note body/path/anchor in place. Replacement always creates a successor.
- Pinning or confirming a retired note; restore it first.
- Interactive multi-line editing, `$EDITOR`, fuzzy filtering, arrow-key UI, color, or an
  external terminal/prompt crate.
- Prompting when text was supplied, even if stdin is a terminal.
- Changing `Why:` trailer syntax, capture's pending producer, anchor discovery, JSON
  schemas, hashing, audit ranking, budget, or promotion timing.
- A note schema migration. `pinned`, `supersedes`, and `vouched` already exist.
- Changing legacy `"holds"` log records or unrelated English uses of "holds".

## Git workflow

- Branch: `advisor/036-note-lifecycle`.
- Use Conventional Commit subjects; because the old spellings are removed, a suitable
  primary subject is `break: group note lifecycle commands`.
- Prefer three logical commits: parser/behavior/tests, protocol and shell migration, then
  website/docs.
- Do not push or open a pull request unless instructed.

## Steps

### Step 1: Replace the flat parser with a note command enum

In `args.rs`, replace `NoteArgs` with explicit payloads and a nested enum:

```rust
pub struct NoteAddArgs {
    pub path: String,
    pub text: Option<String>,
    pub anchor: Option<String>,
}

pub struct NoteReplaceArgs {
    pub id: String,
    pub text: Option<String>,
    pub path: Option<String>,
    pub anchor: Option<String>,
}

pub enum NoteCommand {
    Add(NoteAddArgs),
    Replace(NoteReplaceArgs),
    Confirm { id: String },
    Retire { id: String },
    Restore { id: String },
    Pin { id: String },
    Unpin { id: String },
}
```

Make `Parsed::Note(NoteCommand)` the only note mutation variant and remove
`Parsed::Holds`. Remove `holds` from root `COMMANDS`; add a private note-subcommand list
for tailored unknown-verb errors.

Parsing rules:

- `add`: exactly one required path and at most one text positional; `-a/--anchor` only.
- `replace`: exactly one required id and at most one text; optional `-p/--path` and
  `-a/--anchor`.
- the other five: exactly one id and no options.
- `lane note`, unknown verbs, legacy `lane note -p ...`, legacy `--supersedes`, and
  top-level `lane holds` are usage errors.
- `--` keeps its existing meaning so text may start with a dash.
- `lane note --help` opens the family screen; `lane note <verb> --help` opens leaf help.

Expand `Help` for the parent plus seven leaves. Update static usage/error strings together.
Add exactly seven parser tests: family/leaf help; add; replace; five id verbs as one table;
`--` text; missing/extra values; and explicit legacy refusal.

**Verify**: `cargo test -p lane --test args` -> exit 0, baseline + 7 tests.

### Step 2: Make interactive add testable and terminal-safe

Create `prompt.rs` and export it only as `pub(crate)` functionality through `lib.rs`/module
visibility appropriate to the existing binary-library layout. Implement pure helpers for:

- rendering/selecting one `syntax::Anchor` from a slice;
- retrying an invalid numeric choice;
- reading and trimming one non-empty note line; and
- treating EOF as an error.

Add four unit tests: sole-candidate fast path; numbered choice; invalid-then-valid retry;
and empty/EOF text refusal. Assert exact stderr prompt text in these tests.

In `cli.rs`, add one shared `note_text` helper used by add and replace. If text is present,
return it without inspecting terminal state. If absent, require `stdin().is_terminal()` and
`stderr().is_terminal()`, lock both streams, optionally run the selector for add, then read
text. Do not use stdout for prompts.

For `note add`:

- resolve and validate the path exactly as today;
- with supplied text and no anchor, choose `@file` without prompting;
- in interactive mode with no anchor, call `Source::anchors()` once and select;
- with an explicit anchor, use plan 035's `Source::qualify()`; and
- append the same `PendingNote` and preserve `noted -> <path>#<canonical-anchor>`.

**Verify**: `cargo test -p lane prompt::tests` -> exit 0, four tests pass.

### Step 3: Implement replacement as the only public supersede path

Refactor the current note append logic into a private helper taking resolved text, path,
anchor, and optional full predecessor id. `add` passes none. `replace`:

1. resolves the live predecessor prefix immediately;
2. inherits path and anchor unless overridden;
3. validates/qualifies the final target like add;
4. obtains text explicitly or through the text-only prompt;
5. refuses if any current pending record already has that full id in `supersedes`; and
6. appends one `PendingNote` with the full predecessor id.

Add a read-only store helper that parses the pending queue and answers whether a full id is
already named. Malformed lines should retain `promote_pending`'s warning behavior and must
not be rewritten by this check.

Keep successor creation and predecessor retirement inside `promote_pending`. Do not move
fingerprinting to command time and do not retire the old note when merely queuing.

Add two unit tests: one pure target-selection test in `cli.rs` proving inherited
path/anchor and overrides, and one `store.rs` test proving a second pending replacement for
the same predecessor is detected without appending.

**Verify**: focused store/CLI tests pass; `cargo test --workspace` remains green.

### Step 4: Add explicit live/retired storage transitions

In `store.rs`, factor the directory walk so it can load live memory or the attic without
changing `load_notes`'s existing live-only contract. Add:

- `load_retired(root, filter)`;
- `resolve_retired_id(root, prefix)` with state-specific not-found/ambiguity errors;
- `restore(root, note)` as the exact inverse of `evict`;
- `set_pinned(note, bool)` with unreadable checks and no-op detection; and
- the pending-predecessor query from Step 3 if it is not already present.

Both move functions must create only the destination parent, refuse a destination collision,
rename one file, and preserve bytes. Do not delete now-empty directories as part of the
transition.

In `cli.rs`, wire:

- `retire`: resolve live, refuse when pending replacement names it, then `evict`;
- `restore`: resolve retired, warn about missing/unverifiable current target, then restore;
- `pin`/`unpin`: resolve live and call `set_pinned` idempotently.

Add six storage tests: live resolver excludes attic; retired resolver excludes live;
retire/restore preserve exact bytes; ambiguous prefixes are state-local; pin/unpin render
correctly and are idempotent; unreadable notes refuse mutation.

**Verify**: `cargo test -p lane store::tests` -> all pass, baseline + 7 tests across
Steps 3-4.

### Step 5: Rename the human judgment from holds to confirm

Rename `audit::holds` to `audit::confirm`, the private CLI function likewise, and all
current error/output strings from hold/holds to confirm/confirmed. Preserve its exact
semantics: resolve a live note, require a current span, and write the new baseline plus
vouched timestamp.

Update existing audit unit-test names and assertions; do not count mechanical renames as
new tests. Add no new confirmation behavior.

Critically, leave the legacy-log `kind == "holds"` matching and migration test unchanged.
Add an assertion to that test if necessary so a future mechanical rename cannot remove the
historical reader.

**Verify**:

```bash
cargo test -p lane audit::tests store::tests
rg -n 'kind.*holds|"holds"' crates/lane/src/store.rs
```

Expected: tests pass; legacy handling still has matches. Public help/dispatch no longer has
a top-level holds command.

### Step 6: Migrate the complete real-process suite in one pass

Mechanically convert every direct creation in `scripts/test.sh`:

```text
lane note -p <path> -a <anchor> <text>
-> lane note add <path> -a <anchor> <text>

lane note -p <path> -a <anchor> --supersedes <id> <text>
-> lane note replace <id> -p <path> -a <anchor> <text>

lane holds <id>
-> lane note confirm <id>
```

Then update protocol/skill assertions to the new spelling. Do not alter the setup,
ordering, expected memory behavior, or assertion looseness around these calls.

Add one lifecycle section with exactly sixteen new assertions covering:

1. legacy `lane note -p` exits 2;
2. legacy `lane holds` exits 2;
3. explicit-text add without `-a` does not prompt and stores `@file`;
4. missing text over non-terminal stdin fails without appending;
5. replace inherits the predecessor path;
6. replace inherits the predecessor anchor;
7. the old note stays live before promotion;
8. promotion creates the successor and retires the old note;
9. a second pending replacement is refused;
10. explicit retire moves bytes unchanged to the attic;
11. retire is refused while a pending replacement references the id;
12. restore moves the same bytes back to live memory;
13. pin writes `pinned: true` and protects a missing anchor from audit eviction;
14. pin is idempotent (second call leaves the file hash unchanged);
15. unpin removes the serialized field; and
16. confirm makes a drifted note fresh under the new command spelling.

Interactive menu mechanics stay unit-tested in `prompt.rs`; do not introduce a
platform-specific pseudo-terminal dependency into the shell suite.

**Verify**: `./scripts/test.sh` -> `failed: 0`, all pre-existing assertions still pass,
and the count is baseline + 16.

### Step 7: Upgrade the generated protocol and every shipped instruction surface

Update `cli.rs`'s current marked `PROTOCOL` and root `AGENTS.md` to:

```text
- Record non-obvious findings with `lane note add <path> -a <anchor> "..."`.
```

Keep the unmarked `PROTOCOL_V1` byte-for-byte frozen as a historical fingerprint. Marked
protocols already use replacement semantics; update the protocol unit tests so the new
current block is recognized and the previous marked spelling is upgraded. Do not broaden
replacement of unmarked user-authored sections.

Update all command examples and lifecycle prose in:

- `readme.md`;
- `crates/lane/assets/skill.md`;
- `www/src/data/commands.ts` (seven `note <verb>` leaf entries; remove top-level `holds`);
- `www/src/pages/index.astro`;
- `www/src/pages/memory.md`;
- `www/src/pages/usage.md`;
- `www/src/scripts/acts.ts`; and
- `crates/lane-tour/src/scenes.rs` (the executable interactive tour).

Teach the embedded skill these three resolution choices: `note confirm` when still true,
`note replace` when the sentence changes, and `note retire` when the constraint is gone.
Remove its instruction to delete `.lane/` files manually. Mention `restore`, `pin`, and
`unpin` compactly, and document the interactive rule exactly: omit text to opt in; supplied
text never prompts and defaults to `@file` without `-a`.

Do not replace unrelated prose such as "a lane holds commits" or historical legacy-log
terminology.

**Verify**:

```bash
rg -n 'lane note -p|lane holds|--supersedes' AGENTS.md readme.md crates/lane/src/help.rs \
  crates/lane/assets/skill.md crates/lane-tour/src/scenes.rs www/src
rg -n 'lane note -p|lane holds|--supersedes' scripts/test.sh
rg -n 'lane note (add|replace|confirm|retire|restore|pin|unpin)' \
  readme.md crates/lane/src/help.rs crates/lane/assets/skill.md \
  crates/lane-tour/src/scenes.rs www/src
(cd www && bun run build)
```

Expected: the first search has no matches; the second search names only the deliberate
negative tests that prove legacy spellings fail; the third covers the documented family;
website build exits 0.

### Step 8: Run the complete gate and inspect scope

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/test.sh
(cd www && bun run build)
git status --short
```

Expected: every command exits 0; Clippy has no warnings; the shell suite ends with
`failed: 0`; and only in-scope files plus the plan-index status update changed.

## Test plan

- Parser tests lock the nested grammar, help routing, positional path/id/text behavior,
  `--` handling, and deliberate rejection of both legacy surfaces.
- Prompt tests drive exact menus and failure paths through in-memory readers/writers,
  independent of host terminal facilities.
- Store tests prove state-local id resolution, byte-preserving moves, pin serialization,
  idempotence, collision/unreadable refusal, and pending replacement guards.
- Existing audit tests characterize confirmation; the legacy-log test protects historical
  compatibility.
- End-to-end tests exercise every lifecycle transition in a real repository while all
  existing note/audit/merge/push behavior remains green.
- Website build and search gates ensure there is one public spelling everywhere.

## Done criteria

- [ ] The only public note mutations are the seven `lane note <verb>` commands listed in
      the contract.
- [ ] Old `lane note -p`, `--supersedes`, and top-level `lane holds` are rejected; no
      compatibility aliases remain.
- [ ] Supplying text is always non-interactive and defaults an omitted anchor to `@file`.
- [ ] Omitting text is the sole prompt opt-in; non-terminal use fails clearly.
- [ ] Interactive add selects from plan 035's candidates and writes prompts only to stderr.
- [ ] Replace inherits path/anchor, remains pending until audit, and cannot be duplicated
      for one predecessor.
- [ ] Retire/restore are byte-preserving inverse moves with state-local id resolution.
- [ ] Pin/unpin are explicit, idempotent, live-only mutations honored by audit.
- [ ] Confirm preserves the former holds semantics and makes drift fresh.
- [ ] Legacy persisted `"holds"` records remain readable.
- [ ] Parser tests pass at baseline + 7; prompt/storage/audit focused tests pass at
      baseline + 12.
- [ ] `./scripts/test.sh` passes with every old assertion plus 16 new assertions.
- [ ] Protocol, skill, readme, built-in help, website data/pages/scenes, and root AGENTS all
      use the new family consistently.
- [ ] Format, Clippy, workspace tests, and website build pass.
- [ ] No note schema, anchor, audit-ranking, JSON, or pending-promotion redesign landed.
- [ ] `plans/README.md` marks plan 036 DONE only after every other criterion passes.

## STOP conditions

Stop and report instead of improvising if:

- either dependency is not DONE or `Source::anchors` / `Source::qualify` differs from plan
  035's contract;
- any current note lifecycle requires a state outside live, retired, or pending;
- restore cannot be a pure inverse rename because note paths no longer derive from their
  directories;
- replacement would need to retire its predecessor before audit promotion;
- safe pending-replacement guards require rewriting or deleting the pending queue;
- a prompt would occur with explicit text, or terminal behavior requires a new dependency;
- pin/unpin requires a metadata migration rather than the existing `pinned` field;
- renaming the command appears to require changing historical `"holds"` log data;
- an existing behavior test must be weakened instead of mechanically migrated; or
- any verification fails twice after one focused correction.

## Maintenance notes

- Keep the interaction signal syntactic: no text means interactive. TTY presence alone is
  not consent to prompt; agents frequently run under pseudo-terminals.
- Pending replacements deliberately leave the predecessor live until promotion, so a
  failed/rebased lane does not retire shared memory prematurely.
- Live/retired is a location invariant. Any future note browser should use the two store
  loaders rather than infer state from frontmatter.
- Explicit decisions may rewrite a note (`confirm`, `pin`, `unpin`) and should conflict if
  two branches make different judgments. Retire/restore remain pure moves.
- `"holds"` in the legacy log is permanent historical vocabulary even though the CLI says
  `confirm`.
- A future `note show/list` or attic JSON surface is intentionally separate; it needs its
  own schema and agent-read contract rather than being improvised into this migration.
