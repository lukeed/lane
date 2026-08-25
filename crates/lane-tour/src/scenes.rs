//! The tour's content: what each scene does, and what it is trying to teach.
//!
//! Everything a reader learns lives in this file. `main.rs` only sequences it.

/// One instruction inside a scene.
pub enum Step {
    /// Prose, printed to the reader. Explains before the command, never after.
    Say(&'static str),
    /// A shell command, echoed as `$ ...` and then run in the sandbox root.
    Do(&'static str),
    /// The same, run inside a subdirectory of the sandbox.
    In(&'static str, &'static str),
    /// A pause. The reader looks at the working directory before anything else moves.
    Look(&'static str),
}

pub struct Scene {
    /// Menu key.
    pub key: &'static str,
    /// Menu label.
    pub title: &'static str,
    /// One line on why this is worth seeing, shown under the title when played.
    pub why: &'static str,
    pub steps: &'static [Step],
    /// Recorded on the timeline when the scene finishes.
    pub records: &'static str,
}

use Step::{Do, In, Look, Say};

pub const OPENING: &str = "\
This is a sandbox. Nothing here touches your own repositories, and you can delete
the whole directory when you are done.

It is a small project with one source file, already set up to use lane. Open it in
another terminal or in your editor and keep it visible — the point of this tour is
watching the directory change while you drive.

Each option below runs real commands. They are printed before they run, so you can
type them yourself later.";

pub const CLOSING: &str = "\
What you saw, in one paragraph.

A lane is a worktree that costs almost nothing to open, so you can have several at
once and throw them away. What survives is the memory: short notes attached to a
file and a symbol, written once, carried onto trunk when a lane lands. When the code
under a note changes, the note is flagged and stays flagged until someone resolves
it — because a confidently wrong note is worse than no note.

The commands worth remembering are `lane new`, `lane why` before you edit, and
`lane merge`. Everything else is bookkeeping lane does for you.";

pub const SCENES: &[Scene] = &[
    Scene {
        key: "1",
        title: "Look around first",
        why: "Before anything moves, see what a lane-enabled project actually contains.",
        steps: &[
            Say("A project that uses lane has three additions, and none of them are large."),
            Do("ls -a"),
            Say(
                "`.lane/` is the memory. It is plain markdown at predictable paths, so an\n\
                 agent finds it without any tool integration. It is empty right now.",
            ),
            Do("find .lane -type f | head"),
            Say(
                "`AGENTS.md` is the part an agent always has in context. Three rules and a\n\
                 pointer, deliberately short.",
            ),
            Do("cat AGENTS.md"),
            Look("Open the sandbox in your editor. You are looking at an ordinary git repository."),
        ],
        records: "looked at an empty store",
    },
    Scene {
        key: "2",
        title: "Someone commits on main",
        why: "Trunk moves on its own. Everything later has to survive that.",
        steps: &[
            Say("A colleague pushes a change to `main`. Nothing to do with you."),
            Do(
                "printf 'pub fn parse(token: &str) -> Parsed {\\n    Parsed::from(token)\\n}\\n' >> src/auth.rs",
            ),
            Do("git add -A && git commit -q -m 'add parse' && git log --oneline -1"),
            Say(
                "Remember this. When you land work later, lane rebases onto whatever trunk\n\
                 has become — it never asks you to merge.",
            ),
        ],
        records: "main moved ahead by one commit",
    },
    Scene {
        key: "3",
        title: "Record what you know",
        why: "A note says what must stay true, not what you changed.",
        steps: &[
            Say("You have learned something about `verify` that the code does not say."),
            Do(
                "lane note add src/auth.rs -a 'fn verify' 'must stay constant-time; an early return leaks token length'",
            ),
            Say(
                "`-a` is the anchor: what the note is *about*. `fn verify` binds it to that\n\
                 declaration, not to a line number, so the note survives the file being edited\n\
                 above it.",
            ),
            Say(
                "The note is queued, not stored. It gets resolved and fingerprinted at the next\n\
                 audit, against the tree as it is then — never against a commit that is about to\n\
                 be rewritten.",
            ),
            Do("lane audit"),
            Do("find .lane/memory/ -type f"),
            Look("Open that file. It is markdown with a small header — nothing proprietary."),
            Say(
                "Now commit it. Memory is versioned like source, which is what makes it\n\
                 travel: clone the repository and the notes arrive with it. It also has to\n\
                 be committed before you open a lane, or the lane starts without it.",
            ),
            Do(
                "git add -A .lane && git commit -q -m 'memory: record what we know' && git log --oneline -1",
            ),
        ],
        records: "recorded a note on src/auth.rs#fn verify",
    },
    Scene {
        key: "4",
        title: "Read before you edit",
        why: "This is the whole point. One command, before touching a file.",
        steps: &[
            Do("lane why src/auth.rs"),
            Say(
                "That is the habit worth building: `lane why <file>` before you change it.\n\
                 Reading costs nothing and changes nothing — the command does not write.",
            ),
        ],
        records: "read the context for src/auth.rs",
    },
    Scene {
        key: "5",
        title: "Open a lane and do some work",
        why: "A worktree with a warm build cache, for the price of a directory entry.",
        steps: &[
            Say(
                "A lane is a git worktree plus everything git ignores, cloned by reference\n\
                 where the filesystem supports it. Your caches come along without being copied.",
            ),
            Do("lane new fix-empty-token"),
            Say(
                "It lives inside the repository at `.lane/trees/<name>`, excluded from git so it\n\
                 never shows up in `git status`.",
            ),
            Do("git status --porcelain && echo '(empty: the lane is invisible to git)'"),
            In(
                ".lane/trees/fix-empty-token",
                "printf 'pub fn verify(token: &str) -> bool {\\n    !token.is_empty() && parse(token).is_valid()\\n}\\n' > src/auth.rs",
            ),
            In(
                ".lane/trees/fix-empty-token",
                "git add -A && git commit -q -m 'reject empty tokens' && git log --oneline -1",
            ),
            Look(
                "Look at `.lane/trees/fix-empty-token/` — a complete checkout, on its own branch.",
            ),
        ],
        records: "opened lane fix-empty-token and committed work",
    },
    Scene {
        key: "6",
        title: "Watch the note notice",
        why: "The note was about the code you just changed.",
        steps: &[
            Say(
                "You edited the body of `verify`. The note about it is still there, but the\n\
                 thing it describes has moved underneath it.",
            ),
            In(".lane/trees/fix-empty-token", "lane check"),
            Say(
                "`content-changed` means the implementation changed while the signature held. Lane\n\
                 cannot know whether the note is still true — that is a judgment — so it flags\n\
                 it and leaves it flagged.",
            ),
            In(".lane/trees/fix-empty-token", "lane why src/auth.rs"),
            Say(
                "The `~` is the flag. Notes are not deleted for drifting; a human or a model\n\
                 decides, and until then the uncertainty stays visible.",
            ),
        ],
        records: "a note drifted and was flagged",
    },
    Scene {
        key: "7",
        title: "Record a decision from the commit itself",
        why: "You are already writing a commit message. Use it.",
        steps: &[
            Do("lane install hooks"),
            Say(
                "Now a `Why:` trailer in any commit message becomes a note. The form is\n\
                 `Why: <path>#<anchor> | <what must stay true>`.",
            ),
            In(
                ".lane/trees/fix-empty-token",
                "printf 'pub fn verify(token: &str) -> bool {\\n    !token.is_empty() && parse(token).is_valid()\\n}\\n\\npub fn refresh(token: &str) -> String {\\n    rotate(token)\\n}\\n' > src/auth.rs",
            ),
            In(
                ".lane/trees/fix-empty-token",
                "git add -A && git commit -q -m 'add refresh\n\nWhy: src/auth.rs#fn refresh | callers depend on the rotated value, not the original'",
            ),
            Say(
                "Captured as you committed. No second command, no context switch — which is the\n\
                 only way this habit survives contact with a real day.",
            ),
        ],
        records: "captured a decision from a Why trailer",
    },
    Scene {
        key: "8",
        title: "Three lanes at once",
        why: "The headline: parallel work that cannot collide.",
        steps: &[
            Say(
                "Open three more lanes. Each is a full checkout; on a copy-on-write filesystem\n\
                 they cost almost no disk.",
            ),
            Do("lane new agent-a && lane new agent-b && lane new agent-c"),
            Do("lane ls"),
            Say(
                "Now have all three write a note about the same file, at the same anchor — the\n\
                 case that would deadlock a shared file.",
            ),
            In(
                ".lane/trees/agent-a",
                "lane note add src/auth.rs -a 'fn verify' 'agent-a: rejects empty tokens first'",
            ),
            In(
                ".lane/trees/agent-b",
                "lane note add src/auth.rs -a 'fn verify' 'agent-b: parse is total, never panics'",
            ),
            In(
                ".lane/trees/agent-c",
                "lane note add src/auth.rs -a 'fn verify' 'agent-c: called on every request, keep it allocation-free'",
            ),
            Say(
                "Nothing conflicts, because a note file is written once and never modified.\n\
                 Two writers can never touch the same bytes, so there is nothing to resolve.",
            ),
            Look(
                "Look at `.lane/trees/` — four working directories, four branches, one repository.",
            ),
        ],
        records: "ran three lanes in parallel, all annotating the same anchor",
    },
    Scene {
        key: "9",
        title: "Land them, in any order",
        why: "Each landing rebases onto whatever trunk has become.",
        steps: &[
            In(".lane/trees/agent-b", "lane merge"),
            Say(
                "Rebased, memory folded in, trunk fast-forwarded, lane deleted. Trunk's history\n\
                 stays linear and ends with a marker naming what landed.",
            ),
            In(".lane/trees/agent-c", "lane merge"),
            In(".lane/trees/agent-a", "lane merge"),
            Do("git log --oneline -6"),
            Say(
                "Landed in a different order than they were opened, with no conflicts and no\n\
                 coordination between them.",
            ),
            Do("lane why src/auth.rs"),
            Say(
                "All three are on trunk, each attributed to the lane that wrote it, and none of\n\
                 them conflicted. Nothing is flagged yet — trunk's `verify` is still the code\n\
                 those notes were written against. That changes in a moment.",
            ),
        ],
        records: "landed three lanes out of order",
    },
    Scene {
        key: "a",
        title: "Two landings at the same moment",
        why: "What happens when two agents finish together.",
        steps: &[
            Say(
                "Landing touches trunk, so it has to be exclusive. Lane holds a lock for the\n\
                 duration — one held by the process itself, so it cannot be left behind by a\n\
                 crash.",
            ),
            Say(
                "There is no lane command for \"pretend another agent is finishing right\n\
                 now\", so the line below stands in for one: it takes the same lock and holds\n\
                 it for three seconds while a landing is attempted underneath.",
            ),
            Do(
                "python3 -c \"import fcntl,time;f=open('.git/lane/main.lock','w');fcntl.flock(f,fcntl.LOCK_EX);time.sleep(3)\" & sleep 1; ( cd .lane/trees/fix-empty-token && lane merge ); wait",
            ),
            Say(
                "It refuses at once and says why, rather than blocking or corrupting the\n\
                 shared state. A second later the same command simply works.",
            ),
        ],
        records: "saw a concurrent landing refused",
    },
    Scene {
        key: "b",
        title: "Land the last lane and read the result",
        why: "Where the memory ends up, and what it looks like to the next person.",
        steps: &[
            In(".lane/trees/fix-empty-token", "lane merge"),
            Do("lane why src/auth.rs"),
            Say(
                "Landing the fix changed `verify`, so every note about it is flagged at once —\n\
                 five notes from five branches, four of them now asking to be re-read. None\n\
                 was deleted and none was quietly accepted; the uncertainty is just visible.",
            ),
            Do("git log --oneline | head -8"),
            Look(
                "Open `.lane/` in your editor. Everything lane knows is in there, in plain\n\
                  markdown, at paths that mirror your source tree.",
            ),
        ],
        records: "landed the last lane; four notes on trunk",
    },
    Scene {
        key: "c",
        title: "What lane refuses to do",
        why: "The guardrails are worth knowing before you rely on them.",
        steps: &[
            Say(
                "Lane will not silently overwrite something you edited. The skill file it\n\
                 installs is an example — change it and ask for it again:",
            ),
            Do("lane install skill"),
            Do(
                "printf '\\nedited by hand\\n' >> .agents/skills/lane/SKILL.md && lane install skill; echo \"exit: $?\"",
            ),
            Say(
                "It refuses and tells you how to proceed. The same rule covers commit hooks and\n\
                 the `AGENTS.md` protocol: lane owns a marked region and nothing outside it.",
            ),
        ],
        records: "saw lane refuse to clobber an edited file",
    },
];
