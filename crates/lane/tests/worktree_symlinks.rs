use lane::git::git;
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn lane(root: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_lane"))
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "lane {args:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn repository() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    for args in [
        &["init", "-qb", "main"][..],
        &["config", "user.name", "Test"],
        &["config", "user.email", "test@example.test"],
        &["config", "commit.gpgsign", "false"],
        &["config", "core.hooksPath", ".git/hooks"],
    ] {
        git(args, Some(root)).unwrap();
    }
    std::fs::write(root.join("file.txt"), "base\n").unwrap();
    lane(root, &["init"]);
    git(&["add", "-A"], Some(root)).unwrap();
    git(&["commit", "-qm", "base"], Some(root)).unwrap();
    temp
}

#[cfg(unix)]
#[test]
fn symlinked_trees_keep_lanes_visible_and_mergeable() {
    let temp = repository();
    let root = temp.path();
    let external = TempDir::new().unwrap();
    let trees = external.path().join("trees");
    let sibling = external.path().join("trees-sibling");
    std::fs::create_dir(&trees).unwrap();
    std::os::unix::fs::symlink(&trees, root.join(".lane/trees")).unwrap();
    git(
        &[
            "worktree",
            "add",
            "-qb",
            "sibling",
            sibling.to_str().unwrap(),
        ],
        Some(root),
    )
    .unwrap();

    lane(root, &["new", "topic"]);
    let rows: Vec<Value> = serde_json::from_str(&lane(root, &["ls", "--json"])).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "topic");
    let topic = trees.join("topic").canonicalize().unwrap();
    assert_eq!(rows[0]["path"], topic.to_str().unwrap());

    std::fs::write(topic.join("file.txt"), "changed\n").unwrap();
    git(&["add", "file.txt"], Some(&topic)).unwrap();
    git(&["commit", "-qm", "change"], Some(&topic)).unwrap();
    lane(root, &["merge", "topic"]);

    assert_eq!(
        std::fs::read_to_string(root.join("file.txt")).unwrap(),
        "changed\n"
    );
    assert!(!topic.exists());
    assert!(sibling.is_dir());
    assert_eq!(lane(root, &["ls", "--json"]).trim(), "[]");
}

#[test]
fn missing_trees_keep_registered_lanes_visible() {
    let temp = repository();
    let root = temp.path();
    lane(root, &["new", "gone"]);
    std::fs::remove_dir_all(root.join(".lane/trees")).unwrap();

    let rows: Vec<Value> = serde_json::from_str(&lane(root, &["ls", "--json"])).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "gone");

    lane(root, &["rm", "gone", "--force"]);
    assert_eq!(lane(root, &["ls", "--json"]).trim(), "[]");
}
