//! Lane lifecycle: create, list, land, remove.

use crate::cow;
use crate::git::{git, git_ok, try_git};
use anyhow::{Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const LANES_DIRNAME: &str = ".lanes";

pub const WARM_DEFAULT: [&str; 10] = [
    "node_modules",
    "target",
    ".venv",
    "vendor",
    "dist",
    ".next",
    ".turbo",
    ".gradle",
    "build",
    ".cargo",
];

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

fn tracked_set(root: &Path) -> HashSet<String> {
    try_git(&["ls-files", "-z"], Some(root))
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
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

/// Two strategies: by default git checks out tracked files and we clone only the warm
/// dirs; `--fork` clones the whole tree by reference and rebuilds the index in place.
pub fn create(
    name: &str,
    base: Option<&str>,
    fork: bool,
    warm: Option<Vec<String>>,
) -> Result<Created> {
    let root = main_root()?;
    let base = base
        .map(str::to_string)
        .unwrap_or_else(|| trunk_name(&root));
    let warm: Vec<String> =
        warm.unwrap_or_else(|| WARM_DEFAULT.iter().map(|s| s.to_string()).collect());
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
    let dest_str = dest.to_string_lossy().to_string();

    let stats = if fork {
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
    } else {
        git(
            &["worktree", "add", "-b", name, &dest_str, &base],
            Some(&root),
        )?;
        let tracked = tracked_set(&root);
        let warm_set: HashSet<&str> = warm.iter().map(String::as_str).collect();
        let skip = |rel: &str, is_dir: bool| {
            let top = rel.split(std::path::MAIN_SEPARATOR).next().unwrap_or(rel);
            if top == ".git" {
                return true;
            }
            if is_dir {
                // Descend only into warm entries, and only at the top level.
                return !rel.contains(std::path::MAIN_SEPARATOR) && !warm_set.contains(top);
            }
            tracked.contains(rel) || !warm_set.contains(top)
        };
        cow::clone_tree(&root, &dest, &skip)?
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
