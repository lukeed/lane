//! Command surface. Every command returns an exit code; failures bubble as errors.

use crate::args::{self, Budget, Installable, Parsed};
use crate::audit;
use crate::capture;
use crate::git::{self, git, try_git};
use crate::store::{self, BODY, FRESH, LANE_DIR, MISSING, SIG, UNVERIFIABLE};
use crate::syntax::{Resolution, Source};
use crate::util::now_iso;
use crate::worktree as wt;
use anyhow::{Result, bail};
use rustix::fs::{FlockOperation, flock};
use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Takes the stream it will be written to; under --cd that is stderr, not stdout.
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
        Parsed::New(args) => new(&args.name, args.base.as_deref(), args.dirty, args.cd),
        Parsed::Ls => ls(),
        Parsed::Path { name } => path(&name),
        Parsed::Note(args) => note(
            &args.text,
            &args.path,
            &args.anchor,
            args.supersedes.as_deref(),
        ),
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
        Parsed::Why(args) => why(args.path.as_deref(), args.anchor.as_deref()),
        Parsed::Holds { id } => holds(&id),
        Parsed::Check { json } => check(json),
        Parsed::Audit(args) => audit_cmd(&args.base, &args.budget, args.json),
        Parsed::Done(args) => done(
            args.base.as_deref(),
            args.keep,
            args.cd,
            args.squash,
            &args.budget,
        ),
        Parsed::Push(args) => push(args.base.as_deref(), &args.budget),
        Parsed::Sweep { dry_run } => sweep(dry_run),
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
- Record non-obvious findings with `lane note -p <path> -a <anchor> \"...\"`.\n\
- Do not edit `.lane/` by hand; `lane done` manages it.\n\
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
const SKILL_PATH: &str = ".agents/skills/lane/SKILL.md";

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
    let path = git::repo_root()?.join(SKILL_PATH);
    if path.exists() {
        if std::fs::read_to_string(&path)? == SKILL {
            println!("{} already installed", path.display());
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
    Ok(0)
}

fn skill_uninstall() -> Result<i32> {
    let path = git::repo_root()?.join(SKILL_PATH);
    if !path.exists() {
        return Ok(0);
    }
    std::fs::remove_file(&path)?;
    println!("removed {}", path.display());
    // Only the lane/ dir we just emptied; never .agents/ or .agents/skills/, which may hold others.
    if let Some(dir) = path.parent() {
        if std::fs::read_dir(dir)?.next().is_none() {
            std::fs::remove_dir(dir)?;
        }
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
    println!(
        "capture commit decisions with `lane install hooks` (`lane uninstall hooks` removes them)"
    );
    Ok(0)
}

fn new(name: &str, base: Option<&str>, dirty: bool, cd: bool) -> Result<i32> {
    let created = wt::create(name, base, dirty)?;
    // With --cd, stdout is reserved for the path so the shell can capture it without a pipe.
    let tty = if cd {
        std::io::stderr().is_terminal()
    } else {
        std::io::stdout().is_terminal()
    };
    let info: &mut dyn std::io::Write = if cd {
        &mut std::io::stderr()
    } else {
        &mut std::io::stdout()
    };
    for note in &created.notes {
        writeln!(info, "  {note}")?;
    }
    writeln!(info, "  {}", created.stats)?;
    writeln!(info, "{}", bold(&created.path.to_string_lossy(), tty))?;
    if cd {
        println!("{}", created.path.display());
    }
    Ok(0)
}

fn ls() -> Result<i32> {
    let root = wt::main_root()?;
    let lanes = wt::list_lanes(&root);
    if lanes.is_empty() {
        println!("no lanes");
        return Ok(0);
    }
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
    for (lane, dirty) in lanes.into_iter().zip(dirty) {
        let name = lane
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let dirty = if dirty { "dirty" } else { "clean" };
        let upstream = try_git(&["rev-parse", "@{upstream}"], Some(&lane.path));
        // Only marked lanes pay for the containment probe.
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
    }
    Ok(0)
}

fn sweep(dry_run: bool) -> Result<i32> {
    let root = wt::main_root()?;
    let trunk = wt::trunk_name(&root);
    let lanes = wt::list_lanes(&root);

    let mut removed = 0;
    let mut skipped = 0;
    for lane in lanes {
        // Identity, not name. A lane with no id was not made by `lane new`, and sweep is
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

fn path(name: &str) -> Result<i32> {
    let dest = wt::lanes_dir(&wt::main_root()?).join(name);
    if !dest.exists() {
        bail!("no lane named {name}");
    }
    println!("{}", dest.display());
    Ok(0)
}

fn note(text: &str, path: &str, anchor: &str, supersedes: Option<&str>) -> Result<i32> {
    let root = git::repo_root()?;
    // Resolved here, so the queue carries a whole id and promotion cannot be
    // handed a prefix that has since grown ambiguous.
    let supersedes = match supersedes {
        Some(id) => store::resolve_id(&root, id)?.meta.id,
        None => String::new(),
    };
    let rel = store::rel_to_repo(&root, path)?;
    if !root.join(&rel).exists() {
        // Otherwise the note is promoted, found missing, and atticked in the same audit.
        bail!("{rel} does not exist; note not recorded");
    }
    let source_text = std::fs::read_to_string(root.join(&rel))?;
    let source = Source::new(&source_text, &rel);
    match source.resolve_detail(anchor) {
        Resolution::Found(_) => {}
        Resolution::NotFound => {
            eprintln!("warning: anchor {anchor:?} not found in {rel}; note recorded anyway");
        }
        Resolution::Unparsed => {
            eprintln!("warning: {rel} has no grammar; note will be kept but not checked for drift");
        }
    }
    store::append_pending(
        &root,
        &store::PendingNote {
            text: text.to_string(),
            path: rel.clone(),
            anchor: anchor.to_string(),
            branch: git::current_branch(),
            at: now_iso(),
            supersedes: supersedes.clone(),
        },
    )?;
    println!("noted -> {rel}#{anchor}");
    Ok(0)
}

fn why(path: Option<&str>, anchor: Option<&str>) -> Result<i32> {
    let root = git::repo_root()?;
    let rel = match path {
        Some(p) => Some(store::rel_to_repo(&root, p)?),
        None => None,
    };
    let mut notes = store::load_notes(&root, rel.as_deref());
    if let Some(want) = anchor {
        notes.retain(|n| n.meta.anchor == want);
    }
    if notes.is_empty() {
        println!("no context for {}", rel.as_deref().unwrap_or("repo"));
        return Ok(0);
    }

    let mut groups: std::collections::BTreeMap<(String, String), Vec<_>> = Default::default();
    for note in notes {
        groups
            .entry((note.path(), note.meta.anchor.clone()))
            .or_default()
            .push(note);
    }

    let mut checker = store::Checker::new(&root);
    for ((file, anchor), mut group) in groups {
        println!("\n{file}#{anchor}");
        group.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
        for note in group {
            let tier = checker.check(&note).tier;
            let mark = mark(tier);
            let tail = if tier == FRESH {
                String::new()
            } else {
                format!("   [{tier}]")
            };
            println!(
                "  {mark} {}{tail}",
                note.body.trim().replace('\n', "\n    ")
            );
            println!(
                "      {} · {} · {}",
                &note.meta.id[..10.min(note.meta.id.len())],
                if note.meta.branch.is_empty() {
                    "?"
                } else {
                    &note.meta.branch
                },
                note.meta.created.get(..10).unwrap_or("?")
            );
        }
    }
    Ok(0)
}

fn holds(id: &str) -> Result<i32> {
    // The whole id, not the prefix you typed, so you can see which note you held.
    println!("holds -> {}", audit::holds(&git::repo_root()?, id)?);
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
    base: Option<&str>,
    budget: &Budget,
    check_blocking: bool,
    info: &mut dyn Write,
) -> Result<Preparation> {
    let lane_path = git::repo_root()?;
    let root = wt::main_root()?;
    if lane_path == root {
        eprintln!("error: not inside a lane");
        return Ok(Preparation::Exit(2));
    }
    let branch = git::current_branch();
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
        eprintln!("warning: lane has no id; `lane sweep` will not recognise this landing");
    }
    store::mark_landed(&lane_path)?;

    let changed = try_git(
        &["status", "--porcelain", "--", LANE_DIR, "AGENTS.md"],
        Some(&lane_path),
    );
    if !changed.trim().is_empty() {
        try_git(&["add", LANE_DIR, "AGENTS.md"], Some(&lane_path));
        git(
            &["commit", "-q", "-m", &format!("lane: sync {branch} memory")],
            Some(&lane_path),
        )?;
        writeln!(info, "committed memory update")?;
    }

    Ok(Preparation::Ready(Prepared {
        lane_path,
        root,
        branch,
        base,
        _lock: lock,
    }))
}

fn done(base: Option<&str>, keep: bool, cd: bool, squash: bool, budget: &Budget) -> Result<i32> {
    let info: &mut dyn Write = if cd {
        &mut std::io::stderr()
    } else {
        &mut std::io::stdout()
    };
    let prepared = match prepare(base, budget, true, info)? {
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
    if cd {
        println!("{}", root.display());
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

fn push(base: Option<&str>, budget: &Budget) -> Result<i32> {
    let prepared = match prepare(base, budget, false, &mut std::io::stdout())? {
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
  case "$1" in
    new)  shift; local p; p=$(command lane new --cd "$@")  || return; cd "$p" ;;
    cd)   shift; local p; p=$(command lane path "$1")      || return; cd "$p" ;;
    done) shift; local p; p=$(command lane done --cd "$@") || return; cd "$p" ;;
    *)    command lane "$@" ;;
  esac
}}"#
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_different_marked_protocol_is_replaced() {
        let existing = format!(
            "# AGENTS\n{}",
            PROTOCOL.replace("lane note -p", "lane note edited")
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
                    .replace("lane note -p", "lane note edited")
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
                PROTOCOL.trim().replace("lane note -p", "lane note edited"),
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
                    branch: "main".into(),
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
}
