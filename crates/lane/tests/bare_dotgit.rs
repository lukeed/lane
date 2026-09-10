use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn run(program: &str, root: &Path, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?} in {} failed:\n{}\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn git(root: &Path, args: &[&str]) -> String {
    run("git", root, args)
}

fn lane(root: &Path, args: &[&str]) -> String {
    run(env!("CARGO_BIN_EXE_lane"), root, args)
}

fn seed(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-qb", "main"]);
    for (key, value) in [
        ("user.name", "t"),
        ("user.email", "t@t.t"),
        ("commit.gpgsign", "false"),
    ] {
        git(root, &["config", key, value]);
    }
    fs::write(root.join("README.md"), "initial\n").unwrap();
    git(root, &["add", "README.md"]);
    git(root, &["commit", "-qm", "initial"]);
}

fn bare_dotgit() -> (TempDir, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap().join("repo");
    seed(&root);
    fs::remove_file(root.join("README.md")).unwrap();
    git(&root, &["config", "core.bare", "true"]);
    git(&root, &["worktree", "add", "main", "main"]);
    let host = root.join("main");
    (temp, root, host)
}

fn initialize(host: &Path) {
    lane(host, &["init"]);
    assert!(host.join("AGENTS.md").is_file());
    git(host, &["add", "-A"]);
    git(host, &["commit", "-qm", "lane init"]);
}

fn land_change(start: &Path, host: &Path) {
    let path = PathBuf::from(lane(start, &["new", "alpha"]));
    assert_eq!(path, host.join(".lane/trees/alpha"));
    assert_eq!(PathBuf::from(lane(&path, &["exit"])), host);
    let rows: serde_json::Value = serde_json::from_str(&lane(host, &["ls", "--json"])).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["branch"], "alpha");
    fs::write(path.join("change.txt"), "landed\n").unwrap();
    git(&path, &["add", "change.txt"]);
    git(&path, &["commit", "-qm", "change"]);
    lane(&path, &["merge"]);
    assert_eq!(
        fs::read_to_string(host.join("change.txt")).unwrap(),
        "landed\n"
    );
    assert!(!path.exists());
    assert_eq!(git(host, &["branch", "--list", "alpha"]), "");
    assert_eq!(git(host, &["status", "--porcelain"]), "");
}

#[test]
fn bare_dotgit_lanes_land_in_the_hosting_worktree() {
    let (_temp, root, host) = bare_dotgit();
    initialize(&host);
    assert!(!root.join("AGENTS.md").exists());
    assert!(!root.join(".lane").exists());
    land_change(&host, &host);
}

#[test]
fn bare_dotgit_uses_included_boolean_configuration() {
    let (_temp, root, host) = bare_dotgit();
    git(&root, &["config", "core.bare", "false"]);
    fs::write(root.join(".git/bare.config"), "[core]\n\tbare = on\n").unwrap();
    git(&root, &["config", "include.path", "bare.config"]);
    initialize(&host);
    assert!(!root.join("AGENTS.md").exists());
    land_change(&host, &host);
}

#[test]
fn bare_dotgit_uses_worktree_configuration() {
    let (_temp, root, host) = bare_dotgit();
    git(&root, &["config", "extensions.worktreeConfig", "true"]);
    git(&root, &["config", "--unset", "core.bare"]);
    fs::write(root.join(".git/config.worktree"), "[core]\n\tbare = true\n").unwrap();
    initialize(&host);
    assert!(!root.join("AGENTS.md").exists());
    land_change(&host, &host);
}

#[test]
fn normal_linked_worktrees_keep_the_primary_root() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap().join("repo");
    seed(&root);
    initialize(&root);
    let linked = temp.path().canonicalize().unwrap().join("linked");
    git(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    land_change(&linked, &root);
}
