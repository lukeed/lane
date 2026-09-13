use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn command(root: &Path, program: &str, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn git(root: &Path, args: &[&str]) -> String {
    success(command(root, "git", args).output().unwrap())
}

fn lane(root: &Path, args: &[&str]) -> String {
    success(
        command(root, env!("CARGO_BIN_EXE_lane"), args)
            .output()
            .unwrap(),
    )
}

fn repository(ignore: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for args in [
        &["init", "-qb", "main"][..],
        &["config", "user.name", "Test"],
        &["config", "user.email", "test@example.test"],
        &["config", "commit.gpgsign", "false"],
    ] {
        git(&root, args);
    }
    std::fs::write(root.join("file"), "base\n").unwrap();
    std::fs::write(root.join(".gitignore"), ignore).unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "base"]);
    (temp, root)
}

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn new_lanes_never_copy_siblings_or_trash() {
    for ignore in [".lane/", ".lane/*", ".lane/trees/", ".lane/trees/*"] {
        let (_temp, root) = repository(&format!("{ignore}\ncache/\n.lane-cache/\n"));
        let reflink = lane::cow::probe(&root).0;
        write(&root, "cache/blob", "warm\n");
        write(&root, ".lane-cache/blob", "unrelated\n");
        write(&root, ".lane/trees/.trash/stale/blob", "trash\n");
        let first = PathBuf::from(lane(&root, &["new", "first"]));
        write(&first, "cache/only-in-first", "sibling\n");

        for name in ["second", "third"] {
            let dest = PathBuf::from(lane(&root, &["new", name]));

            assert!(!dest.join(".lane/trees").exists(), "{ignore}: {name}");
            assert_eq!(dest.join("cache/blob").exists(), reflink);
            assert_eq!(dest.join(".lane-cache/blob").exists(), reflink);
            assert_eq!(
                std::fs::read_to_string(dest.join("file")).unwrap(),
                "base\n"
            );
            assert!(git(&dest, &["status", "--porcelain"]).is_empty());
        }
        assert!(first.join("cache/only-in-first").exists());
        assert!(root.join(".lane/trees/.trash/stale/blob").exists());
    }
}

#[test]
fn dirty_lanes_keep_notes_and_edits_without_copying_siblings() {
    let (_temp, root) = repository(".lane/\ncache/\n");
    write(&root, ".lane/memory/file/note.md", "committed note\n");
    git(&root, &["add", "-f", ".lane/memory/file/note.md"]);
    git(&root, &["commit", "-qm", "note"]);
    lane(&root, &["new", "first"]);
    write(&root, ".lane/memory/file/note.md", "edited note\n");
    write(&root, ".lane/trees/.trash/stale/blob", "trash\n");
    write(&root, "file", "edited\n");
    write(&root, "untracked", "new\n");
    write(&root, "cache/blob", "warm\n");

    let dest = PathBuf::from(lane(&root, &["new", "dirty", "--dirty"]));

    assert!(!dest.join(".lane/trees").exists());
    assert_eq!(
        std::fs::read_to_string(dest.join("file")).unwrap(),
        "edited\n"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("untracked")).unwrap(),
        "new\n"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join(".lane/memory/file/note.md")).unwrap(),
        "edited note\n"
    );
    assert_eq!(dest.join("cache/blob").exists(), lane::cow::probe(&root).0);
}

#[test]
fn ignored_lane_roots_do_not_hide_tracked_notes() {
    let (_temp, root) = repository(".lane/\n");
    write(&root, ".lane/memory/file/note.md", "committed note\n");
    git(&root, &["add", "-f", ".lane/memory/file/note.md"]);
    git(&root, &["commit", "-qm", "note"]);
    lane(&root, &["new", "first"]);

    let dest = PathBuf::from(lane(&root, &["new", "second"]));

    assert!(!dest.join(".lane/trees").exists());
    assert_eq!(
        std::fs::read_to_string(dest.join(".lane/memory/file/note.md")).unwrap(),
        "committed note\n"
    );
}

#[test]
fn ignored_lane_roots_do_not_copy_external_trees() {
    let (_temp, root) = repository(".lane/\n");
    let external = TempDir::new().unwrap();
    std::fs::create_dir(root.join(".lane")).unwrap();
    std::os::unix::fs::symlink(external.path(), root.join(".lane/trees")).unwrap();
    lane(&root, &["new", "first"]);

    let dest = PathBuf::from(lane(&root, &["new", "second"]));

    assert!(std::fs::symlink_metadata(dest.join(".lane/trees")).is_err());
    assert!(external.path().join("first/file").exists());
}
