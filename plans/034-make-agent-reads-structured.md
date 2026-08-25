# Plan 034: Make lane inventory and context machine-readable

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving to the next step. If anything in
> the "STOP conditions" section occurs, stop and report — do not improvise. When done,
> update this plan's status row in `plans/README.md`, unless a reviewer dispatched you and
> said they maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat ae4343c..HEAD -- crates/lane/src/args.rs crates/lane/src/cli.rs crates/lane/src/help.rs crates/lane/tests/args.rs scripts/test.sh crates/lane/assets/skill.md www/src/data/commands.ts www/src/pages/usage.md`
>
> If any in-scope file changed since this plan was written, compare the "Current state"
> excerpts against the live code before proceeding. If the parser variants, `ls()` state
> rules, `why()` filtering/grouping, or existing JSON conventions no longer match, STOP
> and report rather than adapting the schema from memory.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: direction
- **Planned at**: commit `ae4343c`, 2026-08-24

## Why this matters

Lane explicitly supports coding agents, and its memory checks already have JSON output,
but the two read surfaces agents need first do not. An orchestrator must scrape the aligned
text from `lane ls`, and an agent must parse headings and abbreviated ids from `lane why`.
Additive `--json` modes give both callers stable data while leaving the human interface
exactly as it is today.

The JSON schema becomes a compatibility contract. This plan therefore fixes the field
names and empty-result behavior up front, tests all three lane states, and excludes nearby
features rather than allowing the executor to invent fields.

## Current state

### Files and roles

- `crates/lane/src/args.rs` defines the complete parser surface and `Parsed` variants.
- `crates/lane/src/cli.rs` dispatches parsed commands and renders `ls`, `why`, `check`, and
  `audit` output.
- `crates/lane/src/help.rs` owns the built-in help and the usage strings copied into parse
  errors.
- `crates/lane/tests/args.rs` drives the parser as shell words in and one `Parsed` value out.
- `scripts/test.sh` is the end-to-end contract for real repositories and literal output.
- `crates/lane/assets/skill.md` is embedded in the binary and teaches agents the read loop.
- `www/src/data/commands.ts` is the website's command reference.
- `www/src/pages/usage.md` documents multi-agent lane management.

### Parser shape

At `crates/lane/src/args.rs:78-82`, `why` carries only its filters:

```rust
pub struct WhyArgs {
    pub path: Option<String>,
    pub anchor: Option<String>,
}
```

At `crates/lane/src/args.rs:115-126`, `ls` has no payload while `check` already carries a
JSON switch:

```rust
pub enum Parsed {
    Init,
    New(NewArgs),
    Ls,
    // ...
    Why(WhyArgs),
    Holds { id: String },
    Check { json: bool },
```

At `crates/lane/src/args.rs:145-160`, `ls` goes through `bare`, which deliberately accepts
no command-specific option:

```rust
Some("ls") => bare(rest(raw), Help::Ls, Parsed::Ls),
// ...
Some("why") => parse_why(rest(raw)),
Some("holds") => parse_one(rest(raw), Help::Holds, "<ID>", |id| Parsed::Holds { id }),
Some("check") => parse_check(rest(raw)),
```

`parse_check` at `crates/lane/src/args.rs:359-368` is the parser convention to match:

```rust
fn parse_check(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Check));
    }
    let json = pargs.contains("--json");
    none(positionals(pargs, after, Help::Check)?, Help::Check)?;
    Ok(Parsed::Check { json })
}
```

### Lane inventory shape

`worktree::Lane` already exposes the two identifiers at
`crates/lane/src/worktree.rs:159-162`:

```rust
pub struct Lane {
    pub path: PathBuf,
    pub branch: String,
}
```

`ls()` at `crates/lane/src/cli.rs:514-555` computes everything the JSON row in this plan
will expose. The state rules are load-bearing and must not change:

```rust
let state = if store::is_landed(&lane.path)
    && wt::contained_in(&root, &wt::trunk_name(&root), &lane.branch)
{
    "landed"
} else if !upstream.is_empty()
    && try_git(&["rev-parse", "HEAD"], Some(&lane.path)) == upstream
{
    "pushed"
} else {
    "open"
};
println!(
    "{name:<20} {state:<7} {dirty:<6} {} pending note(s)",
    store::pending_count(&lane.path)
);
```

Dirty probes run in parallel and are then zipped back to the original `list_lanes` order.
The note on `cli.rs#fn ls` records why: workers may finish out of order, but their results
must stay paired with that original order. Do not sort only one side or run a second set of
probes for JSON.

The state strings are domain terms. `CONTEXT.md:27-35` names `Open`, `Pushed`, and `Landed`;
the JSON values must be the exact lowercase strings `open`, `pushed`, and `landed` already
printed by the command.

### Context read shape

`why()` at `crates/lane/src/cli.rs:655-697`:

1. resolves an optional repository-relative path;
2. loads live notes only;
3. applies the optional exact anchor filter;
4. groups by `(path, anchor)` in a `BTreeMap`; and
5. sorts each group by full note id before printing ten id characters and a date.

The new JSON mode must use those same live notes and filters. It must not read the attic,
compute freshness, resolve spans, or alter any note. Those are separate product decisions.

### Existing JSON convention

`check_json_rows()` at `crates/lane/src/cli.rs:706-723` establishes these conventions:

```rust
let mut row = serde_json::json!({
    "id": note.meta.id, "path": note.path(),
    "anchor": note.meta.anchor, "tier": res.tier,
    "note": note.body.trim(),
});
```

`check` and `audit` render with `serde_json::to_string_pretty` and print JSON to stdout.
Use the existing `serde` / `serde_json` dependencies; add no dependency and create no new
library API.

## Schemas to implement

These field names, types, and empty-result rules are part of the plan.

### `lane ls --json`

Print one JSON array in the existing `list_lanes` order. Each row has exactly:

```json
{
  "name": "agent-a",
  "path": "/absolute/repo/.lane/trees/agent-a",
  "branch": "agent-a",
  "state": "open",
  "dirty": false,
  "pending_notes": 3
}
```

- `name`: worktree basename, using the same lossy path conversion as human output.
- `path`: the absolute worktree path already held by `worktree::Lane`, rendered with
  `to_string_lossy`.
- `branch`: the actual branch from `worktree::Lane`; keep it even when it equals `name`.
- `state`: exactly `open`, `pushed`, or `landed`, using today's tests and probes.
- `dirty`: JSON boolean, not the human strings `clean` / `dirty`.
- `pending_notes`: JSON integer from `store::pending_count`.
- no lanes: print `[]` and exit 0; do not print `no lanes` in JSON mode.

Do not add base, remote, pull-request, commit, reflink, loss, or memory-freshness fields.
None is already computed by `ls()` for every row, and adding them changes cost and scope.

### `lane why --json`

Print one flat JSON array ordered by `(path, anchor, id)`. Each row has exactly:

```json
{
  "id": "01M0B4KQTX7H3EZ8FE7S6BJ91N",
  "path": "src/auth.rs",
  "anchor": "fn verify",
  "created": "2026-08-19T00:00:00Z",
  "note": "must stay constant-time"
}
```

- `id`: the full id, never the ten-character display prefix.
- `path`: repository-relative note path from `Note::path()`.
- `anchor`: the stored exact anchor.
- `created`: the complete stored timestamp, not only its date.
- `note`: `note.body.trim()`, matching `check --json`'s field name and trimming.
- no matching notes: print `[]` and exit 0; do not print `no context ...` in JSON mode.

Do not add tier or span. `lane check --json` is the freshness read and already owns those
fields. Do not add `pinned`, `vouched`, `supersedes`, hashes, or raw frontmatter.

## Commands you will need

Record the initial test counts before editing. Expected success gates are:

| Purpose | Command | Expected on success |
|---|---|---|
| Parser tests | `cargo test -p lane --test args` | exit 0; baseline + 1 test |
| Rust tests | `cargo test --workspace` | exit 0; all tests pass |
| End to end | `./scripts/test.sh` | exit 0; `failed: 0`, baseline + 9 assertions |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, no warnings |
| Format | `cargo fmt --all --check` | exit 0, no diff |
| Website | `cd www && bun run build` | exit 0; Astro check and build pass |

`scripts/test.sh` builds the debug binary and uses temporary Git repositories. It is the
right place for output-schema coverage; do not replace it with mocked Rust unit tests.

## Scope

**In scope** — the only implementation and documentation files to modify:

- `crates/lane/src/args.rs`
- `crates/lane/src/cli.rs`
- `crates/lane/src/help.rs`
- `crates/lane/tests/args.rs`
- `scripts/test.sh`
- `crates/lane/assets/skill.md`
- `www/src/data/commands.ts`
- `www/src/pages/usage.md`

Administrative exception: after every done criterion passes, update only plan 034's status
row in `plans/README.md`. The index is excluded from the drift check because another plan's
status may legitimately change before this one is executed.

**Out of scope** — do not touch these even if they look adjacent:

- `crates/lane/src/worktree.rs`, `store.rs`, `note.rs`, and `syntax.rs`; consume their
  existing data, do not change storage or anchor behavior.
- `crates/lane/src/lib.rs`; the row types, if any, stay private to `cli.rs`.
- `readme.md`, `CONTEXT.md`, and unrelated stale website/help text.
- JSON output for `path`, `prune`, or any mutating command.
- Attic reads, retire/restore/pin verbs, anchor enumeration, or qualified anchors.
- A global `--json` flag, JSON Lines, schema negotiation, or a plugin/API server.
- Refactoring existing `check --json` or `audit --json` schemas.
- New dependencies.

If formatting touches an in-scope Rust file mechanically, that is expected. If formatting
or a documentation tool changes any out-of-scope file, revert that incidental change before
continuing; do not fold it into this plan.

## Git workflow

- Branch: `advisor/034-structured-agent-reads`.
- Use Conventional Commit subjects, matching recent history such as
  `feat: add lane push command` and `chore: simplify lane why output`.
- Prefer two logical commits: implementation + tests, then agent/site documentation.
- Do not push or open a pull request unless the operator explicitly asks.

## Steps

### Step 1: Add the two parser switches without making JSON global

In `crates/lane/src/args.rs`:

1. Change `Parsed::Ls` to `Parsed::Ls { json: bool }`.
2. Add `pub json: bool` to `WhyArgs`.
3. Replace the `bare` parser path for `ls` with a small `parse_ls` function matching the
   shape of `parse_check`: help wins, `--json` is consumed, and all remaining values or
   flags are refused.
4. Have `parse_why` consume `--json` alongside its existing path and anchor filter.
5. Keep `--json` command-local. `lane --json`, `lane note --json`, and misspellings such as
   `--jsonn` must remain usage errors.

In `crates/lane/src/cli.rs`, pass the parsed booleans to `ls(json)` and
`why(path, anchor, json)`.

In `crates/lane/tests/args.rs`:

- update `commands_without_arguments_take_none` to expect `Parsed::Ls { json: false }`;
- update `why_takes_an_optional_path_and_anchor` to assert `json: false` in both current
  cases; and
- add exactly one test named `structured_read_commands_take_json` covering
  `ls --json`, `why --json`, `why <path> -a <anchor> --json`, and refusal of `--jsonn` on
  both commands.

**Verify**: `cargo test -p lane --test args` → exit 0 and exactly one more test than the
recorded baseline.

### Step 2: Compute lane status once and render it two ways

Refactor `ls()` in `crates/lane/src/cli.rs` to accept `json: bool` and materialize one row
per lane before rendering either format. A small private row struct with
`#[derive(serde::Serialize)]` is preferred because the human renderer can read the same
typed fields; do not expose it through `lib.rs`.

Preserve these behaviors exactly:

- dirty probes remain parallel and zipped to the original lane order;
- only a marked lane pays for `contained_in`;
- an exact upstream tip is `pushed`; a local commit returns it to `open`;
- a marked, contained lane is `landed`;
- human mode prints `no lanes` or the current aligned line byte-for-byte; and
- every state is computed once, not once per renderer.

When `json` is true, serialize the exact `lane ls --json` schema above with
`serde_json::to_string_pretty`. When false, use the current output path unchanged.

Do not parallelize the upstream or containment probes as part of this plan. That is a
performance change with ordering and process-cost implications, not required for JSON.

**Verify**: `cargo test --workspace` → exit 0. Then run `cargo fmt --all --check` → exit 0.

### Step 3: Flatten filtered context into deterministic JSON

Refactor `why()` in `crates/lane/src/cli.rs` to accept `json: bool`.

Keep path resolution, live-note loading, and anchor filtering shared. After filters:

- in JSON mode, sort notes by `(Note::path(), meta.anchor, meta.id)`, map them to the exact
  five-field schema above, print the pretty JSON array, and return 0;
- in human mode, retain the current empty message, `BTreeMap` headings, blank lines,
  ten-character ids, dates, body indentation, and exit code.

The JSON branch must occur before the human empty-result message so zero matches produce
`[]`. Do not call `Checker`, resolve a span, load the attic, or write to the store.

**Verify**: `cargo test --workspace` → exit 0. `git diff -- crates/lane/src/cli.rs` must show
no textual change to the existing human format strings.

### Step 4: Lock both schemas down end to end

Extend `scripts/test.sh` using its existing `is` helper and Python's standard-library
`json` module. Add exactly nine assertions:

1. In section 2, before creating a lane, `lane ls --json` parses as an empty array.
2. In section 2, after `fix-login` exists, its JSON row has exactly the six specified keys
   and the expected name, absolute path, branch, `open` state, `dirty: false`, and zero
   pending notes.
3. In section 3, the `spike` row reports `dirty: true` after `--dirty` carries the edit.
4. In section 32, the prepared `feat` row reports `state == "pushed"`.
5. In section 32, after the squash merge, that same row reports `state == "landed"`.
6. In section 29, before recording a note, `lane why --json` parses as an empty array.
7. In section 29, after promotion, `lane why src/auth.rs --json` returns one row with
   exactly the five specified keys, the full id from the note filename, full `created`
   timestamp, expected path/anchor, and exact note text.
8. In section 29, `lane why src/auth.rs -a 'fn refresh' --json` returns an empty array.
9. In section 29, after the memory commit, running `lane why src/auth.rs --json` leaves
   `git status --porcelain` empty.

Use `python3 -c 'import json,sys; ...'` as the existing suite does. Assert exact key sets so
an executor cannot accidentally publish extra metadata. Keep the existing human-output
assertions at `scripts/test.sh:59-60`, `649-655`, and the state assertions in sections 32,
33, 36, and 38 unchanged; they are the regression proof that the additive mode did not
alter human output.

**Verify**: `./scripts/test.sh` → exit 0, `failed: 0`, and exactly nine more passed
assertions than the recorded baseline.

### Step 5: Document the machine contract where agents find it

Update only the `ls` and `why` surfaces:

- `crates/lane/src/help.rs`: make `Help::Ls.usage()` say `lane ls [--json]`; add `--json`
  to the `LS` and `WHY` option lists and describe it as machine-readable JSON.
- `crates/lane/assets/skill.md`: tell agents that `lane why --json` returns full ids and
  note fields, while `lane ls --json` is the reliable multi-lane inventory. Do not change
  the short `PROTOCOL` in `cli.rs`.
- `www/src/data/commands.ts`: add `--json` options to only `ls` and `why`, update their
  usage strings, and give each a compact valid JSON example matching the fixed schemas.
- `www/src/pages/usage.md`: in "Working with agents", add a short machine-reader example
  for `lane ls --json` and mention `lane why <path> --json`. Do not rewrite the surrounding
  lifecycle explanation.

Do not fix unrelated stale prose encountered in these files. This plan's review must be
able to distinguish schema documentation from a general documentation sweep.

**Verify**:

```bash
rg -n -- '--json' crates/lane/src/help.rs crates/lane/assets/skill.md \
  www/src/data/commands.ts www/src/pages/usage.md
cd www && bun run build
```

Expected: both commands are documented in all four surfaces; the website command exits 0
after Astro check and build.

### Step 6: Run the complete gate and inspect scope

Run, in order:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/test.sh
(cd www && bun run build)
git status --short
```

Expected: every command exits 0; Clippy prints no warnings; the shell suite ends with
`failed: 0`; and `git status --short` names only the in-scope files plus this plan's
status-only index update.

## Test plan

- Parser coverage in `crates/lane/tests/args.rs` proves both flags default off, parse in
  every supported position, coexist with `why` filters, and reject near-miss flags.
- End-to-end coverage in `scripts/test.sh` proves the exact key sets and types, empty arrays,
  full note ids/timestamps, open/pushed/landed states, dirty state, pending count, filtering,
  and pure-read behavior.
- Existing literal human-output tests remain unchanged and passing. Do not replace them
  with looser grep-only assertions.
- Website build proves the TypeScript command data and Astro pages remain valid.

## Done criteria

All boxes must be true:

- [ ] `lane ls` human output is byte-for-byte unchanged in existing assertions.
- [ ] `lane ls --json` emits only the six documented fields with correct JSON types.
- [ ] `lane ls --json` represents open, pushed, landed, clean/dirty, pending, and empty cases.
- [ ] `lane why` human output is byte-for-byte unchanged in existing assertions.
- [ ] `lane why --json` emits only the five documented fields, full ids/timestamps, and
      deterministic `(path, anchor, id)` order.
- [ ] `lane why --json` honors path and anchor filters, emits `[]` for no matches, and does
      not dirty the tree.
- [ ] `cargo test -p lane --test args` passes at baseline + 1 test.
- [ ] `./scripts/test.sh` passes at baseline + 9 assertions and `failed: 0`.
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
      and `cargo test --workspace` all exit 0.
- [ ] `cd www && bun run build` exits 0.
- [ ] No dependency or public library API was added.
- [ ] No file outside Scope changed, other than the permitted `plans/README.md` status row.
- [ ] `plans/README.md` marks plan 034 DONE after every other criterion passes.

## STOP conditions

Stop and report instead of improvising if:

- the live parser variants, `ls()` state checks, or `why()` grouping differ from the
  excerpts above;
- producing either schema appears to require a storage migration, an added dependency, or
  a change to `worktree::Lane`, `Note`, or another public library type;
- an existing human-output assertion changes or must be weakened to pass;
- `ls --json` would run more Git/state probes per lane than human `ls`;
- deterministic `why` output cannot be achieved from the already-loaded live notes;
- supporting a path requires a non-UTF-8 policy beyond the existing lossy CLI rendering;
- an exact schema assertion suggests adding an unplanned field; or
- any gate fails twice after one reasonable correction.

## Maintenance notes

- Treat both JSON key sets and types as public compatibility contracts. Additive fields
  still break consumers that validate exact schemas, so expand them only in a deliberate
  future plan with a migration note.
- Keep `why --json` a context read. Freshness, tier, and current span remain owned by
  `check --json`; attic lifecycle remains a separate feature.
- Keep `ls`'s state probes shared between renderers. A future performance change must
  preserve row/result pairing and the open/pushed/landed semantics.
- Reviewers should scrutinize empty-result output, full versus abbreviated ids, boolean
  versus display-string dirtiness, and accidental extra Git processes.
