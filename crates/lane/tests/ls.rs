use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn command(root: &Path, program: &str, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command.args(args).current_dir(root);
    for (key, _) in std::env::vars_os() {
        if key.as_encoded_bytes().starts_with(b"GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

fn run(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn git(root: &Path, args: &[&str]) -> String {
    run(&mut command(root, "git", args))
}

fn lane(root: &Path, args: &[&str]) -> String {
    run(&mut command(root, env!("CARGO_BIN_EXE_lane"), args))
}

fn repository() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    git(root, &["init", "-qb", "main"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    lane(root, &["init"]);
    std::fs::write(root.join("file"), "base\n").unwrap();
    std::fs::write(root.join("# branch.ab +0 -0"), "header\n").unwrap();
    std::fs::create_dir_all(root.join(".lane/memory")).unwrap();
    std::fs::write(root.join(".lane/memory/existing.md"), "old\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "base"]);
    temp
}

fn worktree(root: &Path, name: &str) -> PathBuf {
    let path = root.join(".lane/trees").join(name);
    git(
        root,
        &["worktree", "add", "-qb", name, path.to_str().unwrap()],
    );
    path
}

fn rows(root: &Path) -> Vec<Value> {
    serde_json::from_str(&lane(root, &["ls", "--json"])).unwrap()
}

fn row<'a>(rows: &'a [Value], name: &str) -> &'a Value {
    rows.iter().find(|row| row["name"] == name).unwrap()
}

#[test]
fn listing_preserves_status_notes_and_worktree_order() {
    let temp = repository();
    let root = temp.path();
    let open = worktree(root, "open");
    let pushed = worktree(root, "pushed");
    let ahead = worktree(root, "ahead");
    let detached = worktree(root, "detached");
    let missing = worktree(root, "missing");
    let renamed = worktree(root, "feat/renamed");
    for path in [&pushed, &ahead] {
        git(path, &["branch", "--set-upstream-to=main"]);
    }
    std::fs::write(ahead.join("file"), "ahead\n").unwrap();
    git(&ahead, &["commit", "-qam", "ahead"]);
    git(&detached, &["checkout", "--detach"]);
    std::fs::remove_dir_all(missing).unwrap();
    git(&renamed, &["checkout", "-qb", "different-branch"]);
    git(&open, &["mv", "# branch.ab +0 -0", "moved\nname"]);
    std::fs::write(open.join(".lane/memory/existing.md"), "revised\n").unwrap();
    std::fs::write(open.join(".lane/memory/new.md"), "new\n").unwrap();
    std::fs::write(pushed.join("untracked"), "ignored by dirty status\n").unwrap();
    let listing = rows(root);
    assert_eq!(listing.len(), 6);
    assert_eq!(row(&listing, "open")["state"], "open");
    assert_eq!(row(&listing, "open")["dirty"], true);
    assert_eq!(row(&listing, "open")["pending_notes"], 2);
    assert_eq!(row(&listing, "pushed")["state"], "pushed");
    assert_eq!(row(&listing, "pushed")["dirty"], false);
    assert_eq!(row(&listing, "ahead")["state"], "open");
    assert_eq!(row(&listing, "detached")["branch"], "detached");
    assert_eq!(row(&listing, "detached")["state"], "open");
    assert_eq!(row(&listing, "missing")["state"], "open");
    assert_eq!(row(&listing, "missing")["dirty"], false);
    assert_eq!(row(&listing, "feat/renamed")["branch"], "different-branch");
    assert_eq!(listing, rows(&pushed));
    let inventory = git(root, &["worktree", "list", "--porcelain"]);
    let expected: Vec<_> = inventory
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .filter(|path| path.contains("/.lane/trees/"))
        .collect();
    let actual: Vec<_> = listing
        .iter()
        .map(|row| row["path"].as_str().unwrap())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn listing_uses_each_worktrees_upstream_config() {
    let temp = repository();
    let root = temp.path();
    let path = worktree(root, "feature");
    git(root, &["config", "extensions.worktreeConfig", "true"]);
    git(root, &["config", "remote.origin.url", "/not-used"]);
    git(
        root,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    git(root, &["config", "branch.feature.remote", "origin"]);
    git(root, &["config", "branch.feature.merge", "refs/heads/main"]);
    git(
        &path,
        &["config", "--worktree", "branch.feature.remote", "."],
    );
    for value in ["true", "false"] {
        git(
            &path,
            &["config", "--worktree", "status.aheadBehind", value],
        );
        git(&path, &["config", "--worktree", "status.branch", value]);
        assert_eq!(row(&rows(root), "feature")["state"], "pushed");
    }
    std::fs::write(root.join("file"), "main advanced\n").unwrap();
    git(root, &["commit", "-qam", "advance"]);
    assert_eq!(row(&rows(root), "feature")["state"], "open");
}

#[test]
fn listing_keeps_upstream_config_from_conditional_includes() {
    let temp = repository();
    let root = temp.path();
    let path = worktree(root, "feature");
    let git_dir = git(&path, &["rev-parse", "--absolute-git-dir"]);
    let included = root.join(".git/feature.config");
    std::fs::write(
        &included,
        "[branch \"feature\"]\nremote = .\nmerge = refs/heads/main\n",
    )
    .unwrap();
    git(
        root,
        &[
            "config",
            &format!("includeIf.gitdir:{git_dir}.path"),
            included.to_str().unwrap(),
        ],
    );
    assert_eq!(row(&rows(root), "feature")["state"], "pushed");
}

#[test]
fn marked_lanes_share_refs_and_keep_the_containment_fallback() {
    let temp = repository();
    let root = temp.path();
    let trace = root.join(".git/trace.json");
    git(root, &["config", "remote.origin.url", "/not-used"]);
    git(
        root,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    for name in ["gone", "contained", "open"] {
        let path = worktree(root, name);
        let marker = git(&path, &["rev-parse", "--git-path", "lane/landed"]);
        let marker = path.join(marker);
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        std::fs::write(marker, "marker\n").unwrap();
        if name != "contained" {
            std::fs::write(path.join("file"), name).unwrap();
            git(&path, &["commit", "-qam", name]);
        }
    }
    git(root, &["config", "branch.gone.remote", "origin"]);
    git(root, &["config", "branch.gone.merge", "refs/heads/gone"]);
    let output = run(command(root, env!("CARGO_BIN_EXE_lane"), &["ls", "--json"])
        .env("GIT_TRACE2_EVENT", &trace));
    let listing: Vec<Value> = serde_json::from_str(&output).unwrap();
    assert_eq!(row(&listing, "gone")["state"], "landed");
    assert_eq!(row(&listing, "contained")["state"], "landed");
    assert_eq!(row(&listing, "open")["state"], "open");
    let starts: Vec<Value> = std::fs::read_to_string(trace)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|event| event["event"] == "start")
        .collect();
    assert_eq!(
        starts
            .iter()
            .filter(|event| event["argv"][1] == "for-each-ref")
            .count(),
        1
    );
    assert!(!starts.iter().any(|event| {
        event["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "@{upstream}")
    }));
}
