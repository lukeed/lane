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

fn trash_is_empty(root: &Path) -> bool {
    let trash = root.join(".lane/trees/.trash");
    !trash.exists() || std::fs::read_dir(trash).unwrap().next().is_none()
}

#[test]
fn removal_finishes_cleanup_without_an_external_deleter() {
    use std::os::unix::fs::PermissionsExt;

    let (_temp, root) = repository("cache/\n");
    let dest = PathBuf::from(lane(&root, &["new", "topic"]));
    write(&dest, "cache/blob", "bulk\n");
    write(&root, ".lane/trees/.trash/stale/blob", "old trash\n");
    let bin = TempDir::new().unwrap();
    write(bin.path(), "rm", "#!/bin/sh\nexit 1\n");
    std::fs::set_permissions(
        bin.path().join("rm"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let mut paths = vec![bin.path().to_path_buf()];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));

    success(
        command(
            &root,
            env!("CARGO_BIN_EXE_lane"),
            &["rm", "topic", "--force"],
        )
        .env("PATH", std::env::join_paths(paths).unwrap())
        .output()
        .unwrap(),
    );

    assert!(!dest.exists());
    assert!(trash_is_empty(&root));
    assert!(git(&root, &["branch", "--list", "topic"]).is_empty());
}

#[test]
fn prune_clears_old_trash_without_following_symlinks() {
    let (_temp, root) = repository("");
    let outside = TempDir::new().unwrap();
    write(outside.path(), "keep", "external\n");
    for i in 0..165 {
        write(
            &root,
            &format!(".lane/trees/.trash/{i}-0/blob"),
            "old trash\n",
        );
    }
    write(&root, ".lane/trees/.trash/ignored-file", "file\n");
    std::os::unix::fs::symlink(outside.path(), root.join(".lane/trees/.trash/link")).unwrap();
    std::os::unix::fs::symlink("missing", root.join(".lane/trees/.trash/broken")).unwrap();

    lane(&root, &["prune", "--dry-run"]);
    assert!(root.join(".lane/trees/.trash/0-0/blob").exists());
    lane(&root, &["prune"]);

    assert!(trash_is_empty(&root));
    assert_eq!(
        std::fs::read_to_string(outside.path().join("keep")).unwrap(),
        "external\n"
    );
}

#[test]
fn cleanup_errors_leave_the_branch_for_a_retry() {
    use std::os::unix::fs::PermissionsExt;

    let (_temp, root) = repository("cache/\n");
    let dest = PathBuf::from(lane(&root, &["new", "topic"]));
    write(&dest, "cache/blob", "bulk\n");
    write(&root, ".lane/trees/.trash/stale/locked/blob", "old trash\n");
    let locked = root.join(".lane/trees/.trash/stale/locked");
    let permissions = std::fs::metadata(&locked).unwrap().permissions();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let denied = std::fs::remove_file(locked.join("blob")).is_err();
    if !denied {
        std::fs::set_permissions(&locked, permissions).unwrap();
        eprintln!("skipped: this user can remove files from read-only directories");
        return;
    }

    let output = command(
        &root,
        env!("CARGO_BIN_EXE_lane"),
        &["rm", "topic", "--force"],
    )
    .output()
    .unwrap();
    std::fs::set_permissions(&locked, permissions).unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(".lane/trees/.trash/stale"));
    assert!(!dest.exists());
    assert!(!git(&root, &["branch", "--list", "topic"]).is_empty());
    assert!(locked.join("blob").exists());

    lane(&root, &["rm", "topic", "--force"]);

    assert!(trash_is_empty(&root));
    assert!(git(&root, &["branch", "--list", "topic"]).is_empty());
}

#[test]
fn prune_reports_an_unreadable_trash_directory() {
    let (_temp, root) = repository("");
    write(&root, ".lane/trees/.trash", "not a directory\n");

    let output = command(&root, env!("CARGO_BIN_EXE_lane"), &["prune"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(".lane/trees/.trash"));
    assert_eq!(
        std::fs::read_to_string(root.join(".lane/trees/.trash")).unwrap(),
        "not a directory\n"
    );
}

#[test]
fn failed_git_removal_restores_parked_files() {
    let (_temp, root) = repository("cache/\n");
    let dest = PathBuf::from(lane(&root, &["new", "topic"]));
    write(&dest, "cache/blob", "keep\n");
    git(&root, &["worktree", "lock", dest.to_str().unwrap()]);

    let output = command(
        &root,
        env!("CARGO_BIN_EXE_lane"),
        &["rm", "topic", "--force"],
    )
    .output()
    .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        std::fs::read_to_string(dest.join("cache/blob")).unwrap(),
        "keep\n"
    );
    assert!(trash_is_empty(&root));
    assert!(!git(&root, &["branch", "--list", "topic"]).is_empty());
}

#[test]
fn concurrent_removals_preserve_a_locked_lane() {
    let (_temp, root) = repository("cache/\n");
    for name in ["first", "second", "locked"] {
        let dest = PathBuf::from(lane(&root, &["new", name]));
        write(&dest, "cache/blob", name);
    }
    let locked = root.join(".lane/trees/locked");
    git(&root, &["worktree", "lock", locked.to_str().unwrap()]);
    let children: Vec<_> = [
        &["rm", "first", "--force"][..],
        &["rm", "second", "--force"],
        &["rm", "locked", "--force"],
        &["prune"],
    ]
    .into_iter()
    .map(|args| {
        command(&root, env!("CARGO_BIN_EXE_lane"), args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    })
    .collect();

    for (index, child) in children.into_iter().enumerate() {
        let output = child.wait_with_output().unwrap();
        if index == 2 {
            assert!(!output.status.success());
        } else {
            success(output);
        }
    }

    assert!(!root.join(".lane/trees/first").exists());
    assert!(!root.join(".lane/trees/second").exists());
    assert_eq!(
        std::fs::read_to_string(locked.join("cache/blob")).unwrap(),
        "locked"
    );
    assert!(trash_is_empty(&root));
}
