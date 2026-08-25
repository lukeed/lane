//! The help screens, and the usage lines the parse errors quote.
//!
//! Every screen is a static string rather than something rendered from the
//! parser's tables: nothing here can drift at run time, and the wording is
//! written for a reader instead of derived from field names. [`Help::usage`] is
//! the same line the screen opens with, so an error and the screen it points at
//! cannot disagree.

/// Which screen to print. Reached from a `-h`/`--help` anywhere in a command's
/// arguments, and from a bare `lane`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Help {
    Root,
    Init,
    New,
    Ls,
    Path,
    Anchors,
    Note,
    NoteAdd,
    NoteEdit,
    NoteReplace,
    NoteConfirm,
    NoteRetire,
    NoteRestore,
    NotePin,
    NoteUnpin,
    Install,
    Uninstall,
    Why,
    Check,
    Audit,
    Merge,
    Push,
    Prune,
    Rm,
    Shellenv,
}

impl Help {
    /// The whole screen, as printed to stdout.
    pub fn text(self) -> &'static str {
        match self {
            Help::Root => ROOT,
            Help::Init => INIT,
            Help::New => NEW,
            Help::Ls => LS,
            Help::Path => PATH,
            Help::Anchors => ANCHORS,
            Help::Note => NOTE,
            Help::NoteAdd => NOTE_ADD,
            Help::NoteEdit => NOTE_EDIT,
            Help::NoteReplace => NOTE_REPLACE,
            Help::NoteConfirm => NOTE_CONFIRM,
            Help::NoteRetire => NOTE_RETIRE,
            Help::NoteRestore => NOTE_RESTORE,
            Help::NotePin => NOTE_PIN,
            Help::NoteUnpin => NOTE_UNPIN,
            Help::Install => INSTALL,
            Help::Uninstall => UNINSTALL,
            Help::Why => WHY,
            Help::Check => CHECK,
            Help::Audit => AUDIT,
            Help::Merge => MERGE,
            Help::Push => PUSH,
            Help::Prune => PRUNE,
            Help::Rm => RM,
            Help::Shellenv => SHELLENV,
        }
    }

    /// The one-line grammar, quoted by a parse error.
    pub fn usage(self) -> &'static str {
        match self {
            Help::Root => "lane <COMMAND>",
            Help::Init => "lane init",
            Help::New => "lane new [OPTIONS] <NAME>",
            Help::Ls => "lane ls [--json]",
            Help::Path => "lane path <NAME>",
            Help::Anchors => "lane anchors [--json] <PATH>",
            Help::Note => "lane note <COMMAND>",
            Help::NoteAdd => "lane note add [OPTIONS] <PATH> [TEXT]",
            Help::NoteEdit => "lane note edit <ID>",
            Help::NoteReplace => "lane note replace [OPTIONS] <ID> [TEXT]",
            Help::NoteConfirm => "lane note confirm <ID>",
            Help::NoteRetire => "lane note retire <ID>",
            Help::NoteRestore => "lane note restore <ID>",
            Help::NotePin => "lane note pin <ID>",
            Help::NoteUnpin => "lane note unpin <ID>",
            Help::Install => "lane install <hooks|skill>",
            Help::Uninstall => "lane uninstall <hooks|skill>",
            Help::Why => "lane why [OPTIONS] [PATH]",
            Help::Check => "lane check [--json]",
            Help::Audit => "lane audit [OPTIONS]",
            Help::Merge => "lane merge [OPTIONS]",
            Help::Push => "lane push [OPTIONS]",
            Help::Prune => "lane prune [--dry-run]",
            Help::Rm => "lane rm [--force] <NAME>",
            Help::Shellenv => "lane shellenv",
        }
    }

    /// A line saying what a command's choices mean, where the names do not say
    /// it themselves. Shown under any error about them, so the reader who typed
    /// the wrong one and the reader who typed neither are told the same thing.
    pub fn tip(self) -> &'static str {
        match self {
            Help::Install | Help::Uninstall => {
                "\n\n  tip: `hooks` captures Why: trailers, `skill` teaches an agent the loop"
            }
            _ => "",
        }
    }

    /// What to tell the reader to run for more, without the flag.
    pub fn invocation(self) -> &'static str {
        match self {
            Help::Root => "lane",
            Help::Init => "lane init",
            Help::New => "lane new",
            Help::Ls => "lane ls",
            Help::Path => "lane path",
            Help::Anchors => "lane anchors",
            Help::Note => "lane note",
            Help::NoteAdd => "lane note add",
            Help::NoteEdit => "lane note edit",
            Help::NoteReplace => "lane note replace",
            Help::NoteConfirm => "lane note confirm",
            Help::NoteRetire => "lane note retire",
            Help::NoteRestore => "lane note restore",
            Help::NotePin => "lane note pin",
            Help::NoteUnpin => "lane note unpin",
            Help::Install => "lane install",
            Help::Uninstall => "lane uninstall",
            Help::Why => "lane why",
            Help::Check => "lane check",
            Help::Audit => "lane audit",
            Help::Merge => "lane merge",
            Help::Push => "lane push",
            Help::Prune => "lane prune",
            Help::Rm => "lane rm",
            Help::Shellenv => "lane shellenv",
        }
    }
}

// Each screen opens with a blank line and indents two spaces; `run` prints them
// with `println!`, which supplies the matching trailing one, so a screen is
// padded top and bottom.

const ROOT: &str = "
  Usage
    $ lane <command> [options]

  Commands
    init         Scaffold memory + merge rules, probe reflink
    new          Create a CoW lane
    ls           List lanes
    path         Print a lane's path
    anchors      List addressable anchors in a file
    note         Record a finding
    install      Install lane's agent integrations
    uninstall    Remove lane's agent integrations
    why          Show context for a path
    check        Staleness report
    audit        Promote, re-anchor, rank, evict
    merge        Rebase, audit, fast-forward, remove
    push         Rebase, audit, push for a pull request
    prune        Remove lanes whose branch has landed
    rm           Discard a lane without landing it
    shellenv     Print shell integration

  Options
    -h, --help       Display this message
    -V, --version    Display current version

  Examples
    $ lane new fix-login
    $ lane note add src/auth.rs -a 'fn verify' 'tokens rotate on refresh'
    $ lane why src/auth.rs
    $ lane merge
";

const INIT: &str = "
  Description
    Scaffold .lane/, the merge rule, and the AGENTS.md protocol, and report
    whether this filesystem can reflink. Safe to re-run.

  Usage
    $ lane init

  Options
    -h, --help    Display this message
";

const NEW: &str = "
  Description
    Create a worktree under .lane/trees/ and the branch it is on. Everything
    git ignores is cloned by reference, so a build cache costs no disk.

  Usage
    $ lane new <name> [options]

  Options
    --base <rev>    Branch from <rev> instead of the default base
    --dirty         Carry uncommitted work into the lane
    --cd            Print the path last, for the shell function
    -h, --help      Display this message

  Examples
    $ lane new fix-login
    $ lane new spike --dirty
    $ lane new hotfix --base v1.2.0
";

const LS: &str = "
  Description
    Every lane's name, whether it has landed, whether it is clean, and how many
    notes it has yet to land.

  Usage
    $ lane ls [--json]

  Options
    --json        Emit machine-readable JSON
    -h, --help    Display this message
";

const PATH: &str = "
  Description
    Print one lane's worktree path.

  Usage
    $ lane path <name>

  Options
    -h, --help    Display this message
";

const ANCHORS: &str = "
  Description
    List every addressable anchor in source order with its inclusive line range.
    @file is always present, including for empty and unparsed files.

  Usage
    $ lane anchors [--json] <PATH>

  Options
    --json        Emit machine-readable JSON
    -h, --help    Display this message

  Examples
    $ lane anchors src/auth.rs
";

const NOTE: &str = "
  Description
    Add or interactively edit a note, or apply one lifecycle action directly.

  Usage
    $ lane note <command>

  Commands
    add        Record a finding
    edit       Choose a lifecycle action interactively
    replace    Queue a replacement
    confirm    Confirm a drifted note is still true
    retire     Move a live note to the attic
    restore    Restore a retired note
    pin        Protect a live note from eviction
    unpin      Remove eviction protection

  Options
    -h, --help    Display this message
";

const NOTE_EDIT: &str = "
  Description
    Show a live note and interactively choose to confirm it, replace its text,
    retire it, or toggle its eviction protection. Text replacement creates a
    successor instead of rewriting the existing note.

  Usage
    $ lane note edit <id>

  Options
    -h, --help    Display this message
";

const NOTE_ADD: &str = "
  Description
    Record one finding about a file or anchor. Supplying text never prompts;
    omit it to select an anchor and enter the note interactively.

  Usage
    $ lane note add [options] <path> [text]

  Options
    -a, --anchor <anchor>  The symbol within the file (default: @file)
    -h, --help             Display this message

  Examples
    $ lane note add src/auth.rs -a 'fn verify' 'tokens rotate on refresh'
    $ lane note add README.md
";

const NOTE_REPLACE: &str = "
  Description
    Queue a successor for a live note, inheriting its path and anchor unless
    either is overridden. The predecessor retires when audit promotes it.

  Usage
    $ lane note replace [options] <id> [text]

  Options
    -p, --path <path>      Override the predecessor's file
    -a, --anchor <anchor>  Override the predecessor's anchor
    -h, --help             Display this message

  Examples
    $ lane note replace 01M0G2 'tokens rotate exactly once'
";

const NOTE_CONFIRM: &str = "
  Description
    Confirm that a drifted live note is still true. Any unambiguous id prefix
    works.

  Usage
    $ lane note confirm <id>

  Options
    -h, --help    Display this message
";

const NOTE_RETIRE: &str = "
  Description
    Retire a live note by moving it to the attic unchanged.

  Usage
    $ lane note retire <id>

  Options
    -h, --help    Display this message
";

const NOTE_RESTORE: &str = "
  Description
    Restore a retired note from the attic unchanged.

  Usage
    $ lane note restore <id>

  Options
    -h, --help    Display this message
";

const NOTE_PIN: &str = "
  Description
    Protect a live note from eviction.

  Usage
    $ lane note pin <id>

  Options
    -h, --help    Display this message
";

const NOTE_UNPIN: &str = "
  Description
    Remove eviction protection from a live note.

  Usage
    $ lane note unpin <id>

  Options
    -h, --help    Display this message
";

const INSTALL: &str = "
  Description
    Install an agent integration. `hooks` captures a commit's `Why:` trailer
    as a pending note; `skill` teaches an agent the daily loop.

  Usage
    $ lane install <hooks|skill>

  Options
    -h, --help    Display this message

  Examples
    $ lane install hooks
    $ lane install skill
";

const UNINSTALL: &str = "
  Description
    Remove an agent integration. Only lane's own delimited block is spliced
    out, so the rest of the file survives.

  Usage
    $ lane uninstall <hooks|skill>

  Options
    -h, --help    Display this message
";

const WHY: &str = "
  Description
    Print what earlier lanes learned about a path, each note with the id you
    need to re-vouch for it. With no path, report the whole store.

  Usage
    $ lane why [path] [options]

  Options
    -a, --anchor <anchor>  Only notes held for this anchor
        --json             Emit machine-readable JSON
    -h, --help             Display this message

  Examples
    $ lane why src/auth.rs
    $ lane why src/auth.rs -a 'fn verify'
";

const CHECK: &str = "
  Description
    Every note that is not fresh, each with the id you need next.

  Usage
    $ lane check [options]

  Options
    --json        Emit a JSON report, with each note's body and current span
    -h, --help    Display this message
";

const AUDIT: &str = "
  Description
    Resolve pending notes against the working tree, re-anchor what moved, and
    evict past the budget to the attic. `lane merge` runs this for you.

  Usage
    $ lane audit [options]

  Options
    --base <rev>        Rank notes for paths this lane touched since <rev>
    --max-notes <n>     Notes kept per anchor (default: 5)
    --max-chars <n>     Characters kept per anchor (default: 1200)
    --json              Emit a JSON report
    -h, --help          Display this message
";

const MERGE: &str = "
  Description
    Land a lane: rebase onto its base, audit memory, fast-forward, and remove
    the worktree. Landings are locked, so two at once serialize.

  Usage
    $ lane merge [options]

  Options
    --base <ref>        Rebase onto <ref> instead of the recorded base
    --squash            Squash the lane's commits into one landing commit
    --keep              Leave the worktree in place after landing
    --cd                Print the path last, for the shell function
    --max-notes <n>     Notes kept per anchor (default: 5)
    --max-chars <n>     Characters kept per anchor (default: 1200)
    -h, --help          Display this message

  Examples
    $ lane merge
    $ lane merge --squash
";

const PUSH: &str = "
  Description
    Rebase a lane onto its base, audit and commit memory, then push it for a
    pull request. The remote is the branch's upstream or origin.

  Usage
    $ lane push [options]

  Options
    --base <ref>        Rebase onto <ref> instead of the recorded base
    --max-notes <n>     Notes kept per anchor (default: 5)
    --max-chars <n>     Characters kept per anchor (default: 1200)
    -h, --help          Display this message

  Examples
    $ lane push
";

const PRUNE: &str = "
  Description
    Remove every lane whose branch has landed in trunk. A pushed lane sits here
    until its pull request merges; this is
    what collects it. Work committed after the merge is never discarded.

  Usage
    $ lane prune [options]

  Options
    --dry-run     List what would go, remove nothing
    -h, --help    Display this message
";

const RM: &str = "
  Description
    Discard a lane and everything it still holds: the worktree, the branch,
    the pending notes, the per-branch state. Anything that would be lost with
    it stops the removal and is named instead. A squash or rebase merge counts
    as landed, where `git branch -d` refuses it.

  Usage
    $ lane rm <name> [options]

  Options
    --force       Discard it anyway
    -h, --help    Display this message
";

const SHELLENV: &str = "
  Description
    Print the shell function that leaves you in the right directory after
    `lane new` and `lane merge`, and adds `lane cd <name>`.

  Usage
    $ eval \"$(lane shellenv)\"

  Options
    -h, --help    Display this message
";
