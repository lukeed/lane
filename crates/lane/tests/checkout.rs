use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn run(root: &Path, program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?} in {}: {}\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn git(root: &Path, args: &[&str]) -> String {
    run(root, "git", args)
}

fn lane(root: &Path, args: &[&str]) -> String {
    run(root, env!("CARGO_BIN_EXE_lane"), args)
}

fn configure(root: &Path) {
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@t.t"]);
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn repository(bare: bool) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    git(&root, &["init", "-qb", "main", "seed"]);
    let seed = root.join("seed");
    configure(&seed);
    std::fs::create_dir(seed.join("src")).unwrap();
    std::fs::write(seed.join("src/file"), "base\n").unwrap();
    git(&seed, &["add", "-A"]);
    git(&seed, &["commit", "-qm", "base"]);
    let host = if bare {
        git(&root, &["clone", "--bare", "-q", "seed", "repo.git"]);
        let common = root.join("repo.git");
        configure(&common);
        let host = root.join("main");
        git(
            &common,
            &["worktree", "add", host.to_str().unwrap(), "main"],
        );
        host
    } else {
        seed
    };
    lane(&host, &["init"]);
    git(&host, &["add", "-A"]);
    git(&host, &["commit", "-qm", "lane init"]);
    (temp, host)
}

#[test]
fn checkout_preserves_existing_branch_and_metadata() {
    for bare in [false, true] {
        let (_temp, host) = repository(bare);
        git(&host, &["checkout", "-qb", "review/topic"]);
        std::fs::write(host.join("src/file"), "PR content\n").unwrap();
        git(&host, &["commit", "-qam", "PR change"]);
        let tip = git(&host, &["rev-parse", "HEAD"]);
        git(&host, &["checkout", "-q", "main"]);
        git(&host, &["branch", "--set-upstream-to=main", "review/topic"]);
        git(&host, &["config", "lane.review/topic.base", "main"]);
        std::fs::write(host.join("src/file"), "parent edits\n").unwrap();
        let path = PathBuf::from(lane(&host, &["checkout", "review/topic"]));
        assert_eq!(path, host.join(".lane/trees/review/topic"));
        assert_eq!(git(&path, &["rev-parse", "HEAD"]), tip);
        assert_eq!(git(&path, &["branch", "--show-current"]), "review/topic");
        assert_eq!(
            git(&path, &["rev-parse", "--abbrev-ref", "@{upstream}"]),
            "main"
        );
        assert_eq!(git(&path, &["config", "lane.review/topic.base"]), "main");
        assert_eq!(
            std::fs::read_to_string(path.join("src/file")).unwrap(),
            "PR content\n"
        );
        assert_eq!(
            std::fs::read_to_string(host.join("src/file")).unwrap(),
            "parent edits\n"
        );
        assert!(git(&path, &["status", "--porcelain"]).is_empty());
        let rows: Vec<Value> = serde_json::from_str(&lane(&host, &["ls", "--json"])).unwrap();
        assert_eq!(rows[0]["branch"], "review/topic");
    }
}

#[test]
fn checkout_refuses_missing_invalid_and_busy_branches_without_moving_them() {
    let (_temp, host) = repository(false);
    let tip = git(&host, &["rev-parse", "HEAD"]);
    git(&host, &["tag", "tag-only"]);
    for name in [
        "missing",
        "tag-only",
        "main",
        "../escape",
        "/tmp/escape",
        "HEAD",
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_lane"))
            .args(["checkout", name])
            .current_dir(&host)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{name}");
        assert!(out.stdout.is_empty());
        assert_eq!(git(&host, &["rev-parse", "HEAD"]), tip);
    }
    assert!(git(&host, &["branch", "--list", "missing"]).is_empty());
    git(&host, &["branch", "review"]);
    let path = lane(&host, &["checkout", "review"]);
    let out = Command::new(env!("CARGO_BIN_EXE_lane"))
        .args(["checkout", "review"])
        .current_dir(&host)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(git(Path::new(&path), &["rev-parse", "HEAD"]), tip);
}

#[test]
fn checkout_opens_a_fetched_pull_request_branch() {
    let (_temp, host) = repository(false);
    let remote = host.parent().unwrap().join("remote.git");
    git(
        &host,
        &["clone", "--bare", "-q", ".", remote.to_str().unwrap()],
    );
    git(&remote, &["update-ref", "refs/pull/123/head", "HEAD"]);
    git(
        &host,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&host, &["fetch", "origin", "pull/123/head:pr-123"]);
    let path = lane(&host, &["checkout", "pr-123"]);
    assert_eq!(
        git(Path::new(&path), &["rev-parse", "HEAD"]),
        git(&remote, &["rev-parse", "refs/pull/123/head"])
    );
    assert_eq!(
        git(Path::new(&path), &["branch", "--show-current"]),
        "pr-123"
    );
}
