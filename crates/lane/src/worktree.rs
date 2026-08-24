//! Lane lifecycle: create, list, land, remove.

use crate::cow;
use crate::git::{git, git_ok, layout, try_git};
use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const TREES_DIRNAME: &str = "trees";
const TREES_PATH: &str = ".lane/trees";

/// Root of the primary worktree, even when called from inside a lane.
pub fn main_root() -> Result<PathBuf> {
    Ok(layout(&std::env::current_dir()?)?.main_root)
}

pub fn trunk_name(root: &Path) -> String {
    static TRUNKS: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let trunks = TRUNKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut trunks = trunks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(trunk) = trunks.get(root) {
        return trunk.clone();
    }

    let trunk = trunk_name_uncached(root);
    trunks.insert(root.to_path_buf(), trunk.clone());
    trunk
}

fn trunk_name_uncached(root: &Path) -> String {
    let origin = try_git(
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
        Some(root),
    );
    let named = origin.strip_prefix("origin/").into_iter();
    for candidate in named.chain(["main", "master", "trunk"]) {
        if git_ok(&["rev-parse", "--verify", "--quiet", candidate], Some(root)) {
            return candidate.into();
        }
    }
    try_git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(root))
}

fn new_base(root: &Path) -> String {
    let branch = try_git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(root));
    if !branch.is_empty() && branch != "HEAD" {
        branch
    } else {
        trunk_name(root)
    }
}

pub fn lane_base(root: &Path, branch: &str) -> String {
    let key = format!("lane.{branch}.base");
    let base = try_git(&["config", "--get", &key], Some(root));
    if !base.is_empty() && git_ok(&["rev-parse", "--verify", "--quiet", &base], Some(root)) {
        return base;
    }
    trunk_name(root)
}

fn record_base(root: &Path, branch: &str, base: &str) -> Result<()> {
    let key = format!("lane.{branch}.base");
    git(&["config", "--local", &key, base], Some(root))?;
    Ok(())
}

pub fn lanes_dir(root: &Path) -> PathBuf {
    root.join(crate::store::LANE_DIR).join(TREES_DIRNAME)
}

/// Tracked changes only: untracked files do not block a rebase.
pub fn is_dirty(path: &Path) -> bool {
    !try_git(
        &["status", "--porcelain", "--untracked-files=no"],
        Some(path),
    )
    .trim()
    .is_empty()
}

/// Entries git will not materialize: exactly what a fresh worktree is missing.
/// Already collapsed to directory roots, at any depth, from the user's own ignore rules.
fn ignored_entries(root: &Path) -> Vec<String> {
    try_git(&["status", "--porcelain", "-z", "--ignored"], Some(root))
        .split('\0')
        .filter_map(|e| e.strip_prefix("!! "))
        .map(|p| p.trim_end_matches('/').to_string())
        .filter(|p| !p.is_empty() && p != ".git" && p != TREES_PATH)
        .collect()
}

/// git 2.48+ supports relative paths. Older versions reject just that option, then use
/// absolute paths, which work in place but not after a move.
fn add_worktree(root: &Path, args: &[&str]) -> Result<()> {
    let mut relative_args = vec!["worktree", "add", "--relative-paths"];
    relative_args.extend(args);
    match git(&relative_args, Some(root)) {
        Ok(_) => Ok(()),
        Err(error) if rejects_relative_paths(&error) => {
            let mut absolute_args = vec!["worktree", "add"];
            absolute_args.extend(args);
            git(&absolute_args, Some(root)).map(|_| ())
        }
        Err(error) => Err(error),
    }
}

fn rejects_relative_paths(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("relative-paths")
        && (message.contains("unknown option") || message.contains("unrecognized option"))
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

fn prepare_lanes_dir(root: &Path) -> Result<()> {
    let dir = lanes_dir(root);
    std::fs::create_dir_all(&dir)?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(ignore, "*\n")?;
    }
    let exclude = git(&["rev-parse", "--git-path", "info/exclude"], Some(root))?;
    append_line(Path::new(&exclude), &format!("{TREES_PATH}/"))
}

fn excluded(root: &Path) -> HashSet<String> {
    try_git(&["config", "--get-all", "lane.exclude"], Some(root))
        .lines()
        .map(|p| p.trim_end_matches('/').to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

pub struct Lane {
    pub path: PathBuf,
    pub branch: String,
}

pub fn list_lanes(root: &Path) -> Vec<Lane> {
    let out = try_git(&["worktree", "list", "--porcelain"], Some(root));
    let mut lanes = Vec::new();
    let (mut path, mut branch) = (String::new(), String::new());

    let flush = |path: &mut String, branch: &mut String, lanes: &mut Vec<Lane>| {
        if !path.is_empty() && Path::new(path.as_str()) != root {
            lanes.push(Lane {
                path: PathBuf::from(path.as_str()),
                branch: if branch.is_empty() {
                    "detached".into()
                } else {
                    branch.clone()
                },
            });
        }
        path.clear();
        branch.clear();
    };

    for line in out.lines() {
        if line.trim().is_empty() {
            flush(&mut path, &mut branch, &mut lanes);
        } else if let Some(v) = line.strip_prefix("worktree ") {
            path = v.to_string();
        } else if let Some(v) = line.strip_prefix("branch ") {
            branch = v.trim_start_matches("refs/heads/").to_string();
        }
    }
    flush(&mut path, &mut branch, &mut lanes);
    lanes
}

pub struct Created {
    pub path: PathBuf,
    pub stats: cow::CloneStats,
    pub notes: Vec<String>,
}

#[derive(Debug, PartialEq)]
enum Materialization {
    Plain,
    Ignored,
    Dirty,
    /// Explicit --dirty without reflink: the caches are not worth a byte copy, the
    /// handful of edited files is.
    DirtyPlain,
}

fn materialization(dirty: bool, reflink: bool) -> Materialization {
    match (dirty, reflink) {
        (false, false) => Materialization::Plain,
        (true, false) => Materialization::DirtyPlain,
        (false, true) => Materialization::Ignored,
        (true, true) => Materialization::Dirty,
    }
}

/// Uncommitted work: tracked files that differ from HEAD, plus untracked non-ignored ones.
fn uncommitted(root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = try_git(&["diff", "--name-only", "-z", "HEAD"], Some(root))
        .split('\0')
        .map(str::to_string)
        .collect();
    paths.extend(
        try_git(
            &["ls-files", "--others", "--exclude-standard", "-z"],
            Some(root),
        )
        .split('\0')
        .map(str::to_string),
    );
    paths.retain(|p| !p.is_empty());
    paths
}

fn add_stats(total: &mut cow::CloneStats, next: cow::CloneStats) {
    total.cloned += next.cloned;
    total.copied += next.copied;
    total.links += next.links;
    total.bytes_shared += next.bytes_shared;
    total.bytes_copied += next.bytes_copied;
}

fn clone_entry(root: &Path, dest: &Path, entry: &str) -> Result<cow::CloneStats> {
    let source = root.join(entry);
    let target = dest.join(entry);
    if std::fs::symlink_metadata(&source)?.is_dir() {
        return Ok(cow::clone_tree_rooted(
            &source,
            &target,
            &|_, _| false,
            root,
            dest,
        )?);
    }
    let Some(name) = source.file_name().map(|name| name.to_string_lossy()) else {
        bail!("ignored entry has no file name: {entry}");
    };
    let Some(source_parent) = source.parent() else {
        bail!("ignored entry has no parent: {entry}");
    };
    let Some(target_parent) = target.parent() else {
        bail!("ignored entry has no destination parent: {entry}");
    };
    Ok(cow::clone_tree_rooted(
        source_parent,
        target_parent,
        &|rel, _| rel != name,
        root,
        dest,
    )?)
}

fn branch_args<'a>(adopt: bool, name: &'a str, dest: &'a str, base: &'a str) -> Vec<&'a str> {
    if adopt {
        vec![dest, name]
    } else {
        vec!["-b", name, dest, base]
    }
}

/// By default git checks out tracked files and ignored entries are cloned by reference.
pub fn create(name: &str, base: Option<&str>, dirty: bool) -> Result<Created> {
    let root = main_root()?;
    // An existing branch is adopted, not recreated: a fetched pull request needs a lane
    // to be reviewed or landed in, and a lane pruned early needs one to come back to.
    let adopt = git_ok(
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
        Some(&root),
    );
    if adopt && base.is_some() {
        bail!("branch {name} already exists; --base applies only to a new branch");
    }
    let base = base.map(str::to_string).unwrap_or_else(|| new_base(&root));
    let dest = lanes_dir(&root).join(name);
    if dest.exists() {
        bail!("lane {name} already exists at {}", dest.display());
    }
    prepare_lanes_dir(&root)?;

    let (supported, detail) = cow::probe(&root);
    let mut notes = vec![format!(
        "reflink: {} ({detail})",
        if supported { "yes" } else { "no" }
    )];
    if !supported {
        notes.push("no reflink here; leaving a plain worktree".into());
    }
    if !dirty {
        let carried = try_git(
            &["status", "--porcelain", "--untracked-files=no"],
            Some(&root),
        )
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
        if carried > 0 {
            notes.push(format!(
                "warning: {carried} uncommitted change(s) were not carried\n    lane rm {name} && lane new {name} --dirty   to start over with them"
            ));
        }
    }
    let dest_str = dest.to_string_lossy().to_string();

    let stats = match materialization(dirty, supported) {
        Materialization::Dirty => {
            let mut args = vec!["--no-checkout"];
            args.extend(branch_args(adopt, name, &dest_str, &base));
            add_worktree(&root, &args)?;
            let skip = |rel: &str, _is_dir: bool| {
                rel == ".git"
                    || rel.starts_with(".git/")
                    || rel == TREES_PATH
                    || rel.starts_with(".lane/trees/")
            };
            let stats = cow::clone_tree(&root, &dest, &skip)?;
            // Repopulate the index from the checked-out tree without rewriting a single
            // file. HEAD, not base: an adopted branch is already at its own tip.
            git(&["reset", "--mixed", "--quiet", "HEAD"], Some(&dest))?;
            try_git(&["update-index", "--refresh"], Some(&dest));
            let carried = try_git(&["status", "--porcelain"], Some(&dest))
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count();
            if carried > 0 {
                notes.push(format!(
                    "carried {carried} uncommitted change(s) from the parent tree"
                ));
            }
            stats
        }
        mode => {
            let ignored = if mode == Materialization::Ignored {
                ignored_entries(&root)
            } else {
                Vec::new()
            };
            let carry = if mode == Materialization::DirtyPlain {
                uncommitted(&root)
            } else {
                Vec::new()
            };
            let excluded = excluded(&root);
            let mut args = Vec::new();
            args.extend(branch_args(adopt, name, &dest_str, &base));
            add_worktree(&root, &args)?;
            let mut stats = cow::CloneStats::default();
            for entry in ignored {
                if excluded.contains(&entry) {
                    continue;
                }
                let next = clone_entry(&root, &dest, &entry)
                    .with_context(|| format!("cloning ignored entry {entry}"))?;
                add_stats(&mut stats, next);
            }
            for path in &carry {
                let (from, to) = (root.join(path), dest.join(path));
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&from, &to)
                    .with_context(|| format!("carrying uncommitted {path}"))?;
                stats.copied += 1;
            }
            if !carry.is_empty() {
                notes.push(format!(
                    "carried {} uncommitted change(s) by copy; no reflink here",
                    carry.len()
                ));
            }
            stats
        }
    };

    crate::store::stamp_lane_id(&dest)?;
    if !adopt {
        record_base(&root, name, &base)?;
    }

    Ok(Created {
        path: dest,
        stats,
        notes,
    })
}

/// True while git still has a worktree registered at this path, prunable ones included.
fn registered(root: &Path, dest: &Path) -> bool {
    let target = dest.canonicalize().ok();
    list_lanes(root).iter().any(|lane| {
        lane.path == dest || (target.is_some() && lane.path.canonicalize().ok() == target)
    })
}

/// What removing this lane destroys for good. Empty means nothing is at stake.
///
/// Every caller of `remove` asks this first. `git branch -d` is the weaker question: it
/// refuses every squash and rebase merge, and knows nothing about the memory a lane holds.
pub fn losses(root: &Path, path: &Path, branch: &str, trunk: &str) -> Vec<String> {
    let mut out = Vec::new();
    if path.is_dir() {
        // Untracked counts here where it does not for a rebase: removal deletes the file.
        let changed = try_git(&["status", "--porcelain"], Some(path))
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        if changed > 0 {
            out.push(format!("{changed} uncommitted change(s)"));
        }
        let pending = crate::store::pending_count(path);
        if pending > 0 {
            out.push(format!("{pending} pending note(s)"));
        }
    }
    let refname = format!("refs/heads/{branch}");
    if git_ok(&["rev-parse", "--verify", "--quiet", &refname], Some(root))
        && !contained_in(root, trunk, branch)
    {
        // No count: a squash merge leaves commits whose patches landed inside one of
        // trunk's, so `rev-list` would name a number larger than what is really at risk.
        out.push(format!("commits {trunk} does not have"));
    }
    out
}

/// Remove a lane's worktree and its branch, and with them everything the lane still held.
///
/// Unconditional by design: `losses` is the guard, and every caller runs it first.
pub fn remove(name: &str) -> Result<()> {
    let root = main_root()?;
    let dest = lanes_dir(&root).join(name);

    // Deleting the directory the caller is standing in leaves their shell in a path that no
    // longer exists, which is the failure plan 006 exists to prevent. `done` chdirs to the
    // root before it gets here; `rm` and `prune` have no reason to, so refuse instead.
    let inside = std::env::current_dir()
        .ok()
        .and_then(|cwd| cwd.canonicalize().ok())
        .zip(dest.canonicalize().ok())
        .is_some_and(|(cwd, dest)| cwd.starts_with(dest));
    if inside {
        bail!("cannot remove lane {name} from inside it; cd out first");
    }

    let refname = format!("refs/heads/{name}");
    let branch = git_ok(&["rev-parse", "--verify", "--quiet", &refname], Some(&root));
    let worktree = registered(&root, &dest);
    if !branch && !worktree {
        bail!("no lane {name}");
    }

    // A hand-deleted lane leaves a branch and no worktree, and git calls removing an
    // absent one fatal. Skipping is what lets `rm` clean up after that.
    if worktree {
        let dest_str = dest.to_string_lossy().to_string();
        git(&["worktree", "remove", "--force", &dest_str], Some(&root))?;
    }
    if branch {
        git(&["branch", "-D", name], Some(&root))?;
    }
    if dest.exists() {
        let _ = std::fs::remove_dir_all(&dest);
    }
    Ok(())
}

fn tracked_changes(status: &str) -> Vec<String> {
    let mut entries = status.split('\0');
    let mut paths = Vec::new();
    while let Some(entry) = entries.next() {
        if entry.starts_with("## ") {
            continue;
        }
        let Some(path) = entry.get(3..) else { continue };
        paths.push(path.to_string());
        if entry.as_bytes()[..2]
            .iter()
            .any(|s| matches!(s, b'R' | b'C'))
            && let Some(source) = entries.next()
        {
            paths.push(source.to_string());
        }
    }
    paths
}

/// Files a fast-forward would overwrite in the main worktree. Empty means `done` can land.
pub fn blocking_changes(root: &Path, trunk: &str, branch: &str) -> Vec<String> {
    // Only the merge path touches a working tree; update-ref cannot conflict.
    if try_git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(root)) != trunk {
        return Vec::new();
    }
    let incoming: HashSet<String> = try_git(
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            &format!("{trunk}..{branch}"),
        ],
        Some(root),
    )
    .split('\0')
    .filter(|p| !p.is_empty())
    .map(str::to_string)
    .collect();
    // The branch header protects the first status column from try_git's trim.
    tracked_changes(&try_git(
        &[
            "status",
            "--porcelain",
            "-z",
            "--branch",
            "--untracked-files=no",
        ],
        Some(root),
    ))
    .into_iter()
    .filter(|p| incoming.contains(p))
    .collect()
}

/// Is everything on `branch` already represented in `trunk`?
///
/// Three answers for GitHub's three merge buttons. A merge commit leaves the branch an
/// ancestor. Anything else rewrites the SHAs, so `git branch -d` refuses even when the
/// trees are identical; comparing the branch's cumulative diff against trunk by patch-id
/// is what sees through a rebase or a squash.
pub fn contained_in(root: &Path, trunk: &str, branch: &str) -> bool {
    if git_ok(&["merge-base", "--is-ancestor", branch, trunk], Some(root)) {
        return true;
    }
    // A rebase merge replays the commits one for one, so their patches land separately.
    // The collapsed probe below is the squash answer and matches none of them.
    let replayed = try_git(&["cherry", trunk, branch], Some(root));
    if !replayed.is_empty() && replayed.lines().all(|line| line.starts_with('-')) {
        return true;
    }
    let Ok(base) = git(&["merge-base", trunk, branch], Some(root)) else {
        return false;
    };
    let Ok(tree) = git(&["rev-parse", &format!("{branch}^{{tree}}")], Some(root)) else {
        return false;
    };
    // An empty probe has no patch-id to match, so `cherry` would call it unmerged.
    if git(&["rev-parse", &format!("{base}^{{tree}}")], Some(root)).is_ok_and(|b| b == tree) {
        return true;
    }
    let Ok(probe) = git(
        &[
            "commit-tree",
            &tree,
            "-p",
            &base,
            "-m",
            "lane: containment probe",
        ],
        Some(root),
    ) else {
        return false;
    };
    let cherry = try_git(&["cherry", trunk, &probe], Some(root));
    !cherry.is_empty() && cherry.lines().all(|line| line.starts_with('-'))
}

/// Advance trunk to branch: merge when trunk is checked out, update-ref when it is not.
pub fn fast_forward(root: &Path, trunk: &str, branch: &str) -> Result<()> {
    let head = git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(root))?;
    let target = git(&["rev-parse", branch], Some(root))?;
    if head == trunk {
        git(&["merge", "--ff-only", branch], Some(root))?;
        return Ok(());
    }
    let base = git(&["merge-base", trunk, branch], Some(root))?;
    if base != git(&["rev-parse", trunk], Some(root))? {
        bail!("trunk {trunk} has diverged from {branch}; rebase first");
    }
    git(
        &["update-ref", &format!("refs/heads/{trunk}"), &target],
        Some(root),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_live_inside_the_repository() {
        let root = Path::new("/repo");

        assert_eq!(lanes_dir(root), root.join(".lane/trees"));
    }

    #[test]
    fn ignored_entries_excludes_the_lanes_directory() -> Result<()> {
        let root = tempfile::tempdir()?;
        let r = root.path();
        let run = |args: &[&str]| {
            git(args, Some(r)).ok();
        };
        run(&["init", "-qb", "main"]);
        run(&["config", "user.email", "t@t.t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(r.join(".gitignore"), ".lane/trees/\ncache/\n")?;
        run(&["add", ".gitignore"]);
        run(&["commit", "-qm", "base"]);
        std::fs::create_dir_all(r.join(".lane/trees/other"))?;
        std::fs::create_dir_all(r.join("cache"))?;
        std::fs::write(r.join("cache/blob"), "cache")?;

        let entries = ignored_entries(r);

        assert!(entries.contains(&"cache".to_string()));
        assert!(!entries.contains(&TREES_PATH.to_string()));
        Ok(())
    }

    #[test]
    fn no_reflink_skips_the_caches_but_still_honours_dirty() {
        assert_eq!(materialization(false, false), Materialization::Plain);
        assert_eq!(materialization(true, false), Materialization::DirtyPlain);
    }

    #[test]
    fn reflink_selects_the_requested_mode() {
        assert_eq!(materialization(false, true), Materialization::Ignored);
        assert_eq!(materialization(true, true), Materialization::Dirty);
    }

    #[test]
    fn uncommitted_finds_tracked_edits_and_untracked_files() -> Result<()> {
        let root = tempfile::tempdir()?;
        let r = root.path();
        let run = |args: &[&str]| {
            git(args, Some(r)).ok();
        };
        run(&["init", "-qb", "main"]);
        run(&["config", "user.email", "t@t.t"]);
        run(&["config", "user.name", "t"]);
        std::fs::create_dir_all(r.join("src"))?;
        std::fs::write(r.join("src/a.rs"), "one\n")?;
        std::fs::write(r.join(".gitignore"), "ignored.txt\n")?;
        run(&["add", "-A"]);
        run(&["commit", "-qm", "base"]);

        std::fs::write(r.join("src/a.rs"), "two\n")?; // tracked edit
        std::fs::write(r.join("scratch.txt"), "x")?; // untracked, not ignored
        std::fs::write(r.join("ignored.txt"), "x")?; // ignored, not uncommitted work

        let mut found = uncommitted(r);
        found.sort();
        assert_eq!(
            found,
            vec!["scratch.txt".to_string(), "src/a.rs".to_string()]
        );
        Ok(())
    }

    #[test]
    fn an_ignored_file_is_cloned() -> Result<()> {
        let root = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        std::fs::write(root.path().join(".env"), "SECRET=1")?;

        clone_entry(root.path(), dest.path(), ".env")?;

        assert_eq!(
            std::fs::read_to_string(dest.path().join(".env"))?,
            "SECRET=1"
        );
        Ok(())
    }

    #[test]
    fn tracked_changes_keeps_spaces() {
        assert_eq!(
            tracked_changes("## main\0 M src/auth flow.rs\0"),
            vec!["src/auth flow.rs"]
        );
    }

    #[test]
    fn tracked_changes_includes_both_sides_of_a_rename() {
        assert_eq!(
            tracked_changes("## main\0R  src/new.rs\0src/old.rs\0"),
            vec!["src/new.rs", "src/old.rs"]
        );
    }

    fn repository(branch: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| git(args, Some(root.path())).unwrap();
        run(&["init", "-qb", branch]);
        run(&["config", "user.email", "t@t.t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.path().join("file"), "one\n").unwrap();
        run(&["add", "file"]);
        run(&["commit", "-qm", "base"]);
        root
    }

    #[test]
    fn new_base_prefers_the_main_worktree_branch() {
        let root = repository("develop");
        git(&["branch", "main"], Some(root.path())).unwrap();

        assert_eq!(new_base(root.path()), "develop");
    }

    #[test]
    fn trunk_name_resolves_origin_head() {
        let root = repository("develop");
        git(
            &["update-ref", "refs/remotes/origin/develop", "HEAD"],
            Some(root.path()),
        )
        .unwrap();
        git(
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/develop",
            ],
            Some(root.path()),
        )
        .unwrap();

        assert_eq!(trunk_name(root.path()), "develop");
    }

    #[test]
    fn trunk_name_probes_when_origin_head_is_absent() {
        let root = repository("develop");
        git(&["branch", "main"], Some(root.path())).unwrap();
        git(&["checkout", "--detach"], Some(root.path())).unwrap();

        assert_eq!(trunk_name(root.path()), "main");
    }

    #[test]
    fn trunk_name_ignores_a_different_checked_out_branch() {
        let root = repository("main");
        git(&["checkout", "-qb", "develop"], Some(root.path())).unwrap();

        assert_eq!(trunk_name(root.path()), "main");
    }
}
