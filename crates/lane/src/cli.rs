//! Command surface. Every command returns an exit code; failures bubble as errors.

use crate::audit;
use crate::capture;
use crate::git::{self, git, try_git};
use crate::review;
use crate::store::{self, BODY, CONTEXT_DIR, FRESH, MISSING, PENDING, SIG, UNVERIFIABLE};
use crate::syntax::{Resolution, Source};
use crate::util::now_iso;
use crate::worktree as wt;
use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

const MAX_NOTES: usize = 5;
const MAX_CHARS: usize = 1200;

#[derive(Parser, Debug)]
#[command(
    name = "lane",
    version,
    about = "copy-on-write worktrees with memory that survives them"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Debug, Clone)]
struct ReviewArgs {
    /// drift reviewer (default: auto)
    #[arg(long, value_parser = ["auto", "none", "cmd", "anthropic"])]
    review: Option<String>,
    /// command receiving JSON on stdin, e.g. 'claude -p'
    #[arg(long)]
    review_cmd: Option<String>,
    #[arg(long, default_value_t = 20)]
    review_max: usize,
}

#[derive(Args, Debug, Clone)]
struct BudgetArgs {
    #[arg(long, default_value_t = MAX_NOTES)]
    max_notes: usize,
    #[arg(long, default_value_t = MAX_CHARS)]
    max_chars: usize,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// scaffold memory + merge rules, probe reflink
    Init,
    /// create a CoW lane
    New {
        name: String,
        #[arg(long)]
        base: Option<String>,
        /// carry uncommitted work into the lane
        #[arg(long)]
        dirty: bool,
        /// print path last, for shell fn
        #[arg(long)]
        cd: bool,
    },
    /// list lanes
    Ls,
    /// print a lane's path
    Path { name: String },
    /// record a finding
    Note {
        text: String,
        #[arg(short, long)]
        path: String,
        #[arg(short, long, default_value = "@file")]
        anchor: String,
    },
    /// manage commit-message capture hooks
    Hooks {
        #[command(subcommand)]
        action: HookAction,
    },
    #[command(hide = true)]
    Capture { rev: String },
    /// show context for a path
    Why {
        path: Option<String>,
        #[arg(short, long)]
        anchor: Option<String>,
    },
    /// staleness report
    Check {
        #[arg(long)]
        json: bool,
    },
    /// promote, re-anchor, rank, evict
    Audit {
        #[arg(long, default_value = "")]
        base: String,
        #[command(flatten)]
        budget: BudgetArgs,
        #[command(flatten)]
        review: ReviewArgs,
        #[arg(long)]
        json: bool,
    },
    /// rebase, audit, fast-forward, remove
    Done {
        #[arg(long)]
        trunk: Option<String>,
        #[arg(long)]
        keep: bool,
        #[arg(long)]
        cd: bool,
        #[command(flatten)]
        budget: BudgetArgs,
        #[command(flatten)]
        review: ReviewArgs,
    },
    /// discard a lane without landing it
    Rm {
        name: String,
        /// discard commits trunk does not have
        #[arg(long)]
        force: bool,
    },
    /// print shell integration
    Shellenv,
}

#[derive(Subcommand, Debug)]
enum HookAction {
    /// install commit-message capture hooks
    Install,
    /// remove lane's commit-message capture hooks
    Uninstall,
}

/// Takes the stream it will be written to; under --cd that is stderr, not stdout.
fn bold(text: &str, tty: bool) -> String {
    if tty {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub fn run() -> Result<i32> {
    match Cli::parse().command {
        Command::Init => init(),
        Command::New {
            name,
            base,
            dirty,
            cd,
        } => new(&name, base.as_deref(), dirty, cd),
        Command::Ls => ls(),
        Command::Path { name } => path(&name),
        Command::Note { text, path, anchor } => note(&text, &path, &anchor),
        Command::Hooks { action } => match action {
            HookAction::Install => hooks_install(),
            HookAction::Uninstall => hooks_uninstall(),
        },
        Command::Capture { rev } => {
            capture::capture(&rev);
            Ok(0)
        }
        Command::Why { path, anchor } => why(path.as_deref(), anchor.as_deref()),
        Command::Check { json } => check(json),
        Command::Audit {
            base,
            budget,
            review,
            json,
        } => audit_cmd(&base, &budget, &review, json),
        Command::Done {
            trunk,
            keep,
            cd,
            budget,
            review,
        } => done(trunk.as_deref(), keep, cd, &budget, &review),
        Command::Rm { name, force } => rm(&name, force),
        Command::Shellenv => shellenv(),
    }
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if existing.contains(line) {
        return Ok(());
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(file)?;
    }
    writeln!(file, "{line}")?;
    Ok(())
}

const PROTOCOL: &str = "\n## Context memory\n\n\
- Before editing a file, read `.context/-/<path>/` if it exists, or run `lane why <path>`.\n\
- Record non-obvious findings with `lane note -a <anchor> \"...\"`.\n\
- Do not edit `.context/` by hand; `lane done` manages it.\n";

const POST_COMMIT_MARKER: &str = "# lane: capture Why trailers";
const POST_COMMIT_BLOCK: &str = "# lane: capture Why trailers\n\
command -v lane >/dev/null 2>&1 && lane capture HEAD || true\n";
const PREPARE_MARKER: &str = "# lane: offer the Why form when an editor will open";
const PREPARE_BLOCK: &str = "# lane: offer the Why form when an editor will open\n\
case \"$2\" in\n\
  \"\"|template) printf '\\n# Why: <path>#<anchor> | what must stay true (optional, lane note)\\n' >> \"$1\" ;;\n\
esac\n";

struct HookSpec {
    path: PathBuf,
    marker: &'static str,
    block: &'static str,
}

fn hook_specs(dir: &Path) -> [HookSpec; 2] {
    [
        HookSpec {
            path: dir.join("post-commit"),
            marker: POST_COMMIT_MARKER,
            block: POST_COMMIT_BLOCK,
        },
        HookSpec {
            path: dir.join("prepare-commit-msg"),
            marker: PREPARE_MARKER,
            block: PREPARE_BLOCK,
        },
    ]
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
        if !existing.contains(spec.marker) {
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
            println!("{} already installed", spec.path.display());
            continue;
        }
        std::fs::write(&spec.path, format!("#!/bin/sh\n{}", spec.block))?;
        make_executable(&spec.path)?;
        println!("installed {}", spec.path.display());
    }
    Ok(0)
}

fn hooks_uninstall() -> Result<i32> {
    for spec in hook_specs(&hooks_dir()?) {
        if !spec.path.exists() {
            continue;
        }
        let existing = std::fs::read_to_string(&spec.path)?;
        if !existing.contains(spec.marker) {
            continue;
        }
        let remaining = existing.replace(spec.block, "");
        if remaining.trim() == "#!/bin/sh" {
            std::fs::remove_file(&spec.path)?;
        } else {
            std::fs::write(&spec.path, remaining)?;
        }
        println!("removed lane block from {}", spec.path.display());
    }
    Ok(0)
}

fn init() -> Result<i32> {
    let root = wt::main_root()?;
    let context = root.join(CONTEXT_DIR);
    std::fs::create_dir_all(&context)?;
    std::fs::write(context.join(".gitkeep"), "")?;

    let attrs = root.join(".gitattributes");
    // The log is the one genuinely append-only file, which is what union merge is for.
    // Notes never conflict because they are never modified.
    append_line(&attrs, &format!("{CONTEXT_DIR}/log/*.jsonl merge=union"))?;

    let agents = root.join("AGENTS.md");
    if !agents.exists() {
        std::fs::write(&agents, format!("# AGENTS\n{PROTOCOL}"))?;
    } else if !std::fs::read_to_string(&agents)?.contains("## Context memory") {
        let mut file = std::fs::OpenOptions::new().append(true).open(&agents)?;
        write!(file, "{PROTOCOL}")?;
    }

    let ignore = root.join(".gitignore");
    append_line(&ignore, PENDING)?;

    let (ok, detail) = crate::cow::probe(&root);
    println!("initialized .context/, union merge rules, AGENTS.md protocol");
    println!(
        "reflink on this filesystem: {} ({detail})",
        if ok { "yes" } else { "no" }
    );
    if !ok {
        println!("  lanes will still work as plain worktrees; ignored files will not be cloned");
    }
    println!(
        "capture commit decisions with `lane hooks install` (`lane hooks uninstall` removes them)"
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
    for lane in lanes {
        let name = lane
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let dirty = if wt::is_dirty(&lane.path) {
            "dirty"
        } else {
            "clean"
        };
        println!(
            "{name:<20} {:<24} {dirty:<6} {} pending note(s)",
            lane.branch,
            store::pending_count(&lane.path)
        );
    }
    Ok(0)
}

fn path(name: &str) -> Result<i32> {
    let dest = wt::lanes_dir(&wt::main_root()?).join(name);
    if !dest.exists() {
        bail!("no lane named {name}");
    }
    println!("{}", dest.display());
    Ok(0)
}

fn note(text: &str, path: &str, anchor: &str) -> Result<i32> {
    let root = git::repo_root()?;
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
    let mut shown = Vec::new();
    for ((file, anchor), mut group) in groups {
        println!("\n{file}#{anchor}");
        group.sort_by(|a, b| a.meta.id.cmp(&b.meta.id));
        for note in group {
            let tier = checker.check(&note).tier;
            let mark = match tier {
                BODY => "~",
                SIG => "!",
                MISSING => "x",
                UNVERIFIABLE => "?",
                _ => " ",
            };
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
            shown.push(note.meta.id);
        }
    }
    store::bump_reads(&root, &shown)?;
    Ok(0)
}

fn check(json: bool) -> Result<i32> {
    let root = git::repo_root()?;
    let notes = store::load_notes(&root, None);
    let mut checker = store::Checker::new(&root);
    let rows: Vec<(&str, _)> = notes.iter().map(|n| (checker.check(n).tier, n)).collect();

    if json {
        let out: Vec<_> = rows
            .iter()
            .map(|(tier, n)| {
                serde_json::json!({
                    "id": n.meta.id, "path": n.path(),
                    "anchor": n.meta.anchor, "tier": tier,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(0);
    }

    let mut missing = 0;
    for tier in store::TIERS {
        let count = rows.iter().filter(|(t, _)| *t == tier).count();
        if tier == MISSING {
            missing = count;
        }
        println!("{tier:<18} {count}");
    }
    Ok(i32::from(missing > 0))
}

fn options(base: &str, budget: &BudgetArgs, review: &ReviewArgs) -> audit::Options {
    audit::Options {
        base: base.to_string(),
        max_notes: budget.max_notes,
        max_chars: budget.max_chars,
        review_limit: review.review_max,
    }
}

fn audit_cmd(base: &str, budget: &BudgetArgs, review: &ReviewArgs, json: bool) -> Result<i32> {
    let root = git::repo_root()?;
    let reviewer = review::build(review.review.as_deref(), review.review_cmd.as_deref());
    let out = audit::run(&root, &options(base, budget, review), reviewer.as_ref())?;

    if json {
        let value = serde_json::json!({
            "created": out.created.iter().map(|n| &n.meta.id).collect::<Vec<_>>(),
            "checked": out.stats,
            "needs_review": out.review.iter().map(|n| serde_json::json!({
                "id": n.meta.id, "path": n.path(), "anchor": n.meta.anchor,
            })).collect::<Vec<_>>(),
            "evicted": out.evicted.iter().map(|(n, why)| serde_json::json!({
                "id": n.meta.id, "reason": why,
            })).collect::<Vec<_>>(),
            "reviewer": out.reviewer,
            "verdicts": out.reviewed.iter().map(|(n, v, new)| serde_json::json!({
                "id": n.meta.id, "verdict": v,
                "replacement": new.as_ref().map(|x| x.meta.id.clone()),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(0);
    }
    audit::report(&out, &mut std::io::stdout())?;
    Ok(0)
}

/// rebase -> audit -> commit memory -> fast-forward trunk -> remove lane.
///
/// Audit runs AFTER the rebase so spans resolve against the post-rebase tree.
fn done(
    trunk: Option<&str>,
    keep: bool,
    cd: bool,
    budget: &BudgetArgs,
    review: &ReviewArgs,
) -> Result<i32> {
    let lane_path = git::repo_root()?;
    let root = wt::main_root()?;
    if lane_path == root {
        eprintln!("error: not inside a lane");
        return Ok(2);
    }
    let branch = git::current_branch();
    let trunk = trunk
        .map(str::to_string)
        .unwrap_or_else(|| wt::trunk_name(&root));

    if wt::is_dirty(&lane_path) {
        eprintln!(
            "error: lane has uncommitted changes; commit or stash first, the rebase will refuse them either way"
        );
        return Ok(1);
    }

    let blocked = wt::blocking_changes(&root, &trunk, &branch);
    if !blocked.is_empty() {
        eprintln!(
            "error: {} has uncommitted changes to {}; commit or stash there first",
            trunk,
            blocked.join(", ")
        );
        return Ok(1);
    }

    let info: &mut dyn std::io::Write = if cd {
        &mut std::io::stderr()
    } else {
        &mut std::io::stdout()
    };
    git(&["rebase", &trunk], Some(&lane_path))?;
    writeln!(info, "rebased onto {trunk}")?;

    let reviewer = review::build(review.review.as_deref(), review.review_cmd.as_deref());
    let out = audit::run(
        &lane_path,
        &options(&trunk, budget, review),
        reviewer.as_ref(),
    )?;
    audit::report(&out, info)?;

    // Fold this lane's per-branch files into the trunk's, so nothing accumulates.
    store::roll_up(&lane_path, &branch, &trunk)?;

    let changed = try_git(
        &["status", "--porcelain", "--", CONTEXT_DIR, "AGENTS.md"],
        Some(&lane_path),
    );
    if !changed.trim().is_empty() {
        try_git(&["add", CONTEXT_DIR, "AGENTS.md"], Some(&lane_path));
        git(
            &[
                "commit",
                "-q",
                "-m",
                &format!("memory: update context from lane {branch}"),
            ],
            Some(&lane_path),
        )?;
        writeln!(info, "committed memory update")?;
    }

    wt::fast_forward(&root, &trunk, &branch)?;
    writeln!(info, "fast-forwarded {trunk}")?;

    if !keep {
        std::env::set_current_dir(&root)?;
        let name = lane_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        wt::remove(&name, true)?;
        writeln!(info, "removed lane {branch}")?;
    }
    if cd {
        println!("{}", root.display());
    }
    Ok(0)
}

fn rm(name: &str, force: bool) -> Result<i32> {
    // The work is discarded, so its state and its record go with it.
    if let Ok(root) = wt::main_root() {
        store::discard_branch_files(&root, name);
    }
    if wt::remove(name, force)? {
        println!("removed lane {name}");
        return Ok(0);
    }
    let trunk = wt::trunk_name(&wt::main_root()?);
    eprintln!("removed lane {name}; kept branch {name}, it has commits {trunk} does not");
    eprintln!("  git worktree add <path> {name}   to get back to them");
    eprintln!("  lane rm {name} --force            to discard them");
    Ok(1)
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
