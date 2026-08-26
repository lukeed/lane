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
    Enter,
    Exit,
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
            Help::Enter => ENTER,
            Help::Exit => EXIT,
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
            Help::Enter => "lane enter <NAME>",
            Help::Exit => "lane exit",
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
            Help::Merge => "lane merge [OPTIONS] [NAME]",
            Help::Push => "lane push [OPTIONS] [NAME]",
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
            Help::Enter => "lane enter",
            Help::Exit => "lane exit",
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
    init         Initialize lane in a repository
    new          Create a copy-on-write worktree
    enter        Enter a lane
    exit         Return to the main worktree
    ls           List lanes
    anchors      List note anchors in a file
    note         Manage notes
    install      Install hooks or the agent skill
    uninstall    Remove hooks or the agent skill
    why          Show notes for a path
    check        Find stale notes
    audit        Reconcile pending and stale notes
    merge        Land a lane locally
    push         Push a lane for review
    prune        Remove landed lanes
    rm           Discard a lane
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
    Create .lane/, add the AGENTS.md protocol, and check reflink support.
    Safe to re-run.

  Usage
    $ lane init

  Options
    -h, --help    Display this message
";

const NEW: &str = "
  Description
    Create a branch and worktree under .lane/trees/. Ignored files are cloned
    by reference when the filesystem supports reflinks.

  Usage
    $ lane new <name> [options]

  Options
    --base <rev>    Branch from <rev> instead of the default base
    --dirty         Carry uncommitted work into the lane
    -h, --help      Display this message

  Examples
    $ lane new fix-login
    $ lane new spike --dirty
    $ lane new hotfix --base v1.2.0
";

const LS: &str = "
  Description
    List each lane's state, worktree status, and pending note count.

  Usage
    $ lane ls [--json]

  Options
    --json        Emit machine-readable JSON
    -h, --help    Display this message
";

const ENTER: &str = "
  Description
    Change directory into a lane. `switch` is an alias.

  Usage
    $ lane enter <name>

  Options
    -h, --help    Display this message

  Examples
    $ lane enter fix-login
    $ lane switch fix-login
";

const EXIT: &str = "
  Description
    Change directory back to the main worktree.

  Usage
    $ lane exit

  Options
    -h, --help    Display this message
";

const ANCHORS: &str = "
  Description
    List note anchors in source order with their line ranges. @file is always
    available, including for empty and unparsed files.

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
    Record, update, or retire a memory note.

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
    Interactively confirm, replace, retire, pin, or unpin a live note.

  Usage
    $ lane note edit <id>

  Options
    -h, --help    Display this message
";

const NOTE_ADD: &str = "
  Description
    Record one finding about a file or anchor. Omit text to choose the anchor
    and enter the note interactively.

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
    Replace a live note on the next audit. Path and anchor are inherited unless
    overridden.

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
    Confirm that a drifted note is still true. An unambiguous id prefix works.

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
    Install Git hooks that capture `Why:` trailers, or the lane skill that
    teaches coding agents the workflow.

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
    Remove lane's Git hooks or agent skill. Unrelated file content is kept.

  Usage
    $ lane uninstall <hooks|skill>

  Options
    -h, --help    Display this message
";

const WHY: &str = "
  Description
    Show notes for a file or directory. Omit the path to show every note.

  Usage
    $ lane why [path] [options]

  Options
    -a, --anchor <anchor>  Only notes held for this anchor
        --json             Emit machine-readable JSON
    -h, --help             Display this message

  Examples
    $ lane why src/auth.rs
    $ lane why src/auth.rs -a 'fn verify'
    $ lane why src/
";

const CHECK: &str = "
  Description
    List notes whose anchored content has changed.

  Usage
    $ lane check [options]

  Options
    --json        Emit a JSON report, with each note's body and current span
    -h, --help    Display this message
";

const AUDIT: &str = "
  Description
    Promote pending notes, follow moved anchors, and move notes over budget to
    the attic. `lane merge` and `lane push` run this automatically.

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
    Rebase onto the lane's base, audit its notes, update the local base branch,
    and remove the worktree.

  Usage
    $ lane merge [name] [options]

  Options
    --base <ref>        Rebase onto <ref> instead of the recorded base
    --squash            Squash the lane's commits into one landing commit
    --keep              Leave the worktree in place after landing
    --max-notes <n>     Notes kept per anchor (default: 5)
    --max-chars <n>     Characters kept per anchor (default: 1200)
    -h, --help          Display this message

  Examples
    $ lane merge
    $ lane merge --squash
";

const PUSH: &str = "
  Description
    Rebase onto the lane's base, audit and commit its notes, then push the
    branch for review.

  Usage
    $ lane push [name] [options]

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
    Remove lanes whose branches have landed. Uncommitted work and commits made
    after landing are never discarded.

  Usage
    $ lane prune [options]

  Options
    --dry-run     List what would go, remove nothing
    -h, --help    Display this message
";

const RM: &str = "
  Description
    Remove a lane and its branch, worktree, pending notes, and local state.
    Refuse if anything would be lost unless --force is given.

  Usage
    $ lane rm <name> [options]

  Options
    --force       Discard it anyway
    -h, --help    Display this message
";

const SHELLENV: &str = "
  Description
    Print the shell function that makes `lane enter`, `lane exit`, `lane new`
    and `lane merge` leave the shell in the right directory.

  Usage
    $ eval \"$(lane shellenv)\"

  Options
    -h, --help    Display this message
";
