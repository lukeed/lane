use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(root: &Path, program: &str, args: &[&str]) -> Output {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn lane(root: &Path, args: &[&str]) -> Output {
    run(root, env!("CARGO_BIN_EXE_lane"), args)
}

fn repository() -> TempDir {
    let temp = TempDir::new().unwrap();
    run(temp.path(), "git", &["init", "-qb", "main"]);
    temp
}

fn add(root: &Path, path: &str, text: &str) -> Output {
    let source = root.join(path);
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(source, "pub fn run() {}\n").unwrap();
    lane(root, &["note", "add", path, "-a", "fn run", text])
}

fn rows(root: &Path, path: &str) -> Vec<Value> {
    serde_json::from_slice(&lane(root, &["why", path, "--json"]).stdout).unwrap()
}

fn note_file(root: &Path, path: &str) -> PathBuf {
    std::fs::read_dir(root.join(".lane/memory").join(path))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
}

fn damage(root: &Path, path: &str) {
    let file = note_file(root, path);
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        file,
        text.replacen("created:", "created: duplicate\ncreated:", 1),
    )
    .unwrap();
}

#[test]
fn scoped_reads_and_additions_skip_unrelated_frontmatter() {
    let temp = repository();
    let root = temp.path();
    add(root, "src/target.rs", "keep the target invariant");
    add(root, "src-gen/other.rs", "keep the other invariant");
    damage(root, "src-gen/other.rs");

    for path in ["src/target.rs", "src", "missing.rs"] {
        assert!(lane(root, &["why", path, "--json"]).stderr.is_empty());
    }
    let before = rows(root, "src/target.rs");
    assert!(
        add(root, "src/target.rs", "keep the target invariant")
            .stderr
            .is_empty()
    );
    assert_eq!(rows(root, "src/target.rs"), before);
    assert!(
        add(root, "src/target.rs", "keep another target invariant")
            .stderr
            .is_empty()
    );
    assert_eq!(rows(root, "src/target.rs").len(), 2);
    assert!(
        add(root, "src/second.rs", "keep the target invariant")
            .stderr
            .is_empty()
    );
    assert_eq!(rows(root, "src/second.rs").len(), 1);

    for args in [
        &["why", "--json"][..],
        &["why", "src-gen/other.rs", "--json"],
        &["check", "--json"],
    ] {
        let output = lane(root, args);
        assert!(String::from_utf8_lossy(&output.stderr).contains("unreadable frontmatter"));
    }
}

#[test]
fn directory_scopes_keep_component_boundaries_and_missing_sources() {
    let temp = repository();
    let root = temp.path();
    for path in ["src/one.rs", "src/deep/two.rs", "src-gen/three.rs"] {
        add(root, path, "keep the invariant");
    }
    std::fs::remove_file(root.join("src/one.rs")).unwrap();

    assert_eq!(rows(root, "src").len(), 2);
    assert_eq!(rows(root, "src/one.rs").len(), 1);
    assert_eq!(rows(root, "src/deep").len(), 1);
    assert!(rows(root, "src/de").is_empty());
    assert_eq!(rows(root, ".").len(), 3);
    assert_eq!(rows(&root.join("src"), ".").len(), 2);
}

#[test]
fn replacements_keep_cross_path_links_and_retired_notes_separate() {
    let temp = repository();
    let root = temp.path();
    add(root, "src/old.rs", "keep the original invariant");
    let old = rows(root, "src/old.rs").remove(0);
    let id = old["id"].as_str().unwrap();
    let original = note_file(root, "src/old.rs");
    let bytes = std::fs::read(&original).unwrap();
    std::fs::write(root.join("src/new.rs"), "pub fn run() {}\n").unwrap();

    lane(
        root,
        &[
            "note",
            "replace",
            id,
            "keep the new invariant",
            "--path",
            "src/new.rs",
        ],
    );
    assert!(rows(root, "src/old.rs").is_empty());
    assert_eq!(rows(root, "src/new.rs").len(), 1);
    let replacement = lane::store::load_notes(root, Some("src/new.rs"));
    assert_eq!(replacement[0].meta.supersedes, id);
    let retired = root
        .join(".lane/attic/src/old.rs")
        .join(original.file_name().unwrap());
    assert_eq!(std::fs::read(retired).unwrap(), bytes);
    assert!(!original.exists());

    add(root, "src/old.rs", "keep the original invariant");
    assert_eq!(rows(root, "src/old.rs").len(), 1);
    assert_ne!(rows(root, "src/old.rs")[0]["id"], old["id"]);
    assert_eq!(lane::store::load_retired(root, Some("src/old.rs")).len(), 1);
}

#[cfg(unix)]
#[test]
fn scoped_reads_do_not_follow_inner_memory_symlinks() {
    let temp = repository();
    let root = temp.path();
    let external = TempDir::new().unwrap();
    for (index, path) in ["src/one.rs", "src/deep/two.rs", "src/three.rs"]
        .into_iter()
        .enumerate()
    {
        add(root, path, "keep the invariant");
        let from = if index == 2 {
            note_file(root, path)
        } else {
            root.join(".lane/memory")
                .join(if index == 1 { "src/deep" } else { path })
        };
        let to = external.path().join(index.to_string());
        std::fs::rename(&from, &to).unwrap();
        std::os::unix::fs::symlink(&to, &from).unwrap();
        assert!(rows(root, path).is_empty());
    }
    assert!(rows(root, "src").is_empty());
    assert!(rows(root, ".").is_empty());
}

#[cfg(unix)]
#[test]
fn scoped_reads_follow_the_existing_memory_root_symlink() {
    let temp = repository();
    let root = temp.path();
    let external = TempDir::new().unwrap();
    add(root, "src/one.rs", "keep the invariant");
    let from = root.join(".lane/memory");
    let to = external.path().join("memory");
    std::fs::rename(&from, &to).unwrap();
    std::os::unix::fs::symlink(&to, &from).unwrap();

    assert_eq!(rows(root, "src/one.rs").len(), 1);
    assert_eq!(rows(root, "src").len(), 1);
    assert_eq!(rows(root, ".").len(), 1);
}

#[test]
fn scoped_reads_use_the_worktree_memory_root_and_reject_escape_filters() {
    let temp = repository();
    let root = temp.path().join(".lane/trees/nested");
    std::fs::create_dir_all(&root).unwrap();
    run(&root, "git", &["init", "-qb", "main"]);
    add(&root, "src/one.rs", "keep the nested invariant");
    let all = rows(&root, ".");
    assert_eq!(rows(&root, "src"), all);
    assert_eq!(all[0]["path"], "src/one.rs");

    for filter in ["../../", "../memory/src", root.to_str().unwrap()] {
        assert!(lane::store::load_notes(&root, Some(filter)).is_empty());
    }
}

#[test]
fn scoped_reads_keep_the_deepest_lane_path_mapping() {
    let temp = repository();
    let root = temp.path();
    add(
        root,
        "vendor/.lane/context/src/one.rs",
        "keep the invariant",
    );
    let all = rows(root, ".");
    assert_eq!(all[0]["path"], "src/one.rs");
    assert_eq!(rows(root, "src/one.rs"), all);
    assert_eq!(rows(root, "src"), all);
    assert!(rows(root, "vendor").is_empty());

    add(root, "src/one.rs", "keep the invariant");
    assert_eq!(rows(root, "src/one.rs"), all);
}
