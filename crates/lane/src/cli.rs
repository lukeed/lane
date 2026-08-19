//! Command surface. Every command returns an exit code; failures bubble as errors.

use crate::audit;
use crate::git::{self, git, try_git};
use crate::review;
use crate::store::{self, BODY, CONTEXT_DIR, FRESH, MISSING, PENDING, READS, SIG};
use crate::util::now_iso;
use crate::worktree as wt;
use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use std::io::{IsTerminal, Write};
use std::path::Path;

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
        /// clone the entire tree by reference, dirty state included
        #[arg(long)]
        fork: bool,
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
        allow_dirty: bool,
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

fn bold(text: &str) -> String {
    if std::io::stdout().is_terminal() {
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
            fork,
            cd,
        } => new(&name, base.as_deref(), fork, cd),
        Command::Ls => ls(),
        Command::Path { name } => path(&name),
        Command::Note { text, path, anchor } => note(&text, &path, &anchor),
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
            allow_dirty,
            budget,
            review,
        } => done(trunk.as_deref(), keep, allow_dirty, &budget, &review),
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
- Before editing a file, read `.context/<path>/` if it exists, or run `lane why <path>`.\n\
- Record non-obvious findings with `lane note -a <anchor> \"...\"`.\n\
- Do not edit `.context/` by hand; `lane done` manages it.\n";

fn init() -> Result<i32> {
    let root = wt::main_root()?;
    let context = root.join(CONTEXT_DIR);
    std::fs::create_dir_all(&context)?;
    std::fs::write(context.join(".gitkeep"), "")?;

    let attrs = root.join(".gitattributes");
    append_line(&attrs, &format!("{CONTEXT_DIR}/**/*.md merge=union"))?;
    append_line(&attrs, &format!("{CONTEXT_DIR}/{READS} merge=union"))?;

    let agents = root.join("AGENTS.md");
    if !agents.exists() {
        std::fs::write(&agents, format!("# AGENTS\n{PROTOCOL}"))?;
    } else if !std::fs::read_to_string(&agents)?.contains("## Context memory") {
        let mut file = std::fs::OpenOptions::new().append(true).open(&agents)?;
        write!(file, "{PROTOCOL}")?;
    }

    let ignore = root.join(".gitignore");
    append_line(&ignore, PENDING)?;
    append_line(&ignore, ".lanes-*")?;

    let (ok, detail) = crate::cow::probe(&root);
    println!("initialized .context/, union merge rules, AGENTS.md protocol");
    println!(
        "reflink on this filesystem: {} ({detail})",
        if ok { "yes" } else { "no" }
    );
    if !ok {
        println!("  lanes will still work; warm dirs get copied instead of shared");
    }
    Ok(0)
}

fn new(name: &str, base: Option<&str>, fork: bool, cd: bool) -> Result<i32> {
    let created = wt::create(name, base, fork, None)?;
    for note in &created.notes {
        println!("  {note}");
    }
    println!("  {}", created.stats);
    println!("{}", bold(&created.path.to_string_lossy()));
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
            .entry((note.meta.path.clone(), note.meta.anchor.clone()))
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
                    "id": n.meta.id, "path": n.meta.path,
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
                "id": n.meta.id, "path": n.meta.path,
                "anchor": n.meta.anchor, "status": n.meta.status,
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
    allow_dirty: bool,
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

    if wt::is_dirty(&lane_path) && !allow_dirty {
        eprintln!("error: lane is dirty; commit or stash first");
        return Ok(1);
    }

    git(&["rebase", &trunk], Some(&lane_path))?;
    println!("rebased onto {trunk}");

    let reviewer = review::build(review.review.as_deref(), review.review_cmd.as_deref());
    let out = audit::run(
        &lane_path,
        &options(&trunk, budget, review),
        reviewer.as_ref(),
    )?;
    audit::report(&out, &mut std::io::stdout())?;

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
        println!("committed memory update");
    }

    wt::fast_forward(&root, &trunk, &branch)?;
    println!("fast-forwarded {trunk}");

    if !keep {
        std::env::set_current_dir(&root)?;
        let name = lane_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        wt::remove(&name, true)?;
        println!("removed lane {branch}");
    }
    Ok(0)
}

fn rm(name: &str, force: bool) -> Result<i32> {
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
    new)  shift; local p; p=$(command lane new --cd "$@" | tail -1) && cd "$p" ;;
    cd)   shift; local p; p=$(command lane path "$1") && cd "$p" ;;
    done) command lane done "${{@:2}}" && cd "$(git rev-parse --show-toplevel)" ;;
    *)    command lane "$@" ;;
  esac
}}"#
    );
    Ok(0)
}
