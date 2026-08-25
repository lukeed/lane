# Plan 035: Make resolvable anchors discoverable and canonical

> **Executor instructions**: Follow this plan step by step. Run every verification
> command and confirm the expected result before moving to the next step. If anything in
> the "STOP conditions" section occurs, stop and report — do not improvise. When done,
> update this plan's status row in `plans/README.md`, unless a reviewer told you they
> maintain the index.
>
> **Dependency check (run first)**: plan 034 must be DONE. Confirm that `lane ls --json`
> and `lane why --json` exist and their tests pass before starting this plan.
>
> **Drift check**:
> `git diff --stat ae4343c..HEAD -- crates/lane/src/args.rs crates/lane/src/capture.rs crates/lane/src/cli.rs crates/lane/src/help.rs crates/lane/src/syntax.rs crates/lane/tests/args.rs scripts/test.sh crates/lane/assets/skill.md readme.md www/src/data/commands.ts www/src/pages/usage.md`
>
> Plan 034 is expected to have changed `args.rs`, `cli.rs`, `help.rs`, `args.rs` tests,
> `scripts/test.sh`, the embedded skill, and the two website files only to add JSON modes
> for `ls` and `why`. Compare those exact changes with plan 034 before proceeding. Any
> other mismatch in an in-scope file is a STOP condition.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED
- **Depends on**: `plans/034-make-agent-reads-structured.md`
- **Category**: direction
- **Planned at**: commit `ae4343c`, 2026-08-24

## Why this matters

Lane asks people and agents to attach a note to a file and symbol, but it makes them guess
the anchor string. A misspelled anchor is accepted with a warning, and a bare name such as
`verify` silently resolves to the earliest matching declaration even when several kinds
share that name. The parser already knows every declaration, heading, and component block
it can resolve; expose that knowledge as `lane anchors`, and use the same candidate set to
turn unique shorthand into one canonical stored anchor.

This plan deliberately does not add prompting. It creates the deterministic discovery and
qualification layer that plan 036 can use for an interactive selector without duplicating
tree-sitter logic in the terminal code.

## Product contract

### `lane anchors <path>`

Human mode prints one addressable anchor per line, in source order, with a tab and its
1-indexed inclusive line range. `@file` is always first:

```text
@file	1-8
fn verify	1-4
fn refresh	6-8
```

`lane anchors <path> --json` prints a pretty JSON array in the same order. Every row has
exactly these fields and JSON types:

```json
{
  "anchor": "fn verify",
  "start": 1,
  "end": 4
}
```

Rules:

- A readable file always yields at least `@file`, including an empty file or a file whose
  extension has no grammar. An empty file's range is `1-1`.
- Declarations use a canonical, language-appropriate kind plus captured name. The exact
  kind table is fixed in Step 1.
- Markdown headings keep their exact ATX text, such as `## Rate limiting`.
- Present SFC/HTML blocks use `#script`, `#style`, and `#template`.
- Identical anchor strings are de-duplicated, retaining the earliest span. This preserves
  the resolver's existing "earliest declaration wins" behavior for overloads or repeated
  headings that lane cannot stably distinguish.
- A missing, directory, outside-repository, or unreadable path is an error. JSON mode does
  not turn errors into JSON.
- Human and JSON modes are pure reads and never create `.lane/`, pending state, or a note.

### Qualification when recording a note

For a supported file, the CLI and `Why:` trailer capture use the discovered candidates:

- An exact canonical anchor, heading, block, or `@file` is stored unchanged.
- A bare declaration name that maps to one canonical anchor is stored canonically:
  `verify` becomes `fn verify` in Rust.
- A bare name that maps to multiple canonical values is refused and the error lists those
  values in discovery order. For example, a Rust `fn run` and `const run` make `run`
  ambiguous. No pending note is written.
- An anchor with no candidate keeps today's behavior: warn and record it anyway, because
  the declaration may be about to be added.
- A named anchor in an unparsed file keeps today's behavior: warn, record it verbatim, and
  let audit report it as `unverifiable`.

The hook must never fail a commit. An ambiguous `Why:` trailer is warned and skipped, just
as other invalid trailers are today; the direct CLI returns an error.

## Current state

### Files and roles

- `crates/lane/src/syntax.rs` owns tree-sitter grammars, declaration queries, span
  resolution, and hashing.
- `crates/lane/src/args.rs` and `help.rs` own the accepted command surface and static help.
- `crates/lane/src/cli.rs` dispatches commands and currently validates direct notes.
- `crates/lane/src/capture.rs` validates `Why:` trailers independently before appending the
  same pending-note shape.
- `crates/lane/tests/args.rs` is the parser contract; `scripts/test.sh` is the real-process
  and output contract.
- `crates/lane/assets/skill.md`, `readme.md`, and the two website files teach anchor syntax.

### Resolver behavior to preserve

At `crates/lane/src/syntax.rs:296-327`, `Source::resolve_detail` handles `@file`, blocks,
headings, declarations, and the parsed/unparsed distinction. At `:381-425`,
`resolve_decl` splits a declaration anchor into an optional first word and final name,
walks every query match, and picks the earliest matching declaration.

At `crates/lane/src/cli.rs:617-652`, direct note creation warns on `NotFound` or
`Unparsed`, appends a `PendingNote`, and prints `noted -> <path>#<anchor>`. At
`crates/lane/src/capture.rs:139-168`, trailer capture repeats that resolution logic but
must degrade all failures to warnings.

Do not fork a second anchor parser in either caller. Discovery, canonicalization, and
resolution must share one internal declaration-candidate walk in `syntax.rs`.

### Canonical declaration kinds

Use this exact table. The left side is a grammar and tree-sitter declaration node; the
right side is the public anchor kind:

| Grammar | Declaration nodes | Kind |
|---|---|---|
| Rust | `function_item`, `struct_item`, `enum_item`, `trait_item`, `type_item`, `mod_item`, `const_item`, `static_item`, `macro_definition`, `impl_item` | `fn`, `struct`, `enum`, `trait`, `type`, `mod`, `const`, `static`, `macro`, `impl` |
| Go | `function_declaration`, `method_declaration`, `type_declaration`, `const_declaration`, `var_declaration` | `func`, `func`, `type`, `const`, `var` |
| Python | `function_definition`, `class_definition` | `def`, `class` |
| JavaScript/TypeScript | function/generator, class, method, lexical/variable, interface, type alias, enum nodes | `function`, `class`, `method`, `const` or `let`, `var`, `interface`, `type`, `enum` |
| C/C++ | function definition/declaration, struct, union, enum, typedef nodes | `fn`, `struct`, `union`, `enum`, `type` |
| Java | method, constructor, class, interface, enum, record nodes | `method`, `constructor`, `class`, `interface`, `enum`, `record` |
| Bash | `function_definition` | `function` |

For a JavaScript/TypeScript `lexical_declaration`, choose `const` or `let` from the
declaration's own first line. Do not use visibility/export words such as `pub` or `export`
as kinds. CSS, HTML, and Markdown have no declaration kinds beyond the block/heading rules.

## Commands you will need

Record baseline test and shell-assertion counts before editing.

| Purpose | Command | Expected on success |
|---|---|---|
| Dependency | `cargo test -p lane --test args structured_read_commands_take_json` | exit 0 |
| Parser | `cargo test -p lane --test args` | exit 0; baseline + 1 test |
| Syntax/capture | `cargo test -p lane --lib` | exit 0; baseline + 12 tests |
| Workspace | `cargo test --workspace` | exit 0; all pass |
| End to end | `./scripts/test.sh` | exit 0; `failed: 0`, baseline + 8 assertions |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0, no warnings |
| Format | `cargo fmt --all --check` | exit 0, no diff |
| Website | `cd www && bun run build` | exit 0; Astro check/build pass |

## Scope

**In scope** — the only implementation and documentation files to modify:

- `crates/lane/src/syntax.rs`
- `crates/lane/src/args.rs`
- `crates/lane/src/cli.rs`
- `crates/lane/src/help.rs`
- `crates/lane/src/capture.rs`
- `crates/lane/tests/args.rs`
- `scripts/test.sh`
- `crates/lane/assets/skill.md`
- `readme.md`
- `www/src/data/commands.ts`
- `www/src/pages/usage.md`

Administrative exception: update only plan 035's row in `plans/README.md` after every
done criterion passes.

**Out of scope**:

- Interactive input, terminal menus, or reading note text from stdin; plan 036 owns them.
- The `lane note add/replace/retire/restore/pin/unpin/confirm` family; plan 036 owns it.
- A new qualified-owner syntax such as `Auth::verify`, line-number anchors, or overload
  signatures. Identical canonical anchors retain the current earliest-match rule.
- Adding, removing, or feature-gating grammars; plan 012 owns grammar packaging.
- Changing hashing, normalization, freshness tiers, budgets, or note storage.
- Rejecting anchors that have no current match. They remain warn-and-record by design.
- JSON for any mutating command or changes to plan 034's `ls`/`why` schemas.
- New dependencies or a public stability promise for Rust library types.

## Git workflow

- Branch: `advisor/035-anchor-discovery`.
- Use Conventional Commit subjects matching history, for example
  `feat: add lane anchors command`.
- Prefer implementation/tests first, then documentation.
- Do not push or open a pull request unless instructed.

## Steps

### Step 1: Extract one declaration-candidate walk

In `crates/lane/src/syntax.rs`, add an internal declaration candidate containing a
canonical `value` and `Span`. Extend each `Grammar` with the per-node kind mapping from the
table above. Refactor the existing query walk so both `resolve_decl` and discovery consume
the same candidates.

Add a crate-private result type; this is shared by syntax, CLI, and later prompt code, not
a public Rust API:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub(crate) value: String,
    pub(crate) span: Span,
}
```

Implement `Source::anchors() -> Vec<Anchor>`:

1. add `@file` first;
2. collect the applicable heading, block, or declaration candidates;
3. sort non-file candidates by `(span.start, span.end, value)`; and
4. de-duplicate by `value`, retaining the earliest candidate.

Refactor `resolve_decl` to resolve an exact canonical value first and retain the old
bare-name/earliest fallback for notes already stored with shorthand. Do not change
`resolve_detail`'s `Found`, `NotFound`, and `Unparsed` contract.

Add exactly eight syntax tests covering: Rust order/canonical values; JavaScript lexical
kinds; Markdown headings; present SFC blocks; unparsed files; empty files; identical-value
de-duplication; and continued earliest resolution for an old bare anchor.

**Verify**: `cargo test -p lane syntax::tests` -> all pass, baseline + 8 tests.

### Step 2: Qualify user input without making missing anchors fatal

In `syntax.rs`, add a result enum and `Source::qualify(anchor: &str)`:

```rust
pub enum Qualification {
    Canonical(Anchor),
    Ambiguous(Vec<Anchor>),
    NotFound,
    Unparsed,
}
```

Exact discovered values return `Canonical`. A bare declaration name compares against the
name portion of canonical declaration values; one distinct value canonicalizes, several
distinct values are `Ambiguous` in discovery order. Hash-prefixed headings/blocks and
`@file` are exact-only. Unknown input and unparsed named input preserve the existing two
failure categories.

Update direct note creation in `cli.rs` and trailer capture in `capture.rs` to call this
method. Store and print the canonical value on success. Direct ambiguity is an error whose
body lists each choice as `  <anchor>` and writes no pending record. Trailer ambiguity is
`warning: rejected Why trailer: ...` and capture continues without failing the commit.
Keep the exact current warn-and-record behavior for `NotFound` and `Unparsed`.

Add two syntax tests for unique and ambiguous shorthand, plus two capture tests proving a
unique trailer is canonicalized and an ambiguous trailer is skipped without aborting later
valid trailers.

**Verify**: run `cargo test -p lane syntax::tests` and
`cargo test -p lane capture::tests` -> both pass, baseline + 12 tests total across
Steps 1-2.

### Step 3: Add the read-only `anchors` command

In `args.rs`:

- add `anchors` to the root command list immediately after `path` and before `note`;
- add `Parsed::Anchors { path: String, json: bool }`; and
- parse `lane anchors <PATH> [--json]`, with help winning and all extra values/flags
  refused using the existing parser helpers.

In `help.rs`, add a root row in that same position and a dedicated screen. Its usage is
`lane anchors [--json] <PATH>`. Describe `--json`, source ordering, and the fact that
`@file` is always present.

In `cli.rs`, resolve the path with `store::rel_to_repo`, require a regular readable file,
construct one `Source`, call `anchors()` once, and render either the exact human or JSON
contract above. Do not resolve each candidate again.

Add one parser test covering default human mode, `--json` before/after the path, help, a
missing path, an extra positional, and a misspelled option.

**Verify**: `cargo test -p lane --test args` -> exit 0, baseline + 1 test.

### Step 4: Lock the behavior down end to end

Add one new section to `scripts/test.sh` with exactly eight `is` assertions:

1. Human output for the setup repository is exactly `@file`, `fn verify`, `fn refresh`
   with their current line ranges and in that order.
2. JSON parses and every row has exactly `anchor`, `start`, and `end`.
3. JSON order and ranges match human output.
4. An unknown-extension file reports only `@file`.
5. `lane note -p src/auth.rs -a verify ...` prints/stores `fn verify`.
6. The promoted note's frontmatter contains `anchor: fn verify`.
7. A fixture containing `fn run` and `const run` makes direct `-a run` exit non-zero and
   list both canonical choices.
8. That ambiguous attempt does not change the pending-note count.

Keep all existing literal human tests for `note`, `why`, `check`, and plan 034's JSON
schemas unchanged.

**Verify**: `./scripts/test.sh` -> `failed: 0`, baseline + 8 assertions.

### Step 5: Document discovery without promising interaction yet

Update:

- `crates/lane/assets/skill.md`: tell agents to run `lane anchors <path> --json` when they
  do not already know a valid anchor; prefer returned canonical values.
- `readme.md`: add `lane anchors src/auth.rs` to the command overview and explain unique
  shorthand canonicalization in the Anchors paragraph.
- `www/src/data/commands.ts`: add the complete `anchors` command in parser order, including
  `--json` and a real transcript.
- `www/src/pages/usage.md`: add discovery beside the existing anchor table and agent loop.
- `help.rs`: already updated in Step 3; keep its wording consistent with these surfaces.

Do not document an interactive selector or the future note command family in this plan.

**Verify**:

```bash
rg -n 'lane anchors' crates/lane/src/help.rs crates/lane/assets/skill.md readme.md \
  www/src/data/commands.ts www/src/pages/usage.md
(cd www && bun run build)
```

Expected: every listed surface names the command; the website exits 0.

### Step 6: Run the complete gate and inspect scope

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
`failed: 0`; only in-scope files and the status-only plan-index update are changed.

## Test plan

- Syntax unit tests lock down every candidate class, canonical kind, ordering rule,
  de-duplication rule, and shorthand outcome without spawning a process.
- Capture tests prove hook-safe ambiguity handling and canonical storage.
- Parser tests cover the complete command grammar and near-miss errors.
- End-to-end tests prove path handling, exact output schemas, pure reads, canonical pending
  notes, and no write on ambiguity in a real Git repository.
- Existing tests continue to prove old stored bare anchors resolve earliest and unmatched
  anchors remain warn-and-record.

## Done criteria

- [ ] `lane anchors <path>` prints every addressable canonical anchor once, in the fixed
      order and line-range format.
- [ ] `lane anchors <path> --json` emits only `anchor`, `start`, and `end`, in the same
      order, and changes no repository state.
- [ ] `@file` is present for parsed, unparsed, and empty readable files.
- [ ] Unique shorthand is canonicalized for both direct notes and `Why:` capture.
- [ ] Ambiguous shorthand lists choices and writes nothing; capture warns without failing
      the commit.
- [ ] Unknown and unparsed anchors retain their current warn-and-record semantics.
- [ ] Existing stored bare anchors and identical canonical overloads retain earliest-match
      resolution.
- [ ] Parser tests pass at baseline + 1; syntax/capture tests pass at baseline + 12.
- [ ] `./scripts/test.sh` passes at baseline + 8 assertions and `failed: 0`.
- [ ] Format, Clippy, workspace tests, and website build all pass.
- [ ] No dependency, storage schema, grammar, freshness, or note-lifecycle change landed.
- [ ] `plans/README.md` marks plan 035 DONE only after every other criterion passes.

## STOP conditions

Stop and report instead of improvising if:

- plan 034 is not DONE or its `ls`/`why` JSON changes do not match its written schemas;
- a grammar's live query capture cannot be mapped to the fixed kind table without changing
  which declarations it resolves;
- discovery and resolution would need separate tree-sitter queries or parse trees;
- a proposed anchor value needs a line number, signature text, generated hash, or new
  persisted field to resolve;
- canonicalizing a unique shorthand changes the resolved span from today's earliest match;
- ambiguity handling would make the post-commit hook return failure;
- an existing unmatched/unparsed note test must be weakened;
- any human or JSON contract from plan 034 changes; or
- any verification fails twice after one focused correction.

## Maintenance notes

- `Source::anchors`, `Source::qualify`, and `Source::resolve_detail` must continue to share
  candidate generation. A new grammar query is incomplete until all three see it.
- Identical canonical values intentionally collapse to the earliest span. Stable overload
  or owner qualification is a separate design problem; do not smuggle line numbers into
  this feature.
- The JSON row is an agent-facing contract. Add fields only through an explicit follow-up.
- Plan 036 consumes `Source::anchors()` for terminal selection and `Source::qualify()` for
  explicit `-a`; review that plan if either API shape changes.
