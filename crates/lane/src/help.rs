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
    Note,
    Install,
    Uninstall,
    Why,
    Holds,
    Check,
    Audit,
    Done,
    Push,
    Sweep,
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
            Help::Note => NOTE,
            Help::Install => INSTALL,
            Help::Uninstall => UNINSTALL,
            Help::Why => WHY,
            Help::Holds => HOLDS,
            Help::Check => CHECK,
            Help::Audit => AUDIT,
            Help::Done => DONE,
            Help::Push => PUSH,
            Help::Sweep => SWEEP,
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
            Help::Ls => "lane ls",
            Help::Path => "lane path <NAME>",
            Help::Note => "lane note [OPTIONS] --path <PATH> <TEXT>",
            Help::Install => "lane install <hooks|skill>",
            Help::Uninstall => "lane uninstall <hooks|skill>",
            Help::Why => "lane why [OPTIONS] [PATH]",
            Help::Holds => "lane holds <ID>",
            Help::Check => "lane check [--json]",
            Help::Audit => "lane audit [OPTIONS]",
            Help::Done => "lane done [OPTIONS]",
            Help::Push => "lane push [OPTIONS]",
            Help::Sweep => "lane sweep [--dry-run]",
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
            Help::Note => "lane note",
            Help::Install => "lane install",
            Help::Uninstall => "lane uninstall",
            Help::Why => "lane why",
            Help::Holds => "lane holds",
            Help::Check => "lane check",
            Help::Audit => "lane audit",
            Help::Done => "lane done",
            Help::Push => "lane push",
            Help::Sweep => "lane sweep",
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
    note         Record a finding
    install      Install lane's agent integrations
    uninstall    Remove lane's agent integrations
    why          Show context for a path
    holds        Re-vouch for a drifted note
    check        Staleness report
    audit        Promote, re-anchor, rank, evict
    done         Rebase, audit, fast-forward, remove
    push         Rebase, audit, push for a pull request
    sweep        Remove lanes whose branch has landed
    rm           Discard a lane without landing it
    shellenv     Print shell integration

  Options
    -h, --help       Display this message
    -V, --version    Display current version

  Examples
    $ lane new fix-login
    $ lane note -p src/auth.rs -a 'fn verify' 'tokens rotate on refresh'
    $ lane why src/auth.rs
    $ lane done
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
    $ lane ls

  Options
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

const NOTE: &str = "
  Description
    Record one finding about one anchor. An anchor is `fn verify`, `#script`,
    `## Heading`, or `@file` for the whole file.

  Usage
    $ lane note -p <path> [options] <text>

  Options
    -p, --path <path>      The file the finding is about (required)
    -a, --anchor <anchor>  The symbol within it (default: @file)
        --supersedes <id>  Retire this note, which the new one replaces
    -h, --help             Display this message

  Examples
    $ lane note -p src/auth.rs -a 'fn verify' 'tokens rotate on refresh'
    $ lane note -p README.md -a '## Install' 'the tarball is flat, not nested'
    $ lane note -p src/auth.rs -a 'fn verify' --supersedes 01M0G2 'and only once'
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
    -h, --help             Display this message

  Examples
    $ lane why src/auth.rs
    $ lane why src/auth.rs -a 'fn verify'
";

const HOLDS: &str = "
  Description
    Re-vouch for a drifted note: its span changed and you say it is still
    true. Any unambiguous prefix of the id works.

  Usage
    $ lane holds <id>

  Options
    -h, --help    Display this message
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
    evict past the budget to the attic. `lane done` runs this for you.

  Usage
    $ lane audit [options]

  Options
    --base <rev>        Rank notes for paths this lane touched since <rev>
    --max-notes <n>     Notes kept per anchor (default: 5)
    --max-chars <n>     Characters kept per anchor (default: 1200)
    --json              Emit a JSON report
    -h, --help          Display this message
";

const DONE: &str = "
  Description
    Land a lane: rebase onto its base, audit memory, fast-forward, and remove
    the worktree. Landings are locked, so two at once serialize.

  Usage
    $ lane done [options]

  Options
    --base <ref>        Rebase onto <ref> instead of the recorded base
    --squash            Squash the lane's commits into one landing commit
    --keep              Leave the worktree in place after landing
    --cd                Print the path last, for the shell function
    --max-notes <n>     Notes kept per anchor (default: 5)
    --max-chars <n>     Characters kept per anchor (default: 1200)
    -h, --help          Display this message

  Examples
    $ lane done
    $ lane done --squash
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

const SWEEP: &str = "
  Description
    Remove every lane whose branch has landed in trunk. A pushed lane sits here
    until its pull request merges; this is
    what collects it. Work committed after the merge is never discarded.

  Usage
    $ lane sweep [options]

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
    `lane new` and `lane done`, and adds `lane cd <name>`.

  Usage
    $ eval \"$(lane shellenv)\"

  Options
    -h, --help    Display this message
";
