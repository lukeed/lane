//! Argument parsing. A leading word selects the command; everything after it is
//! read by `pico-args` and what it could not place is checked here.
//!
//! [`parse`] is pure and takes the argument list explicitly, so a test drives it
//! without a process. `-h`/`--help` anywhere in a command's arguments wins over
//! the rest of them, and a bare `lane` prints the root screen. Words after `--`
//! are positional whatever they look like, which is what lets a note's text
//! start with a dash.

use crate::help::Help;
use anyhow::Result;
use std::ffi::OsString;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const MAX_NOTES: usize = 5;
const MAX_CHARS: usize = 1200;

/// Every command that appears in help, for the typo suggestion.
const COMMANDS: &[&str] = &[
    "init",
    "new",
    "ls",
    "path",
    "anchors",
    "note",
    "install",
    "uninstall",
    "why",
    "holds",
    "check",
    "audit",
    "merge",
    "push",
    "prune",
    "rm",
    "shellenv",
];

/// How much memory one `(path, anchor)` may keep. Shared by `audit` and `merge`,
/// which is why it is a type rather than two pairs of fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Budget {
    pub max_notes: usize,
    pub max_chars: usize,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_notes: MAX_NOTES,
            max_chars: MAX_CHARS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installable {
    Hooks,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewArgs {
    pub name: String,
    pub base: Option<String>,
    pub dirty: bool,
    pub cd: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteArgs {
    pub text: String,
    pub path: String,
    pub anchor: String,
    pub supersedes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhyArgs {
    pub path: Option<String>,
    pub anchor: Option<String>,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditArgs {
    pub base: String,
    pub budget: Budget,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeArgs {
    pub base: Option<String>,
    pub keep: bool,
    pub cd: bool,
    pub squash: bool,
    pub budget: Budget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushArgs {
    pub base: Option<String>,
    pub budget: Budget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmArgs {
    pub name: String,
    pub force: bool,
}

/// What the argument list asked for. `Help` and `Version` are answers in their
/// own right rather than a flag on a command, because neither reaches the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Init,
    New(NewArgs),
    Ls { json: bool },
    Path { name: String },
    Anchors { path: String, json: bool },
    Note(NoteArgs),
    Install(Installable),
    Uninstall(Installable),
    Capture { rev: String },
    Why(WhyArgs),
    Holds { id: String },
    Check { json: bool },
    Audit(AuditArgs),
    Merge(MergeArgs),
    Push(PushArgs),
    Prune { dry_run: bool },
    Rm(RmArgs),
    Shellenv,
    Help(Help),
    Version,
}

/// Parse the arguments after the program name.
///
/// # Example
/// ```
/// use lane::args::{Installable, Parsed, parse};
/// let parsed = parse(vec!["install".into(), "skill".into()]).unwrap();
/// assert_eq!(parsed, Parsed::Install(Installable::Skill));
/// ```
pub fn parse(raw: Vec<OsString>) -> Result<Parsed> {
    let head = raw.first().and_then(|a| a.to_str()).map(str::to_owned);
    match head.as_deref() {
        Some("init") => bare(rest(raw), Help::Init, Parsed::Init),
        Some("new") => parse_new(rest(raw)),
        Some("ls") => parse_ls(rest(raw)),
        Some("path") => parse_one(rest(raw), Help::Path, "<NAME>", |name| Parsed::Path {
            name,
        }),
        Some("anchors") => parse_anchors(rest(raw)),
        Some("note") => parse_note(rest(raw)),
        Some("install") => parse_installable(rest(raw), Help::Install, Parsed::Install),
        Some("uninstall") => parse_installable(rest(raw), Help::Uninstall, Parsed::Uninstall),
        Some("capture") => parse_capture(rest(raw)),
        Some("why") => parse_why(rest(raw)),
        Some("holds") => parse_one(rest(raw), Help::Holds, "<ID>", |id| Parsed::Holds { id }),
        Some("check") => parse_check(rest(raw)),
        Some("audit") => parse_audit(rest(raw)),
        Some("merge") => parse_merge(rest(raw)),
        Some("push") => parse_push(rest(raw)),
        Some("prune") => parse_prune(rest(raw)),
        Some("rm") => parse_rm(rest(raw)),
        Some("shellenv") => bare(rest(raw), Help::Shellenv, Parsed::Shellenv),
        Some("-h" | "--help") => Ok(Parsed::Help(Help::Root)),
        Some("-V" | "--version") => Ok(Parsed::Version),
        None => Ok(Parsed::Help(Help::Root)),
        Some(other) if other.starts_with('-') => Err(unexpected(other, Help::Root)),
        Some(other) => Err(unrecognized(other)),
    }
}

/// Drop the leading command word.
fn rest(raw: Vec<OsString>) -> Vec<OsString> {
    raw.into_iter().skip(1).collect()
}

/// Split at a bare `--`. Nothing after it is read as a flag, by pico-args or by
/// the leftover check below.
fn terminated(raw: Vec<OsString>) -> (Vec<OsString>, Vec<OsString>) {
    match raw.iter().position(|a| a.as_os_str() == "--") {
        Some(at) => {
            let mut flags = raw;
            let tail = flags.split_off(at);
            (flags, tail.into_iter().skip(1).collect())
        }
        None => (raw, Vec::new()),
    }
}

/// What pico-args could not place, refusing anything that still looks like a
/// flag. Words held back by `--` join the positionals without that check.
fn positionals(
    pargs: pico_args::Arguments,
    after: Vec<OsString>,
    help: Help,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for arg in pargs.finish() {
        let text = arg.to_string_lossy().into_owned();
        if text.starts_with('-') {
            return Err(unexpected(&text, help));
        }
        out.push(text);
    }
    out.extend(after.into_iter().map(|a| a.to_string_lossy().into_owned()));
    Ok(out)
}

fn one(got: Vec<String>, name: &str, help: Help) -> Result<String> {
    let mut got = got.into_iter();
    let Some(first) = got.next() else {
        return Err(missing(&[name], help));
    };
    match got.next() {
        Some(extra) => Err(unexpected(&extra, help)),
        None => Ok(first),
    }
}

fn at_most_one(got: Vec<String>, help: Help) -> Result<Option<String>> {
    let mut got = got.into_iter();
    let first = got.next();
    match got.next() {
        Some(extra) => Err(unexpected(&extra, help)),
        None => Ok(first),
    }
}

fn none(got: Vec<String>, help: Help) -> Result<()> {
    match got.into_iter().next() {
        Some(extra) => Err(unexpected(&extra, help)),
        None => Ok(()),
    }
}

/// A command with no arguments of its own.
fn bare(raw: Vec<OsString>, help: Help, parsed: Parsed) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(help));
    }
    none(positionals(pargs, after, help)?, help)?;
    Ok(parsed)
}

/// A command whose whole surface is one required word.
fn parse_one(
    raw: Vec<OsString>,
    help: Help,
    name: &str,
    build: fn(String) -> Parsed,
) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(help));
    }
    Ok(build(one(positionals(pargs, after, help)?, name, help)?))
}

fn parse_new(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::New));
    }
    let base = pargs.opt_value_from_str("--base")?;
    let dirty = pargs.contains("--dirty");
    let cd = pargs.contains("--cd");
    let name = one(positionals(pargs, after, Help::New)?, "<NAME>", Help::New)?;
    Ok(Parsed::New(NewArgs {
        name,
        base,
        dirty,
        cd,
    }))
}

fn parse_ls(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Ls));
    }
    let json = pargs.contains("--json");
    none(positionals(pargs, after, Help::Ls)?, Help::Ls)?;
    Ok(Parsed::Ls { json })
}

fn parse_anchors(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Anchors));
    }
    let json = pargs.contains("--json");
    let path = one(
        positionals(pargs, after, Help::Anchors)?,
        "<PATH>",
        Help::Anchors,
    )?;
    Ok(Parsed::Anchors { path, json })
}

fn parse_note(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Note));
    }
    let path: Option<String> = pargs.opt_value_from_str(["-p", "--path"])?;
    let anchor: Option<String> = pargs.opt_value_from_str(["-a", "--anchor"])?;
    let supersedes = pargs.opt_value_from_str("--supersedes")?;
    let text = at_most_one(positionals(pargs, after, Help::Note)?, Help::Note)?;

    // Both are required, and a reader who forgot both should be told so once.
    match (text, path) {
        (Some(text), Some(path)) => Ok(Parsed::Note(NoteArgs {
            text,
            path,
            anchor: anchor.unwrap_or_else(|| "@file".to_string()),
            supersedes,
        })),
        (text, path) => {
            let mut absent = Vec::new();
            if path.is_none() {
                absent.push("--path <PATH>");
            }
            if text.is_none() {
                absent.push("<TEXT>");
            }
            Err(missing(&absent, Help::Note))
        }
    }
}

fn parse_installable(
    raw: Vec<OsString>,
    help: Help,
    build: fn(Installable) -> Parsed,
) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(help));
    }
    let what = one(positionals(pargs, after, help)?, "<hooks|skill>", help)?;
    match what.as_str() {
        "hooks" => Ok(build(Installable::Hooks)),
        "skill" => Ok(build(Installable::Skill)),
        other => Err(anyhow::anyhow!(
            "unrecognized integration '{other}'{}\n\n\
             Usage: {}\n\nFor more information, try '{} --help'.",
            help.tip(),
            help.usage(),
            help.invocation()
        )),
    }
}

/// The commit-message hook's own entry point. Hidden: it is called by a hook,
/// never typed, so it has no help screen to point a reader at.
fn parse_capture(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let pargs = pico_args::Arguments::from_vec(flags);
    let rev = one(positionals(pargs, after, Help::Root)?, "<REV>", Help::Root)?;
    Ok(Parsed::Capture { rev })
}

fn parse_why(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Why));
    }
    let anchor = pargs.opt_value_from_str(["-a", "--anchor"])?;
    let json = pargs.contains("--json");
    let path = at_most_one(positionals(pargs, after, Help::Why)?, Help::Why)?;
    Ok(Parsed::Why(WhyArgs { path, anchor, json }))
}

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

fn parse_audit(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Audit));
    }
    let base: Option<String> = pargs.opt_value_from_str("--base")?;
    let budget = budget(&mut pargs)?;
    let json = pargs.contains("--json");
    none(positionals(pargs, after, Help::Audit)?, Help::Audit)?;
    Ok(Parsed::Audit(AuditArgs {
        base: base.unwrap_or_default(),
        budget,
        json,
    }))
}

fn parse_merge(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Merge));
    }
    let base = pargs.opt_value_from_str("--base")?;
    let keep = pargs.contains("--keep");
    let cd = pargs.contains("--cd");
    let squash = pargs.contains("--squash");
    let budget = budget(&mut pargs)?;
    none(positionals(pargs, after, Help::Merge)?, Help::Merge)?;
    Ok(Parsed::Merge(MergeArgs {
        base,
        keep,
        cd,
        squash,
        budget,
    }))
}

fn parse_push(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Push));
    }
    let base = pargs.opt_value_from_str("--base")?;
    let budget = budget(&mut pargs)?;
    none(positionals(pargs, after, Help::Push)?, Help::Push)?;
    Ok(Parsed::Push(PushArgs { base, budget }))
}

fn parse_prune(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Prune));
    }
    let dry_run = pargs.contains("--dry-run");
    none(positionals(pargs, after, Help::Prune)?, Help::Prune)?;
    Ok(Parsed::Prune { dry_run })
}

fn parse_rm(raw: Vec<OsString>) -> Result<Parsed> {
    let (flags, after) = terminated(raw);
    let mut pargs = pico_args::Arguments::from_vec(flags);
    if pargs.contains(["-h", "--help"]) {
        return Ok(Parsed::Help(Help::Rm));
    }
    let force = pargs.contains("--force");
    let name = one(positionals(pargs, after, Help::Rm)?, "<NAME>", Help::Rm)?;
    Ok(Parsed::Rm(RmArgs { name, force }))
}

/// The budget flags `audit` and `merge` share.
fn budget(pargs: &mut pico_args::Arguments) -> Result<Budget> {
    let default = Budget::default();
    Ok(Budget {
        max_notes: pargs
            .opt_value_from_str("--max-notes")?
            .unwrap_or(default.max_notes),
        max_chars: pargs
            .opt_value_from_str("--max-chars")?
            .unwrap_or(default.max_chars),
    })
}

fn unexpected(token: &str, help: Help) -> anyhow::Error {
    // A word that only looks like a flag — a note's text, a branch named `-x` —
    // has somewhere to go, and the reader is told where rather than left to guess.
    let tip = match token.starts_with('-') {
        true => format!("\n\n  tip: to pass '{token}' as a value, use '-- {token}'"),
        false => String::new(),
    };
    anyhow::anyhow!(
        "unexpected argument '{token}' found{tip}\n\nUsage: {}\n\nFor more information, try '{} --help'.",
        help.usage(),
        help.invocation()
    )
}

fn missing(absent: &[&str], help: Help) -> anyhow::Error {
    let list = absent
        .iter()
        .map(|name| format!("  {name}"))
        .collect::<Vec<_>>()
        .join("\n");
    anyhow::anyhow!(
        "the following required arguments were not provided:\n{list}{}\n\nUsage: {}\n\nFor more information, try '{} --help'.",
        help.tip(),
        help.usage(),
        help.invocation()
    )
}

fn unrecognized(typed: &str) -> anyhow::Error {
    let tip = match nearest(typed) {
        Some(name) => format!("\n\n  tip: a similar subcommand exists: '{name}'"),
        None => String::new(),
    };
    anyhow::anyhow!(
        "unrecognized subcommand '{typed}'{tip}\n\nUsage: {}\n\nFor more information, try '{} --help'.",
        Help::Root.usage(),
        Help::Root.invocation()
    )
}

/// The closest command name, when one is close enough to be worth offering.
/// Two edits is the bound: past that the guess is noise rather than a typo.
fn nearest(typed: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .map(|name| (distance(typed, name), *name))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, name)| name)
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ac) in a.chars().enumerate() {
        cur[0] = i + 1;
        for (j, bc) in b.iter().enumerate() {
            let cost = usize::from(ac != *bc);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}
