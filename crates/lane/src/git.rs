//! git via subprocess: rebase and worktree stay git's job, so we never reimplement them.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

fn spawn(args: &[&str], cwd: Option<&Path>) -> Result<std::process::Output> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    Ok(cmd.output()?)
}

/// Run git, failing with its stderr.
pub fn git(args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let out = spawn(args, cwd)?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run git, treating failure as empty output.
pub fn try_git(args: &[&str], cwd: Option<&Path>) -> String {
    match spawn(args, cwd) {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Run git for its exit status alone.
pub fn git_ok(args: &[&str], cwd: Option<&Path>) -> bool {
    matches!(spawn(args, cwd), Ok(out) if out.status.success())
}

/// Filesystem layout for the worktree containing a directory.
///
/// `git_dir` is per-worktree; `common_dir` is shared by every linked worktree.
/// Keep those separate: lane's pending queue and identity intentionally live in
/// the former, while the primary worktree is the parent of the latter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoLayout {
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub repo_root: PathBuf,
    pub main_root: PathBuf,
}

/// Discover a repository layout without spawning git.
pub fn layout(start: &Path) -> Result<RepoLayout> {
    resolve_layout(
        start,
        std::env::var_os("GIT_DIR").map(PathBuf::from),
        std::env::var_os("GIT_COMMON_DIR").map(PathBuf::from),
    )
}

fn resolve_layout(
    start: &Path,
    git_dir_override: Option<PathBuf>,
    common_dir_override: Option<PathBuf>,
) -> Result<RepoLayout> {
    let start = std::fs::canonicalize(start)
        .with_context(|| format!("{} is not a git repository", start.display()))?;
    let repo_root = find_repo_root(&start)?;
    let discovered_git_dir = git_dir_at(&repo_root)?;
    let git_dir = canonical_path(git_dir_override.unwrap_or(discovered_git_dir), &repo_root)?;

    let common_dir = match common_dir_override {
        Some(path) => canonical_path(path, &repo_root)?,
        None => match std::fs::read_to_string(git_dir.join("commondir")) {
            Ok(path) => canonical_path(PathBuf::from(path.trim()), &git_dir)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => git_dir.clone(),
            Err(error) => return Err(error).context("read git commondir"),
        },
    };
    let main_root = common_dir
        .parent()
        .context("git common directory has no parent")?
        .to_path_buf();

    Ok(RepoLayout {
        git_dir,
        common_dir,
        repo_root,
        main_root,
    })
}

fn find_repo_root(start: &Path) -> Result<PathBuf> {
    for dir in start.ancestors() {
        if dir.join(".git").exists() {
            return Ok(dir.to_path_buf());
        }
    }
    bail!("{} is not a git repository", start.display())
}

fn git_dir_at(repo_root: &Path) -> Result<PathBuf> {
    let dot_git = repo_root.join(".git");
    if dot_git.is_dir() {
        return Ok(dot_git);
    }
    let contents = std::fs::read_to_string(&dot_git).context("read .git file")?;
    let Some(path) = contents.trim().strip_prefix("gitdir:") else {
        bail!("{} has an invalid .git file", repo_root.display());
    };
    Ok(PathBuf::from(path.trim()))
}

fn canonical_path(path: PathBuf, base: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    std::fs::canonicalize(&path).with_context(|| format!("canonicalize {}", path.display()))
}

pub fn repo_root() -> Result<PathBuf> {
    Ok(layout(&std::env::current_dir()?)?.repo_root)
}

pub fn current_branch() -> String {
    let b = try_git(&["rev-parse", "--abbrev-ref", "HEAD"], None);
    if b.is_empty() { "unknown".into() } else { b }
}

/// Paths this branch changed relative to base; biases note retention.
pub fn touched_paths(base: &str) -> std::collections::HashSet<String> {
    try_git(&["diff", "--name-only", &format!("{base}...HEAD")], None)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Renames reachable from HEAD, oldest first so a chain of them applies in order.
///
/// A lane knows its base and can diff against it. On trunk there is no base — the rename
/// is already in history — so fall back to a bounded walk of recent commits.
pub fn renames(base: &str) -> Vec<(String, String)> {
    let (out, newest_first) = if base.is_empty() {
        (
            try_git(
                &[
                    "log",
                    "--diff-filter=R",
                    "--name-status",
                    "--find-renames",
                    "--format=",
                    "--max-count=200",
                ],
                None,
            ),
            true,
        )
    } else {
        (
            try_git(
                &[
                    "diff",
                    "--name-status",
                    "--find-renames",
                    &format!("{base}...HEAD"),
                ],
                None,
            ),
            false,
        )
    };

    let mut pairs = Vec::new();
    for line in out.lines() {
        let mut parts = line.split('\t');
        let Some(status) = parts.next() else { continue };
        if !status.starts_with('R') {
            continue;
        }
        if let (Some(old), Some(new)) = (parts.next(), parts.next()) {
            pairs.push((old.to_string(), new.to_string()));
        }
    }
    if newest_first {
        pairs.reverse();
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn primary() -> (TempDir, PathBuf) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        (temp, root)
    }

    fn linked_worktree(root: &Path) -> PathBuf {
        let lane = root.join(".lane/trees/alpha");
        let git_dir = root.join(".git/worktrees/alpha");
        std::fs::create_dir_all(&lane).unwrap();
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(lane.join(".git"), "gitdir: ../../../.git/worktrees/alpha\n").unwrap();
        std::fs::write(git_dir.join("commondir"), "../..\n").unwrap();
        lane
    }

    #[test]
    fn discovers_primary_root_from_a_subdirectory() {
        let (_temp, root) = primary();
        let nested = root.join("src/nested");
        std::fs::create_dir_all(&nested).unwrap();

        let layout = resolve_layout(&nested, None, None).unwrap();
        let git_dir = std::fs::canonicalize(root.join(".git")).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        assert_eq!(layout.git_dir, git_dir);
        assert_eq!(layout.common_dir, layout.git_dir);
        assert_eq!(layout.repo_root, root);
        assert_eq!(layout.main_root, layout.repo_root);
    }

    #[test]
    fn linked_worktree_uses_per_worktree_git_dir_and_primary_main_root() {
        let (_temp, root) = primary();
        let lane = linked_worktree(&root);

        let layout = resolve_layout(&lane.join("src"), None, None).unwrap_err();
        assert!(layout.to_string().contains("not a git repository"));
        std::fs::create_dir(lane.join("src")).unwrap();
        let layout = resolve_layout(&lane.join("src"), None, None).unwrap();

        assert_eq!(
            layout.git_dir,
            std::fs::canonicalize(root.join(".git/worktrees/alpha")).unwrap()
        );
        assert_eq!(
            layout.common_dir,
            std::fs::canonicalize(root.join(".git")).unwrap()
        );
        assert_eq!(layout.repo_root, std::fs::canonicalize(&lane).unwrap());
        // A lane's own root and the primary root must never be conflated.
        assert_ne!(layout.repo_root, layout.main_root);
        assert_eq!(layout.main_root, std::fs::canonicalize(root).unwrap());
    }

    #[test]
    fn absolute_gitdir_and_environment_overrides_win() {
        let (_temp, root) = primary();
        let lane = root.join("lane");
        let alternate_git = root.join("alternate/git");
        let alternate_common = root.join("alternate/common");
        std::fs::create_dir_all(&lane).unwrap();
        std::fs::create_dir_all(&alternate_git).unwrap();
        std::fs::create_dir_all(&alternate_common).unwrap();
        std::fs::write(
            lane.join(".git"),
            format!("gitdir: {}\n", alternate_git.display()),
        )
        .unwrap();

        let layout = resolve_layout(
            &lane,
            Some(alternate_git.clone()),
            Some(alternate_common.clone()),
        )
        .unwrap();
        assert_eq!(
            layout.git_dir,
            std::fs::canonicalize(alternate_git).unwrap()
        );
        assert_eq!(
            layout.common_dir,
            std::fs::canonicalize(&alternate_common).unwrap()
        );
        assert_eq!(
            layout.main_root,
            std::fs::canonicalize(alternate_common.parent().unwrap()).unwrap()
        );
    }

    #[test]
    fn rejects_non_repositories() {
        let temp = TempDir::new().unwrap();
        let error = resolve_layout(temp.path(), None, None).unwrap_err();
        assert!(error.to_string().contains("not a git repository"));
    }

    #[cfg(unix)]
    #[test]
    fn canonicalizes_symlinked_starting_paths() {
        let (temp, root) = primary();
        let link = temp.path().join("linked-repo");
        std::os::unix::fs::symlink(&root, &link).unwrap();

        let layout = resolve_layout(&link, None, None).unwrap();
        assert_eq!(layout.repo_root, std::fs::canonicalize(root).unwrap());
    }
}
