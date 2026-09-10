use lane::note::{Meta, Note};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run(root: &Path, program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn lane(root: &Path, args: &[&str]) -> String {
    run(root, env!("CARGO_BIN_EXE_lane"), args)
}

fn fixture() -> TempDir {
    let temp = TempDir::new().unwrap();
    for args in [
        &["init", "-qb", "main"][..],
        &["config", "user.name", "t"],
        &["config", "user.email", "t@t.t"],
        &["config", "commit.gpgsign", "false"],
        &["commit", "--allow-empty", "-qm", "initial"],
    ] {
        run(temp.path(), "git", args);
    }
    lane(temp.path(), &["init"]);
    fs::create_dir(temp.path().join("src")).unwrap();
    temp
}

fn add(root: &Path, path: &str, text: &str) {
    lane(root, &["note", "add", path, "-a", "fn verify", text]);
}

fn check(root: &Path) -> Vec<Value> {
    serde_json::from_str(&lane(root, &["check", "--json"])).unwrap()
}

fn row<'a>(rows: &'a [Value], text: &str) -> &'a Value {
    rows.iter().find(|row| row["note"] == text).unwrap()
}

#[test]
fn check_and_audit_keep_independent_baselines_and_files() {
    let temp = fixture();
    let root = temp.path();
    let first = "pub fn verify(value: usize) -> usize {\n    value + 1 // original\n}\n";
    let second = "pub fn verify(value: usize) -> usize {\n    value + 9\n}\n";
    fs::write(root.join("src/a.rs"), first).unwrap();
    fs::write(root.join("src/b.rs"), second).unwrap();
    add(root, "src/a.rs", "a old");
    add(root, "src/b.rs", "b note");
    lane(root, &["audit", "--base", "main"]);

    fs::write(
        root.join("src/a.rs"),
        "pub fn verify(value: usize) -> usize {\r\n value   + 1 // revised\r\n}\r\n",
    )
    .unwrap();
    assert!(check(root).iter().all(|row| row["tier"] == "fresh"));
    add(root, "src/a.rs", "a formatted");
    lane(root, &["audit", "--base", "main"]);
    let baselines: Vec<_> = lane::store::load_notes(root, None)
        .into_iter()
        .map(|note| (note.file.unwrap(), note.raw))
        .collect();

    let changed = "pub fn verify(value: usize) -> usize {\n    value + 2\n}\n";
    fs::write(root.join("src/a.rs"), changed).unwrap();
    add(root, "src/a.rs", "a current");
    let audit: Value =
        serde_json::from_str(&lane(root, &["audit", "--base", "main", "--json"])).unwrap();
    assert_eq!(audit["checked"]["content-changed"], 2);
    assert_eq!(audit["checked"]["fresh"], 2);
    for (path, bytes) in baselines {
        assert_eq!(fs::read_to_string(path).unwrap(), bytes);
    }
    let rows = check(root);
    assert_eq!(rows.len(), 4);
    for text in ["a old", "a formatted"] {
        assert_eq!(row(&rows, text)["tier"], "content-changed");
        assert_eq!(row(&rows, text)["span"], changed);
    }
    for text in ["a current", "b note"] {
        assert_eq!(row(&rows, text)["tier"], "fresh");
        assert!(row(&rows, text).get("span").is_none());
    }
    lane(
        root,
        &[
            "note",
            "confirm",
            row(&rows, "a old")["id"].as_str().unwrap(),
        ],
    );
    let rows = check(root);
    assert_eq!(row(&rows, "a old")["tier"], "fresh");
    assert_eq!(row(&rows, "a formatted")["tier"], "content-changed");
    assert_eq!(row(&rows, "b note")["tier"], "fresh");
}

#[test]
fn check_json_keeps_missing_and_unparsed_spans_empty() {
    let temp = fixture();
    let root = temp.path();
    fs::write(root.join("src/missing.rs"), "fn keep() {}\n").unwrap();
    fs::write(root.join("src/unknown.swift"), "func verify() {}\n").unwrap();
    for (index, (path, anchor)) in [
        ("src/missing.rs", "fn verify"),
        ("src/missing.rs", "fn verify"),
        ("src/unknown.swift", "func verify"),
        ("src/unknown.swift", "func verify"),
        ("src/absent.rs", "@file"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("01M0{index:022}");
        Note::new(
            Meta {
                id: id.clone(),
                anchor: anchor.into(),
                ..Default::default()
            },
            format!("finding {index}"),
        )
        .write(&lane::store::note_dir(root, path).join(format!("{id}-note.md")))
        .unwrap();
    }
    let rows = check(root);
    assert_eq!(rows.len(), 5);
    for row in rows {
        let expected = if row["path"] == "src/unknown.swift" {
            "unverifiable"
        } else {
            "anchor-missing"
        };
        assert_eq!(row["tier"], expected);
        assert_eq!(row["span"], "");
    }
}
