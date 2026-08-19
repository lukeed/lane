//! git via subprocess: rebase and worktree stay git's job, so we never reimplement them.

use anyhow::{Result, bail};
use std::path::Path;
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

pub fn repo_root() -> Result<std::path::PathBuf> {
    Ok(git(&["rev-parse", "--show-toplevel"], None)?.into())
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
