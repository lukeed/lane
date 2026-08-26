//! Command surface. Every command returns an exit code; failures bubble as errors.

use crate::args::{self, Budget, Installable, NoteAddArgs, NoteCommand, NoteReplaceArgs, Parsed};
use crate::audit;
use crate::capture;
use crate::git::{self, git, try_git};
use crate::prompt;
use crate::store::{self, BODY, FRESH, LANE_DIR, MISSING, SIG, UNVERIFIABLE};
use crate::syntax::{Anchor, Qualification, Source};
use crate::util::now_iso;
use crate::worktree as wt;
use anyhow::{Result, bail};
use rustix::fs::{FlockOperation, flock};
use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Takes the stream it will be written to; for `new` that is stderr, not stdout.
fn bold(text: &str, tty: bool) -> String {
    if tty {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn run() -> Result<i32> {
    // A usage error is the reader's, not the program's: it exits 2, the way a
    // command line has always distinguished "you typed it wrong" from "it failed".
    let parsed = match args::parse(std::env::args_os().skip(1).collect()) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("error: {err:#}");
            return Ok(2);
        }
    };

    match parsed {
        Parsed::Help(topic) => {
            println!("{}", topic.text());
            Ok(0)
        }
        Parsed::Version => {
            println!("lane {}", args::VERSION);
            Ok(0)
        }
        Parsed::Init => init(),
        Parsed::New(args) => new(&args.name, args.base.as_deref(), args.dirty),
        Parsed::Ls { json } => ls(json),
        Parsed::Enter { name } => enter(&name),
        Parsed::Exit => exit(),
        Parsed::Anchors { path, json } => anchors(&path, json),
        Parsed::Note(command) => match command {
            NoteCommand::Add(args) => note_add(args),
            NoteCommand::Edit { id } => edit(&id),
            NoteCommand::Replace(args) => note_replace(args),
            NoteCommand::Confirm { id } => confirm(&id),
            NoteCommand::Retire { id } => retire(&id),
            NoteCommand::Restore { id } => restore(&id),
            NoteCommand::Pin { id } => pin(&id, true),
            NoteCommand::Unpin { id } => pin(&id, false),
        },
        Parsed::Install(what) => match what {
            Installable::Hooks => hooks_install(),
            Installable::Skill => skill_install(),
        },
        Parsed::Uninstall(what) => match what {
            Installable::Hooks => hooks_uninstall(),
            Installable::Skill => skill_uninstall(),
        },
        Parsed::Capture { rev } => {
            capture::capture(&rev);
            Ok(0)
        }
        Parsed::Why(args) => why(args.path.as_deref(), args.anchor.as_deref(), args.json),
        Parsed::Check { json } => check(json),
        Parsed::Audit(args) => audit_cmd(&args.base, &args.budget, args.json),
        Parsed::Merge(args) => merge(
            args.name.as_deref(),
            args.base.as_deref(),
            args.keep,
            args.squash,
            &args.budget,
        ),
        Parsed::Push(args) => push(args.name.as_deref(), args.base.as_deref(), &args.budget),
        Parsed::Prune { dry_run } => prune(dry_run),
        Parsed::Rm(args) => rm(&args.name, args.force),
        Parsed::Shellenv => shellenv(),
    }
}

struct LandingLock {
    _file: File,
}

impl LandingLock {
    fn acquire(trunk: &str) -> Result<Self> {
        let common = git::layout(&std::env::current_dir()?)?.common_dir;
        let path = common.join("lane").join(format!("{trunk}.lock"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        flock(&file, FlockOperation::NonBlockingLockExclusive)
            .map_err(|err| anyhow::anyhow!("another lane is landing; try again ({err})"))?;
        Ok(Self { _file: file })
    }
}

const PROTOCOL: &str = "\n<!-- lane:protocol -->\n\
## Context memory\n\n\
- Before editing a file, read `.lane/memory/<path>/` if it exists, or run `lane why <path>`.\n\
- Record non-obvious findings with `lane note add <path> -a <anchor> \"...\"`.\n\
- Do not edit `.lane/` by hand; `lane merge` manages it.\n\
- Detailed workflow lives in the `lane` skill; run `lane install skill` if it is absent.\n\
<!-- /lane:protocol -->\n";

/// The protocol as shipped before markers, recognised so an upgrade can replace it.
/// Never edit this; it is a fingerprint of what is already in users' files, not content.
const PROTOCOL_V1: &str = "## Context memory\n\n\
- Before editing a file, read `.lane/memory/<path>/` if it exists, or run `lane why <path>`.\n\
- Record non-obvious findings with `lane note -a <anchor> \"...\"`.\n\
- Do not edit `.lane/` by hand; `lane done` manages it.\n";

const PROTOCOL_START: &str = "<!-- lane:protocol -->";
const PROTOCOL_END: &str = "<!-- /lane:protocol -->";

enum ProtocolAction {
    Write,
    Append,
    Current,
    Replace(Range<usize>),
    Upgrade(Range<usize>),
    Refuse,
}

fn context_memory_section(existing: &str) -> Option<Range<usize>> {
    let start = existing
        .match_indices("## Context memory")
        .find_map(|(start, _)| {
            (start == 0 || existing.as_bytes()[start - 1] == b'\n').then_some(start)
        })?;
    let end = existing[start..]
        .find("\n## ")
        .map_or(existing.len(), |offset| start + offset + 1);
    Some(start..end)
}

fn protocol_action(existing: Option<&str>) -> ProtocolAction {
    let Some(existing) = existing else {
        return ProtocolAction::Write;
    };

    if let Some(start) = existing.find(PROTOCOL_START) {
        let Some(end) = existing[start..].find(PROTOCOL_END) else {
            return ProtocolAction::Refuse;
        };
        let end = start + end + PROTOCOL_END.len();
        return if &existing[start..end] == PROTOCOL.trim() {
            ProtocolAction::Current
        } else {
            ProtocolAction::Replace(start..end)
        };
    }

    let Some(section) = context_memory_section(existing) else {
        return ProtocolAction::Append;
    };
    if existing[section.clone()].trim() == PROTOCOL_V1.trim() {
        ProtocolAction::Upgrade(section)
    } else {
        ProtocolAction::Refuse
    }
}

fn replace_protocol(existing: &mut String, range: Range<usize>) {
    let ends_at_eof = range.end == existing.len();
    existing.replace_range(range, PROTOCOL.trim());
    if ends_at_eof && !existing.ends_with('\n') {
        existing.push('\n');
    }
}

fn write_protocol(agents: &Path) -> Result<i32> {
    let existing = agents
        .exists()
        .then(|| std::fs::read_to_string(agents))
        .transpose()?;
    match protocol_action(existing.as_deref()) {
        ProtocolAction::Write => {
            std::fs::write(agents, format!("# AGENTS\n{PROTOCOL}"))?;
            println!("wrote {} protocol", agents.display());
        }
        ProtocolAction::Append => {
            let mut file = std::fs::OpenOptions::new().append(true).open(agents)?;
            write!(file, "{PROTOCOL}")?;
            println!("added protocol to {}", agents.display());
        }
        ProtocolAction::Current => println!("{} protocol is current", agents.display()),
        ProtocolAction::Replace(range) => {
            let mut existing = existing.expect("existing protocol file");
            replace_protocol(&mut existing, range);
            std::fs::write(agents, existing)?;
            println!("repaired {} protocol", agents.display());
        }
        ProtocolAction::Upgrade(range) => {
            let mut existing = existing.expect("existing protocol file");
            replace_protocol(&mut existing, range);
            std::fs::write(agents, existing)?;
            println!("upgraded {} protocol", agents.display());
        }
        ProtocolAction::Refuse => {
            eprintln!(
                "{} has a Context memory section lane did not write; replace it with:",
                agents.display()
            );
            eprintln!("{}", PROTOCOL.trim());
            return Ok(1);
        }
    }
    Ok(0)
}

const SKILL: &str = include_str!("../assets/skill.md");
const SKILL_HOME: &str = ".agents/skills";
const SKILL_PATH: &str = ".agents/skills/lane/SKILL.md";
const SKILL_ALIAS: &str = ".claude/skills";
const SKILL_ALIAS_TARGET: &str = "../.agents/skills";

const POST_COMMIT_MARKER: &str = "# lane: capture Why trailers";
const POST_COMMIT_END: &str = "# lane: end";
const POST_COMMIT_BLOCK: &str = "# lane: capture Why trailers\n\
if [ -d \"$(git rev-parse --git-path rebase-merge 2>/dev/null)\" ] || [ -d \"$(git rev-parse --git-path rebase-apply 2>/dev/null)\" ]; then\n\
  :\n\
elif command -v lane >/dev/null 2>&1; then\n\
  lane capture HEAD || true\n\
elif git log -1 --format=%B | grep -qi '^Why:'; then\n\
  echo \"lane: not on PATH, so the Why trailer in this commit was not captured\" >&2\n\
  echo \"lane: run 'lane capture HEAD' once lane is installed to record it\" >&2\n\
fi\n\
# lane: end\n";
/// The post-commit body as shipped before end markers, recognised so it can be replaced.
/// Never edit this; it is a fingerprint of what is already in users' hooks, not content.
const POST_COMMIT_V1: &str = "# lane: capture Why trailers\n\
command -v lane >/dev/null 2>&1 && lane capture HEAD || true\n";
const PREPARE_MARKER: &str = "# lane: offer the Why form when an editor will open";
const PREPARE_END: &str = "# lane: end";
const PREPARE_BLOCK: &str = "# lane: offer the Why form when an editor will open\n\
case \"$2\" in\n\
  \"\"|template) printf '\\n# Why: <path>#<anchor> | what must stay true (optional, lane note)\\n' >> \"$1\" ;;\n\
esac\n\
# lane: end\n";
// Prepare's V1 and current body are identical today; keep this fingerprint for its first update.
const PREPARE_V1: &str = "# lane: offer the Why form when an editor will open\n\
case \"$2\" in\n\
  \"\"|template) printf '\\n# Why: <path>#<anchor> | what must stay true (optional, lane note)\\n' >> \"$1\" ;;\n\
esac\n";

struct HookSpec {
    path: PathBuf,
    marker: &'static str,
    end: &'static str,
    block: &'static str,
    legacy: &'static str,
}

fn hook_specs(dir: &Path) -> [HookSpec; 2] {
    [
        HookSpec {
            path: dir.join("post-commit"),
            marker: POST_COMMIT_MARKER,
            end: POST_COMMIT_END,
            block: POST_COMMIT_BLOCK,
            legacy: POST_COMMIT_V1,
        },
        HookSpec {
            path: dir.join("prepare-commit-msg"),
            marker: PREPARE_MARKER,
            end: PREPARE_END,
            block: PREPARE_BLOCK,
            legacy: PREPARE_V1,
        },
    ]
}

enum HookAction {
    Current(Range<usize>),
    Replace(Range<usize>),
    Upgrade(Range<usize>),
    Refuse,
    Foreign,
}

fn hook_action(existing: &str, spec: &HookSpec) -> HookAction {
    let Some(start) = existing.find(spec.marker) else {
        return HookAction::Foreign;
    };

    if let Some(end) = existing[start..].find(spec.end) {
        let mut end = start + end + spec.end.len();
        if existing[end..].starts_with('\n') {
            end += 1;
        }
        return if &existing[start..end] == spec.block {
            HookAction::Current(start..end)
        } else {
            HookAction::Replace(start..end)
        };
    }

    if let Some(legacy) = existing[start..].find(spec.legacy) {
        let start = start + legacy;
        return HookAction::Upgrade(start..start + spec.legacy.len());
    }

    HookAction::Refuse
}

/// What the hook files hold, next to the blocks this binary ships.
#[derive(Debug, PartialEq, Eq)]
enum HooksState {
    Absent,
    Current,
    Stale(Vec<PathBuf>),
}

/// A hook is written once, at install, so a block that changed in a later release
/// reaches an existing repository only through another install. Nothing else rewrites
/// it, and a stale block goes on running the behavior its release shipped.
fn hooks_state(dir: &Path) -> HooksState {
    let mut stale = Vec::new();
    let mut current = false;
    for spec in hook_specs(dir) {
        let Ok(existing) = std::fs::read_to_string(&spec.path) else {
            continue;
        };
        match hook_action(&existing, &spec) {
            HookAction::Current(_) => current = true,
            HookAction::Replace(_) | HookAction::Upgrade(_) => stale.push(spec.path),
            // Someone else's hook, or ours past recognition; install already says so.
            HookAction::Foreign | HookAction::Refuse => {}
        }
    }
    match (stale.is_empty(), current) {
        (false, _) => HooksState::Stale(stale),
        (true, true) => HooksState::Current,
        (true, false) => HooksState::Absent,
    }
}

/// Named on stderr, so a `--json` reader keeps one parseable document on stdout.
fn warn_stale_hooks() {
    let Ok(dir) = hooks_dir() else {
        return;
    };
    if let HooksState::Stale(paths) = hooks_state(&dir) {
        for path in paths {
            eprintln!(
                "warning: {} is out of date; run `lane install hooks`",
                path.display()
            );
        }
    }
}

fn hooks_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(git(
        &["rev-parse", "--git-path", "hooks"],
        None,
    )?))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn hooks_install() -> Result<i32> {
    let dir = hooks_dir()?;
    let specs = hook_specs(&dir);
    let mut foreign = false;
    for spec in &specs {
        if !spec.path.exists() {
            continue;
        }
        let existing = std::fs::read_to_string(&spec.path)?;
        if matches!(
            hook_action(&existing, spec),
            HookAction::Foreign | HookAction::Refuse
        ) {
            foreign = true;
            eprintln!("{} already exists; add this block:", spec.path.display());
            eprint!("{}", spec.block);
        }
    }
    if foreign {
        return Ok(1);
    }

    std::fs::create_dir_all(&dir)?;
    for spec in specs {
        if spec.path.exists() {
            let mut existing = std::fs::read_to_string(&spec.path)?;
            match hook_action(&existing, &spec) {
                HookAction::Current(_) => println!("{} is current", spec.path.display()),
                HookAction::Replace(range) => {
                    existing.replace_range(range, spec.block);
                    std::fs::write(&spec.path, existing)?;
                    println!("upgraded {}", spec.path.display());
                }
                HookAction::Upgrade(range) => {
                    existing.replace_range(range, spec.block);
                    std::fs::write(&spec.path, existing)?;
                    println!("upgraded {}", spec.path.display());
                }
                HookAction::Refuse | HookAction::Foreign => unreachable!("checked above"),
            }
            continue;
        }
        std::fs::write(&spec.path, format!("#!/bin/sh\n{}", spec.block))?;
        make_executable(&spec.path)?;
        println!("installed {}", spec.path.display());
    }
    Ok(0)
}

fn hooks_uninstall() -> Result<i32> {
    let specs = hook_specs(&hooks_dir()?);
    let mut foreign = false;
    for spec in &specs {
        if !spec.path.exists() {
            continue;
        }
        let existing = std::fs::read_to_string(&spec.path)?;
        if matches!(hook_action(&existing, spec), HookAction::Refuse) {
            foreign = true;
            eprintln!("{} already exists; add this block:", spec.path.display());
            eprint!("{}", spec.block);
        }
    }
    if foreign {
        return Ok(1);
    }

    for spec in specs {
        if !spec.path.exists() {
            continue;
        }
        let mut existing = std::fs::read_to_string(&spec.path)?;
        let range = match hook_action(&existing, &spec) {
            HookAction::Foreign => continue,
            HookAction::Current(range)
            | HookAction::Replace(range)
            | HookAction::Upgrade(range) => range,
            HookAction::Refuse => unreachable!("checked above"),
        };
        existing.replace_range(range, "");
        let remaining = existing;
        if remaining.trim() == "#!/bin/sh" {
            std::fs::remove_file(&spec.path)?;
        } else {
            std::fs::write(&spec.path, remaining)?;
        }
        println!("removed lane block from {}", spec.path.display());
    }
    Ok(0)
}

fn skill_install() -> Result<i32> {
    let root = git::repo_root()?;
    let path = root.join(SKILL_PATH);
    if path.exists() {
        if std::fs::read_to_string(&path)? == SKILL {
            println!("{} already installed", path.display());
            link_alias(&root)?;
            return Ok(0);
        }
        eprintln!(
            "{} was edited; remove it, then re-run to install the current skill",
            path.display()
        );
        return Ok(1);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, SKILL)?;
    println!("installed {}", path.display());
    link_alias(&root)?;
    Ok(0)
}

/// Harnesses disagree on where a skill lives, so the other spelling is a link, never a copy:
/// one file cannot drift from itself. An alias already on disk is left alone, because a real
/// `.claude/skills` directory holds skills that are not ours.
fn link_alias(root: &Path) -> Result<()> {
    let alias = root.join(SKILL_ALIAS);
    if std::fs::symlink_metadata(&alias).is_ok() {
        return Ok(());
    }
    if let Some(dir) = alias.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::os::unix::fs::symlink(SKILL_ALIAS_TARGET, &alias)?;
    println!("linked {} -> {SKILL_ALIAS_TARGET}", alias.display());
    Ok(())
}

fn skill_uninstall() -> Result<i32> {
    let root = git::repo_root()?;
    let path = root.join(SKILL_PATH);
    if !path.exists() {
        return Ok(0);
    }
    std::fs::remove_file(&path)?;
    println!("removed {}", path.display());
    // Only the lane/ dir we just emptied; never .agents/skills/, which may hold others.
    let Some(dir) = path.parent() else {
        return Ok(0);
    };
    if std::fs::read_dir(dir)?.next().is_some() {
        return Ok(0);
    }
    std::fs::remove_dir(dir)?;
    // The alias is ours only while it points where we put it, and only until something else
    // installs a skill beside ours.
    let alias = root.join(SKILL_ALIAS);
    let ours =
        std::fs::read_link(&alias).is_ok_and(|target| target == Path::new(SKILL_ALIAS_TARGET));
    if ours && std::fs::read_dir(root.join(SKILL_HOME))?.next().is_none() {
        std::fs::remove_file(&alias)?;
        println!("removed {}", alias.display());
    }
    Ok(0)
}

fn init() -> Result<i32> {
    let root = wt::main_root()?;
    let lane = root.join(LANE_DIR);
    std::fs::create_dir_all(&lane)?;
    std::fs::write(lane.join(".gitkeep"), "")?;

    let agents = root.join("AGENTS.md");
    if write_protocol(&agents)? != 0 {
        return Ok(1);
    }

    let (ok, detail) = crate::cow::probe(&root);
    println!("initialized .lane/ and AGENTS.md protocol");
    println!(
        "reflink on this filesystem: {} ({detail})",
        if ok { "yes" } else { "no" }
    );
    if !ok {
        println!("  lanes will still work as plain worktrees; ignored files will not be cloned");
    }
    match hooks_state(&hooks_dir()?) {
        HooksState::Absent => println!(
            "capture commit decisions with `lane install hooks` (`lane uninstall hooks` removes them)"
        ),
        HooksState::Current => println!("commit hooks are current"),
        HooksState::Stale(paths) => {
            for path in paths {
                println!(
                    "{} is out of date; run `lane install hooks`",
                    path.display()
                );
            }
        }
    }
    Ok(0)
}

fn new(name: &str, base: Option<&str>, dirty: bool) -> Result<i32> {
    let created = wt::create(name, base, dirty)?;
    // Progress goes to stderr so stdout carries the path alone, as `enter` does. Bold only
    // for a terminal: a captured path must not carry escapes.
    let info = &mut std::io::stderr();
    for note in &created.notes {
        writeln!(info, "  {note}")?;
    }
    writeln!(info, "  {}", created.stats)?;
    let tty = std::io::stdout().is_terminal();
    println!("{}", bold(&created.path.to_string_lossy(), tty));
    Ok(0)
}

#[derive(serde::Serialize)]
struct LaneRow {
    name: String,
    path: String,
    branch: String,
    state: &'static str,
    dirty: bool,
    pending_notes: usize,
}

fn ls(json: bool) -> Result<i32> {
    let root = wt::main_root()?;
    let lanes = wt::list_lanes(&root);
    let dirty: Vec<bool> = std::thread::scope(|scope| {
        let workers: Vec<_> = lanes
            .iter()
            .map(|lane| scope.spawn(|| wt::is_dirty(&lane.path)))
            .collect();
        workers
            .into_iter()
            .map(|handle| handle.join().expect("status worker panicked"))
            .collect()
    });
    let rows: Vec<_> = lanes
        .into_iter()
        .zip(dirty)
        .map(|(lane, dirty)| {
            let name = lane
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let upstream = try_git(&["rev-parse", "@{upstream}"], Some(&lane.path));
            // Only marked lanes pay for a probe, and a retired upstream settles it before
            // the expensive one runs.
            let state = if store::is_landed(&lane.path)
                && (wt::upstream_gone(&root, &lane.branch)
                    || wt::contained_in(&root, &wt::trunk_name(&root), &lane.branch))
            {
                "landed"
            } else if !upstream.is_empty()
                && try_git(&["rev-parse", "HEAD"], Some(&lane.path)) == upstream
            {
                "pushed"
            } else {
                "open"
            };
            LaneRow {
                name,
                path: lane.path.to_string_lossy().to_string(),
                branch: lane.branch,
                state,
                dirty,
                pending_notes: store::pending_count(&lane.path),
            }
        })
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(0);
    }
    if rows.is_empty() {
        println!("no lanes");
        return Ok(0);
    }
    for row in rows {
        let name = row.name;
        let state = row.state;
        let dirty = if row.dirty { "dirty" } else { "clean" };
        println!(
            "{name:<20} {state:<7} {dirty:<6} {} pending note(s)",
            row.pending_notes
        );
    }
    Ok(0)
}

fn prune(dry_run: bool) -> Result<i32> {
    let root = wt::main_root()?;
    // A retired upstream is read from a cache that only empties on a prune. Failing here
    // costs accuracy, never the command: fall through and decide on the refs we have.
    if !try_git(&["remote"], Some(&root)).trim().is_empty()
        && !git::git_ok(&["fetch", "--prune", "--quiet"], Some(&root))
    {
        eprintln!("warning: fetch failed; deciding on cached remote refs");
    }
    let trunk = wt::trunk_name(&root);
    let lanes = wt::list_lanes(&root);

    let mut removed = 0;
    let mut skipped = 0;
    for lane in lanes {
        // Identity, not name. A lane with no id was not made by `lane new`, and prune is
        // destructive, so an unrecognised lane is left alone rather than guessed at.
        if !store::is_landed(&lane.path) {
            continue;
        }
        let name = lane
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // A landing marker says the work reached trunk, not that the lane kept nothing
        // back: notes written after it, or an edit never committed, are still only here.
        let losses = wt::losses(&root, &lane.path, &lane.branch, &trunk);
        if !losses.is_empty() {
            eprintln!("skipped {name}: {}", losses.join(", "));
            skipped += 1;
            continue;
        }
        if dry_run {
            println!("would remove {name}");
            removed += 1;
            continue;
        }
        wt::remove(&name)?;
        println!("removed {name}");
        removed += 1;
    }

    if removed == 0 && skipped == 0 {
        println!("no landed lanes");
        let behind = try_git(
            &["rev-list", "--count", &format!("{trunk}..origin/{trunk}")],
            Some(&root),
        );
        if behind.parse::<u32>().unwrap_or(0) > 0 {
            println!("  origin/{trunk} is {behind} commit(s) ahead; fetch and merge it first");
        }
    }
    Ok(i32::from(skipped > 0))
}

/// A named lane git still holds; an unregistered directory would resolve to the primary
/// worktree and land trunk onto itself.
fn registered_lane(root: &Path, name: &str) -> Result<PathBuf> {
    let dest = lane_named(root, name)?;
    if !wt::registered(root, &dest) {
        bail!("{name} is not a lane; git has no worktree there");
    }
    Ok(dest)
}

fn lane_named(root: &Path, name: &str) -> Result<PathBuf> {
    let dest = wt::lanes_dir(root).join(name);
    if !dest.exists() {
        bail!("no lane named {name}");
    }
    Ok(dest)
}

/// Move the shell, or say why it did not.
///
/// A terminal on stdout means nothing captured the destination, so no shell function ran.
fn move_to(dest: &Path) -> Result<i32> {
    println!("{}", dest.display());
    if std::io::stdout().is_terminal() {
        eprintln!(
            "note: no shell integration; add `eval \"$(lane shellenv)\"` to cd automatically"
        );
    }
    Ok(0)
}

fn enter(name: &str) -> Result<i32> {
    move_to(&lane_named(&wt::main_root()?, name)?)
}

fn exit() -> Result<i32> {
    move_to(&wt::main_root()?)
}

#[derive(serde::Serialize)]
struct AnchorRow {
    anchor: String,
    start: usize,
    end: usize,
}

fn anchors(path: &str, json: bool) -> Result<i32> {
    let root = git::repo_root()?;
    let rel = store::rel_to_repo(&root, path)?;
    let target = root.join(&rel);
    if !target.is_file() {
        bail!("{rel} is not a regular file");
    }
    let source_text = std::fs::read_to_string(&target)?;
    let source = Source::new(&source_text, &rel);
    let anchors = source.anchors();

    if json {
        let rows: Vec<_> = anchors
            .into_iter()
            .map(|anchor| AnchorRow {
                anchor: anchor.value,
                start: anchor.span.start,
                end: anchor.span.end,
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        for anchor in anchors {
            println!(
                "{}\t{}-{}",
                anchor.value, anchor.span.start, anchor.span.end
            );
        }
    }
    Ok(0)
}

fn note_add(args: NoteAddArgs) -> Result<i32> {
    let root = git::repo_root()?;
    let rel = store::rel_to_repo(&root, &args.path)?;
    if !root.join(&rel).exists() {
        // Otherwise the note is promoted, found missing, and atticked in the same audit.
        bail!("{rel} does not exist; note not recorded");
    }
    let source_text = std::fs::read_to_string(root.join(&rel))?;
    let source = Source::new(&source_text, &rel);
    let (text, anchor) = match args.anchor {
        Some(anchor) => {
            let anchor = qualify_anchor(&source, &rel, &anchor)?;
            let (text, _) = note_text(args.text, None)?;
            (text, anchor)
        }
        None if args.text.is_some() => {
            let (text, _) = note_text(args.text, None)?;
            (text, store::WHOLE_FILE.to_string())
        }
        None => {
            let anchors = source.anchors();
            let (text, selected) = note_text(None, Some((&rel, &anchors)))?;
            let selected = selected.expect("interactive add requested an anchor selector");
            (text, selected.value)
        }
    };
    append_note(&root, text, rel.clone(), anchor.clone(), String::new())?;
    println!("noted -> {rel}#{anchor}");
    Ok(0)
}

fn note_replace(args: NoteReplaceArgs) -> Result<i32> {
    let root = git::repo_root()?;
    let predecessor = store::resolve_id(&root, &args.id)?;
    let predecessor_id = predecessor.meta.id.clone();
    if predecessor.unreadable {
        bail!("cannot replace note {predecessor_id}: frontmatter is unreadable");
    }
    let (path, anchor) = replacement_target(&predecessor, args.path, args.anchor);
    let rel = store::rel_to_repo(&root, &path)?;
    if !root.join(&rel).exists() {
        bail!("{rel} does not exist; replacement not recorded");
    }
    let source_text = std::fs::read_to_string(root.join(&rel))?;
    let source = Source::new(&source_text, &rel);
    let anchor = qualify_anchor(&source, &rel, &anchor)?;
    let (text, _) = note_text(args.text, None)?;
    if store::pending_supersedes(&root, &predecessor_id)? {
        bail!("note {predecessor_id} already has a pending replacement");
    }
    append_note(
        &root,
        text,
        rel.clone(),
        anchor.clone(),
        predecessor_id.clone(),
    )?;
    println!("replacement queued -> {predecessor_id} {rel}#{anchor}");
    Ok(0)
}

fn replacement_target(
    predecessor: &crate::note::Note,
    path: Option<String>,
    anchor: Option<String>,
) -> (String, String) {
    (
        path.unwrap_or_else(|| predecessor.path()),
        anchor.unwrap_or_else(|| predecessor.meta.anchor.clone()),
    )
}

fn qualify_anchor(source: &Source, rel: &str, anchor: &str) -> Result<String> {
    Ok(match source.qualify(anchor) {
        Qualification::Canonical(candidate) => candidate.value,
        Qualification::Ambiguous(choices) => {
            let choices = choices
                .into_iter()
                .map(|choice| format!("  {}", choice.value))
                .collect::<Vec<_>>()
                .join("\n");
            bail!("anchor {anchor:?} is ambiguous in {rel}; choose one:\n{choices}");
        }
        Qualification::NotFound => {
            eprintln!("warning: anchor {anchor:?} not found in {rel}; note recorded anyway");
            anchor.to_string()
        }
        Qualification::Unparsed => {
            eprintln!("warning: {rel} has no grammar; note will be kept but not checked for drift");
            anchor.to_string()
        }
    })
}

fn note_text(
    text: Option<String>,
    selection: Option<(&str, &[Anchor])>,
) -> Result<(String, Option<Anchor>)> {
    if let Some(text) = text {
        return Ok((text, None));
    }
    let stdin = std::io::stdin();
    let stderr = std::io::stderr();
    if !stdin.is_terminal() || !stderr.is_terminal() {
        bail!("note text omitted outside a terminal; pass text explicitly");
    }
    let mut input = stdin.lock();
    let mut output = stderr.lock();
    let selected = match selection {
        Some((path, anchors)) => Some(prompt::select_anchor(
            &mut input,
            &mut output,
            path,
            anchors,
        )?),
        None => None,
    };
    let text = prompt::read_note(&mut input, &mut output)?;
    Ok((text, selected))
}

fn append_note(
    root: &Path,
    text: String,
    path: String,
    anchor: String,
    supersedes: String,
) -> Result<()> {
    store::append_pending(
        root,
        &store::PendingNote {
            text,
            path,
            anchor,
            at: now_iso(),
            supersedes,
        },
    )
}

#[derive(serde::Serialize)]
struct WhyRow {
    id: String,
    path: String,
    anchor: String,
    created: String,
    note: String,
}

fn why(path: Option<&str>, anchor: Option<&str>, json: bool) -> Result<i32> {
    let root = git::repo_root()?;
    let rel = match path {
        Some(p) => store::rel_scope(&root, p)?,
        None => None,
    };
    let mut notes = store::load_notes(&root, rel.as_deref());
    if let Some(want) = anchor {
        notes.retain(|n| n.meta.anchor == want);
    }
    if json {
        notes.sort_by(|a, b| {
            a.path()
                .cmp(&b.path())
                .then_with(|| a.meta.anchor.cmp(&b.meta.anchor))
                .then_with(|| a.meta.id.cmp(&b.meta.id))
        });
        let rows: Vec<_> = notes
            .into_iter()
            .map(|note| {
                let path = note.path();
                WhyRow {
                    id: note.meta.id,
                    path,
                    anchor: note.meta.anchor,
                    created: note.meta.created,
                    note: note.body.trim().to_string(),
                }
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(0);
    }
    if notes.is_empty() {
        println!("no context for {}", rel.as_deref().unwrap_or("repo"));
        return Ok(0);
    }

    // Only an argument that is itself a note's path named one file; a directory never does.
    let one_file = rel
        .as_deref()
        .is_some_and(|rel| notes.iter().any(|n| n.path() == rel));

    let mut groups: std::collections::BTreeMap<(String, String), Vec<_>> = Default::default();
    for note in notes {
        groups
            .entry((note.path(), note.meta.anchor.clone()))
            .or_default()
            .push(note);
    }

    for (index, ((file, anchor), mut group)) in groups.into_iter().enumerate() {
        if index > 0 {
            println!();
        }
        if one_file {
            println!("[{anchor}]");
        } else {
            println!("[{file}#{anchor}]");
        }
        group.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
        for note in group {
            println!(
                "  - {} · {}",
                &note.meta.id[..10.min(note.meta.id.len())],
                note.meta.created.get(..10).unwrap_or("?")
            );
            println!("    {}", note.body.trim().replace('\n', "\n    "));
        }
    }
    Ok(0)
}

fn confirm(id: &str) -> Result<i32> {
    // The whole id, not the prefix you typed, so you can see which note you confirmed.
    println!("confirmed -> {}", audit::confirm(&git::repo_root()?, id)?);
    Ok(0)
}

fn edit(id: &str) -> Result<i32> {
    let stdin = std::io::stdin();
    let stderr = std::io::stderr();
    if !stdin.is_terminal() || !stderr.is_terminal() {
        bail!(
            "note edit requires stdin and stderr terminals; use a direct `lane note <action>` command instead"
        );
    }

    let root = git::repo_root()?;
    let note = store::resolve_id(&root, id)?;
    let full_id = note.meta.id.clone();
    if note.unreadable {
        bail!(
            "cannot edit note {full_id}: frontmatter is unreadable; use `lane note retire {full_id}` to preserve it unchanged"
        );
    }
    let path = note.path();
    let status = store::Checker::new(&root).check(&note).tier;
    let pinned = note.meta.pinned;
    let mut input = stdin.lock();
    let mut output = stderr.lock();
    writeln!(output, "Editing {full_id}")?;
    writeln!(output, "  {path}#{}", note.meta.anchor)?;
    writeln!(
        output,
        "  status: {status}{}",
        if pinned { ", pinned" } else { "" }
    )?;
    writeln!(output, "  {}", note.body.trim().replace('\n', "\n  "))?;
    let action = prompt::select_edit_action(&mut input, &mut output, pinned)?;
    let replacement = if action == prompt::EditAction::Replace {
        Some(prompt::read_replacement(&mut input, &mut output)?)
    } else {
        None
    };
    drop(output);
    drop(input);

    match action {
        prompt::EditAction::Confirm => confirm(&full_id),
        prompt::EditAction::Replace => note_replace(NoteReplaceArgs {
            id: full_id,
            text: replacement,
            path: None,
            anchor: None,
        }),
        prompt::EditAction::Retire => retire(&full_id),
        prompt::EditAction::SetPinned(pinned) => pin(&full_id, pinned),
    }
}

fn retire(id: &str) -> Result<i32> {
    let root = git::repo_root()?;
    let mut note = store::resolve_id(&root, id)?;
    let id = note.meta.id.clone();
    if store::pending_supersedes(&root, &id)? {
        bail!("note {id} has a pending replacement and cannot be retired");
    }
    store::evict(&root, &mut note, "retired explicitly")?;
    println!("retired -> {id}");
    Ok(0)
}

fn restore(id: &str) -> Result<i32> {
    let root = git::repo_root()?;
    let mut note = store::resolve_retired_id(&root, id)?;
    let id = note.meta.id.clone();
    let path = note.path();
    let anchor = note.meta.anchor.clone();
    match store::Checker::new(&root).check(&note).tier {
        MISSING => eprintln!(
            "warning: {path}#{anchor} does not resolve; the next audit may retire note {id} again unless it is pinned"
        ),
        UNVERIFIABLE => {
            eprintln!("warning: {path}#{anchor} cannot be verified with the available grammars")
        }
        _ => {}
    }
    store::restore(&root, &mut note)?;
    println!("restored -> {id}");
    Ok(0)
}

fn pin(id: &str, pinned: bool) -> Result<i32> {
    let root = git::repo_root()?;
    let mut note = store::resolve_id(&root, id)?;
    let id = note.meta.id.clone();
    store::set_pinned(&mut note, pinned)?;
    println!("{} -> {id}", if pinned { "pinned" } else { "unpinned" });
    Ok(0)
}

fn check_json_rows(
    rows: &[(store::Check, &crate::note::Note)],
    checker: &mut store::Checker,
) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|(res, note)| {
            let mut row = serde_json::json!({
                "id": note.meta.id, "path": note.path(),
                "anchor": note.meta.anchor, "tier": res.tier,
                "note": note.body.trim(),
            });
            if res.tier != FRESH {
                row["span"] = serde_json::json!(checker.span_text(note));
            }
            row
        })
        .collect()
}

fn check(json: bool) -> Result<i32> {
    warn_stale_hooks();
    let root = git::repo_root()?;
    let notes = store::load_notes(&root, None);
    let mut checker = store::Checker::new(&root);
    let rows: Vec<_> = notes
        .iter()
        .map(|note| (checker.check(note), note))
        .collect();

    if json {
        let out = check_json_rows(&rows, &mut checker);
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(0);
    }

    let mut missing = 0;
    for tier in store::TIERS {
        let count = rows.iter().filter(|(res, _)| res.tier == tier).count();
        if tier == MISSING {
            missing = count;
        }
        println!("{tier:<18} {count}");
    }

    // A count says something drifted and not which note, which is a dead end when
    // the next thing you type needs an id. Grouped, because what you do about a
    // note depends on which tier it is in.
    for tier in store::TIERS.iter().filter(|tier| **tier != FRESH) {
        let mut group = rows.iter().filter(|(res, _)| res.tier == *tier).peekable();
        if group.peek().is_none() {
            continue;
        }
        println!("\n[{tier}]");
        for (_, note) in group {
            println!(
                "{} {}  {}#{}",
                mark(tier),
                &note.meta.id[..10.min(note.meta.id.len())],
                note.path(),
                note.meta.anchor
            );
        }
    }
    Ok(i32::from(missing > 0))
}

fn mark(tier: &str) -> &'static str {
    match tier {
        BODY => "~",
        SIG => "!",
        MISSING => "x",
        UNVERIFIABLE => "?",
        _ => " ",
    }
}

fn options(base: &str, budget: &Budget) -> audit::Options {
    audit::Options {
        base: base.to_string(),
        max_notes: budget.max_notes,
        max_chars: budget.max_chars,
    }
}

fn audit_cmd(base: &str, budget: &Budget, json: bool) -> Result<i32> {
    let root = git::repo_root()?;
    let _lock = if root == wt::main_root()? {
        Some(LandingLock::acquire(&wt::trunk_name(&root))?)
    } else {
        None
    };
    let out = audit::run(&root, &options(base, budget))?;

    if json {
        let value = serde_json::json!({
            "created": out.created.iter().map(|n| &n.meta.id).collect::<Vec<_>>(),
            "checked": out.stats,
            "drifted": out.drifted.iter().map(|n| serde_json::json!({
                "id": n.meta.id, "path": n.path(), "anchor": n.meta.anchor,
            })).collect::<Vec<_>>(),
            "evicted": out.evicted.iter().map(|(n, why)| serde_json::json!({
                "id": n.meta.id, "reason": why,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(0);
    }
    audit::report(&out, &mut std::io::stdout())?;
    Ok(0)
}

struct Prepared {
    lane_path: PathBuf,
    root: PathBuf,
    branch: String,
    base: String,
    _lock: LandingLock,
}

enum Preparation {
    Ready(Prepared),
    Exit(i32),
}

fn prepare(
    name: Option<&str>,
    base: Option<&str>,
    budget: &Budget,
    check_blocking: bool,
    info: &mut dyn Write,
) -> Result<Preparation> {
    let root = wt::main_root()?;
    let lane_path = match name {
        Some(name) => registered_lane(&root, name)?,
        None => git::repo_root()?,
    };
    if lane_path == root {
        eprintln!("error: not inside a lane; name one");
        return Ok(Preparation::Exit(2));
    }
    let branch = git::current_branch(&lane_path);
    let base = base
        .map(str::to_string)
        .unwrap_or_else(|| wt::lane_base(&root, &branch));

    if wt::is_dirty(&lane_path) {
        eprintln!(
            "error: lane has uncommitted changes; commit or stash first, the rebase will refuse them either way"
        );
        return Ok(Preparation::Exit(1));
    }

    if check_blocking {
        let blocked = wt::blocking_changes(&root, &base, &branch);
        if !blocked.is_empty() {
            eprintln!(
                "error: {base} has uncommitted changes to {}; commit or stash there first",
                blocked.join(", ")
            );
            return Ok(Preparation::Exit(1));
        }
    }

    let lock = LandingLock::acquire(&base)?;

    git(&["rebase", &base], Some(&lane_path))?;
    writeln!(info, "rebased onto {base}")?;

    let out = audit::run(&lane_path, &options(&base, budget))?;
    audit::report(&out, info)?;

    // Per-worktree state: a reused branch name starts without this marker.
    if store::lane_id(&lane_path).is_empty() {
        eprintln!("warning: lane has no id; `lane prune` will not recognise this landing");
    }

    if stage_memory(&lane_path) {
        git(
            &["commit", "-q", "-m", &format!("lane: sync {branch} memory")],
            Some(&lane_path),
        )?;
        writeln!(info, "committed memory update")?;
    }

    // After the memory commit: the marker records the tip, and that commit is part of what
    // lands. Marking earlier would date the landing one commit short of itself.
    store::mark_landed(&lane_path)?;

    Ok(Preparation::Ready(Prepared {
        lane_path,
        root,
        branch,
        base,
        _lock: lock,
    }))
}

/// Stage lane's own paths one at a time: a pathspec matching nothing fails the whole add.
fn stage_memory(lane_path: &Path) -> bool {
    for path in [LANE_DIR, "AGENTS.md"] {
        try_git(&["add", "--", path], Some(lane_path));
    }
    !git::git_ok(&["diff", "--cached", "--quiet"], Some(lane_path))
}

fn merge(
    name: Option<&str>,
    base: Option<&str>,
    keep: bool,
    squash: bool,
    budget: &Budget,
) -> Result<i32> {
    let info: &mut dyn Write = &mut std::io::stdout();
    let prepared = match prepare(name, base, budget, true, info)? {
        Preparation::Ready(prepared) => prepared,
        Preparation::Exit(code) => return Ok(code),
    };
    let Prepared {
        lane_path,
        root,
        branch,
        base,
        _lock,
    } = prepared;
    let upstream = upstream_branch(&lane_path, &branch);

    if squash {
        git(&["merge", "--squash", &branch], Some(&root))?;
        git(
            &["commit", "-q", "-m", &format!("lane: merged {branch}")],
            Some(&root),
        )?;
        writeln!(info, "squash-merged {branch} into {base}")?;
    } else {
        wt::fast_forward(&root, &base, &branch)?;
        writeln!(info, "fast-forwarded {base}")?;
    }

    if let Some(upstream) = upstream {
        if !git::git_ok(
            &["merge-base", "--is-ancestor", &upstream.tip, &base],
            Some(&root),
        ) {
            let number = pull_request_number(&lane_path, &upstream.remote, &upstream.tip);
            let detail = number.map_or_else(String::new, |number| format!(" #{number}"));
            eprintln!(
                "warning: pushed pull request{detail} remains open; close it on {}",
                upstream.remote
            );
        }
    }

    if !keep {
        std::env::set_current_dir(&root)?;
        let name = lane_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        wt::remove(&name)?;
        writeln!(info, "removed lane {branch}")?;
    }
    Ok(0)
}

fn branch_remote(lane_path: &Path) -> Option<String> {
    let upstream = try_git(
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        Some(lane_path),
    );
    if let Some((remote, _)) = upstream.split_once('/') {
        return Some(remote.into());
    }
    git::git_ok(&["config", "--get", "remote.origin.url"], Some(lane_path)).then(|| "origin".into())
}

struct Upstream {
    remote: String,
    tip: String,
}

fn upstream_branch(lane_path: &Path, branch: &str) -> Option<Upstream> {
    let name = try_git(
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        Some(lane_path),
    );
    let tip = try_git(&["rev-parse", "@{upstream}"], Some(lane_path));
    if name.is_empty() || tip.is_empty() {
        return None;
    }
    let key = format!("branch.{branch}.remote");
    let configured = try_git(&["config", "--get", &key], Some(lane_path));
    let remote = if configured.is_empty() {
        name.split_once('/')?.0.to_string()
    } else {
        configured
    };
    (remote != ".").then_some(Upstream { remote, tip })
}

fn pull_request_number(lane_path: &Path, remote: &str, tip: &str) -> Option<String> {
    let refs = try_git(&["ls-remote", remote, "refs/pull/*/head"], Some(lane_path));
    refs.lines().find_map(|line| {
        let (sha, name) = line.split_once('\t')?;
        (sha == tip)
            .then_some(name)
            .and_then(|name| name.strip_prefix("refs/pull/"))
            .and_then(|name| name.strip_suffix("/head"))
            .map(Into::into)
    })
}

fn push(name: Option<&str>, base: Option<&str>, budget: &Budget) -> Result<i32> {
    let prepared = match prepare(name, base, budget, false, &mut std::io::stdout())? {
        Preparation::Ready(prepared) => prepared,
        Preparation::Exit(code) => return Ok(code),
    };
    let remote = branch_remote(&prepared.lane_path).ok_or_else(|| {
        anyhow::anyhow!("no remote configured; add an upstream or `git remote add origin <url>`")
    })?;
    git(
        &[
            "push",
            "--force-with-lease",
            "--force-if-includes",
            "-u",
            &remote,
            &prepared.branch,
        ],
        Some(&prepared.lane_path),
    )?;
    println!("pushed {} to {remote}", prepared.branch);
    Ok(0)
}

fn rm(name: &str, force: bool) -> Result<i32> {
    let root = wt::main_root()?;
    if !force {
        let trunk = wt::trunk_name(&root);
        let path = wt::lanes_dir(&root).join(name);
        let losses = wt::losses(&root, &path, name, &trunk);
        if !losses.is_empty() {
            eprintln!("kept lane {name}: {}", losses.join(", "));
            eprintln!("  lane rm {name} --force   to discard it anyway");
            return Ok(1);
        }
    }
    wt::remove(name)?;
    println!("removed lane {name}");
    Ok(0)
}

fn shellenv() -> Result<i32> {
    println!(
        r#"lane() {{
  local p
  case "$1" in
    new|enter|switch|exit) p=$(command lane "$@") || return; cd "$p" ;;
    # Read the destination first: merge deletes the worktree, and nothing runs from a
    # directory that no longer exists. Stay put unless it was ours that went away.
    merge) p=$(command lane exit) || return; command lane "$@" || return
           cd "$PWD" 2>/dev/null || cd "$p" ;;
    *)     command lane "$@" ;;
  esac
}}"#
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_targets_inherit_and_accept_independent_overrides() {
        let root = tempfile::tempdir().unwrap();
        let file = store::note_dir(root.path(), "src/auth.rs").join("01M0A-note.md");
        let note = crate::note::Note::new(
            crate::note::Meta {
                id: "01M0A".into(),
                anchor: "fn verify".into(),
                ..Default::default()
            },
            "keep",
        );
        note.write(&file).unwrap();
        let note = crate::note::parse(&file).unwrap();

        assert_eq!(
            replacement_target(&note, None, None),
            ("src/auth.rs".into(), "fn verify".into())
        );
        assert_eq!(
            replacement_target(&note, Some("src/token.rs".into()), None),
            ("src/token.rs".into(), "fn verify".into())
        );
        assert_eq!(
            replacement_target(&note, None, Some("fn refresh".into())),
            ("src/auth.rs".into(), "fn refresh".into())
        );
    }

    #[test]
    fn flock_excludes_a_second_open_file_description() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("landing.lock");
        let first = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .unwrap();
        let second = OpenOptions::new().write(true).open(path).unwrap();

        flock(&first, FlockOperation::NonBlockingLockExclusive).unwrap();
        assert!(flock(&second, FlockOperation::NonBlockingLockExclusive).is_err());
    }

    #[test]
    fn a_current_delimited_hook_is_current() {
        let specs = hook_specs(Path::new(".git/hooks"));
        assert!(matches!(
            hook_action(POST_COMMIT_BLOCK, &specs[0]),
            HookAction::Current(_)
        ));
    }

    #[test]
    fn a_changed_delimited_hook_is_replaced() {
        let specs = hook_specs(Path::new(".git/hooks"));
        let changed = POST_COMMIT_BLOCK.replace("lane capture HEAD", "lane capture changed");
        assert!(matches!(
            hook_action(&changed, &specs[0]),
            HookAction::Replace(_)
        ));
    }

    #[test]
    fn an_exact_legacy_hook_is_upgraded() {
        let specs = hook_specs(Path::new(".git/hooks"));
        assert!(matches!(
            hook_action(POST_COMMIT_V1, &specs[0]),
            HookAction::Upgrade(_)
        ));
    }

    #[test]
    fn a_foreign_hook_is_refused() {
        let specs = hook_specs(Path::new(".git/hooks"));
        assert!(matches!(
            hook_action("#!/bin/sh\necho mine\n", &specs[0]),
            HookAction::Foreign
        ));
    }

    fn hooks_dir_holding(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).unwrap();
        }
        dir
    }

    #[test]
    fn hooks_nobody_installed_are_absent() {
        let dir = hooks_dir_holding(&[]);
        assert_eq!(hooks_state(dir.path()), HooksState::Absent);

        let foreign = hooks_dir_holding(&[("post-commit", "#!/bin/sh\necho mine\n")]);
        assert_eq!(hooks_state(foreign.path()), HooksState::Absent);
    }

    #[test]
    fn hooks_holding_the_shipped_blocks_are_current() {
        let dir = hooks_dir_holding(&[
            ("post-commit", &format!("#!/bin/sh\n{POST_COMMIT_BLOCK}")),
            ("prepare-commit-msg", &format!("#!/bin/sh\n{PREPARE_BLOCK}")),
        ]);
        assert_eq!(hooks_state(dir.path()), HooksState::Current);
    }

    #[test]
    fn a_block_from_an_earlier_release_is_stale() {
        let older = POST_COMMIT_BLOCK.replace("lane capture HEAD", "lane capture");
        let dir = hooks_dir_holding(&[
            ("post-commit", &format!("#!/bin/sh\n{older}")),
            ("prepare-commit-msg", &format!("#!/bin/sh\n{PREPARE_BLOCK}")),
        ]);
        assert_eq!(
            hooks_state(dir.path()),
            HooksState::Stale(vec![dir.path().join("post-commit")])
        );

        let legacy = hooks_dir_holding(&[("post-commit", &format!("#!/bin/sh\n{POST_COMMIT_V1}"))]);
        assert_eq!(
            hooks_state(legacy.path()),
            HooksState::Stale(vec![legacy.path().join("post-commit")])
        );
    }

    #[test]
    fn the_previous_marked_protocol_is_replaced() {
        let existing = format!(
            "# AGENTS\n{}",
            PROTOCOL.replace("lane note add <path>", "lane note -p <path>")
        );
        assert!(matches!(
            protocol_action(Some(&existing)),
            ProtocolAction::Replace(_)
        ));
    }

    #[test]
    fn replacing_a_final_protocol_ends_with_one_newline() {
        let root = tempfile::tempdir().unwrap();
        let agents = root.path().join("AGENTS.md");
        std::fs::write(
            &agents,
            format!(
                "# AGENTS\n{}",
                PROTOCOL
                    .replace("lane note add", "lane note edited")
                    .trim_end()
            ),
        )
        .unwrap();

        write_protocol(&agents).unwrap();

        let rewritten = std::fs::read_to_string(agents).unwrap();
        assert!(rewritten.ends_with('\n'));
        assert!(!rewritten.ends_with("\n\n"));
    }

    #[test]
    fn replacing_a_protocol_preserves_the_following_section() {
        let root = tempfile::tempdir().unwrap();
        let agents = root.path().join("AGENTS.md");
        let following = "\n## Something else\n\nKeep this byte-identical.\n";
        std::fs::write(
            &agents,
            format!(
                "# AGENTS\n{}{}",
                PROTOCOL.trim().replace("lane note add", "lane note edited"),
                following
            ),
        )
        .unwrap();

        write_protocol(&agents).unwrap();

        let rewritten = std::fs::read_to_string(agents).unwrap();
        assert_eq!(
            rewritten,
            format!("# AGENTS\n{}{}", PROTOCOL.trim(), following)
        );
    }

    #[test]
    fn unrelated_agents_content_gets_a_protocol_appended() {
        let existing = "# AGENTS\n\nSome existing house rules for this project.\n";
        assert!(matches!(
            protocol_action(Some(existing)),
            ProtocolAction::Append
        ));
    }

    #[test]
    fn the_exact_legacy_protocol_is_upgraded() {
        let existing = format!("# AGENTS\n\n{PROTOCOL_V1}");
        assert!(matches!(
            protocol_action(Some(&existing)),
            ProtocolAction::Upgrade(_)
        ));
    }

    #[test]
    fn a_modified_legacy_protocol_is_refused() {
        let existing = format!(
            "# AGENTS\n\n{}",
            PROTOCOL_V1.replace("lane note -a", "lane note -p <path> -a")
        );
        assert!(matches!(
            protocol_action(Some(&existing)),
            ProtocolAction::Refuse
        ));
    }

    #[test]
    fn check_json_includes_only_drifted_spans() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(
            root.path().join("src/drift.rs"),
            "pub fn drift() {\n    println!(\"old\");\n}\n",
        )
        .unwrap();
        std::fs::write(root.path().join("src/fresh.rs"), "pub fn fresh() {}\n").unwrap();
        for (path, anchor, text) in [
            ("src/drift.rs", "fn drift", "drift note"),
            ("src/fresh.rs", "fn fresh", "fresh note"),
        ] {
            store::append_pending(
                root.path(),
                &store::PendingNote {
                    text: text.into(),
                    path: path.into(),
                    anchor: anchor.into(),
                    at: "2026-08-21T00:00:00Z".into(),
                    supersedes: String::new(),
                },
            )
            .unwrap();
        }
        store::promote_pending(root.path()).unwrap();
        std::fs::write(
            root.path().join("src/drift.rs"),
            "pub fn drift() {\n    println!(\"new\");\n}\n",
        )
        .unwrap();

        let notes = store::load_notes(root.path(), None);
        let mut checker = store::Checker::new(root.path());
        let rows: Vec<_> = notes
            .iter()
            .map(|note| (checker.check(note), note))
            .collect();
        let json = check_json_rows(&rows, &mut checker);
        let drifted = json.iter().find(|row| row["tier"] == BODY).unwrap();
        let fresh = json.iter().find(|row| row["tier"] == FRESH).unwrap();

        assert_eq!(drifted["note"], "drift note");
        assert!(drifted["span"].as_str().unwrap().contains("\"new\""));
        assert_eq!(fresh["note"], "fresh note");
        assert!(fresh.get("span").is_none());
    }

    fn repository() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for args in [
            &["init", "-qb", "main"][..],
            &["config", "user.email", "t@t.t"],
            &["config", "user.name", "t"],
        ] {
            git(args, Some(root)).unwrap();
        }
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        git(&["add", "-A"], Some(root)).unwrap();
        git(&["commit", "-qm", "base"], Some(root)).unwrap();
        temp
    }

    fn write_note(root: &Path) {
        let dir = root.join(LANE_DIR).join("memory/main.rs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("01M0A-note.md"), "note\n").unwrap();
    }

    #[test]
    fn memory_stages_where_the_repository_has_no_agents_file() {
        let temp = repository();
        let root = temp.path();
        write_note(root);

        assert!(stage_memory(root));
        assert_eq!(
            git(&["diff", "--cached", "--name-only"], Some(root)).unwrap(),
            ".lane/memory/main.rs/01M0A-note.md"
        );
    }

    #[test]
    fn an_ignored_store_stages_nothing_to_commit() {
        let temp = repository();
        let root = temp.path();
        std::fs::write(root.join(".gitignore"), ".lane/\n").unwrap();
        git(&["add", "-A"], Some(root)).unwrap();
        git(&["commit", "-qm", "ignore the store"], Some(root)).unwrap();
        write_note(root);

        assert!(!stage_memory(root));
    }

    #[test]
    fn an_agents_file_stages_alongside_the_store() {
        let temp = repository();
        let root = temp.path();
        write_note(root);
        std::fs::write(root.join("AGENTS.md"), "# AGENTS\n").unwrap();

        assert!(stage_memory(root));
        assert_eq!(
            git(&["diff", "--cached", "--name-only"], Some(root)).unwrap(),
            ".lane/memory/main.rs/01M0A-note.md\nAGENTS.md"
        );
    }

    #[test]
    fn a_directory_git_does_not_know_is_not_a_lane() {
        let temp = repository();
        let root = temp.path();

        assert!(
            registered_lane(root, "ghost")
                .unwrap_err()
                .to_string()
                .contains("no lane named ghost")
        );

        std::fs::create_dir_all(wt::lanes_dir(root).join("ghost")).unwrap();

        assert!(
            registered_lane(root, "ghost")
                .unwrap_err()
                .to_string()
                .contains("git has no worktree there")
        );
    }
}
