//! Lane lifecycle: create, list, land, remove.

use crate::cow;
use crate::git::{git, git_ok, try_git};
use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const LANES_DIRNAME: &str = ".lanes";

/// Root of the primary worktree, even when called from inside a lane.
pub fn main_root() -> Result<PathBuf> {
    let common = git(
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        None,
    )?;
    Ok(PathBuf::from(common)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default())
}

pub fn trunk_name(root: &Path) -> String {
    for candidate in ["main", "master", "trunk"] {
        if git_ok(&["rev-parse", "--verify", "--quiet", candidate], Some(root)) {
            return candidate.into();
        }
    }
    try_git(&["rev-parse", "--abbrev-ref", "HEAD"], Some(root))
}

pub fn lanes_dir(root: &Path) -> PathBuf {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    root.parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
        .join(format!("{LANES_DIRNAME}-{name}"))
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
        .filter(|p| !p.is_empty() && p != ".git")
        .collect()
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
        return Ok(cow::clone_tree(&source, &target, &|_, _| false)?);
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
    Ok(cow::clone_tree(source_parent, target_parent, &|rel, _| {
        rel != name
    })?)
}

/// By default git checks out tracked files and ignored entries are cloned by reference.
pub fn create(name: &str, base: Option<&str>, dirty: bool) -> Result<Created> {
    let root = main_root()?;
    let base = base
        .map(str::to_string)
        .unwrap_or_else(|| trunk_name(&root));
    let dest = lanes_dir(&root).join(name);
    if dest.exists() {
        bail!("lane {name} already exists at {}", dest.display());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

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
            git(
                &[
                    "worktree",
                    "add",
                    "--no-checkout",
                    "-b",
                    name,
                    &dest_str,
                    &base,
                ],
                Some(&root),
            )?;
            let skip = |rel: &str, _is_dir: bool| rel == ".git" || rel.starts_with(".git/");
            let stats = cow::clone_tree(&root, &dest, &skip)?;
            // Repopulate the index from the base tree without rewriting a single file.
            git(&["reset", "--mixed", "--quiet", &base], Some(&dest))?;
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
            git(
                &["worktree", "add", "-b", name, &dest_str, &base],
                Some(&root),
            )?;
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

    Ok(Created {
        path: dest,
        stats,
        notes,
    })
}

/// Remove a lane's worktree, and its branch when that is safe. True if the branch went too.
///
/// `-d` not `-D`: an unlanded lane holds the only reference to its commits, so discarding
/// them is asked for, never a side effect. `done` passes force after fast-forwarding trunk.
pub fn remove(name: &str, force: bool) -> Result<bool> {
    let root = main_root()?;
    let dest = lanes_dir(&root).join(name);
    let dest_str = dest.to_string_lossy().to_string();

    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&dest_str);
    git(&args, Some(&root))?;

    let refname = format!("refs/heads/{name}");
    let mut deleted = true;
    if git_ok(&["rev-parse", "--verify", "--quiet", &refname], Some(&root)) {
        deleted = git_ok(
            &["branch", if force { "-D" } else { "-d" }, name],
            Some(&root),
        );
    }
    if dest.exists() {
        let _ = std::fs::remove_dir_all(&dest);
    }
    Ok(deleted)
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
}
