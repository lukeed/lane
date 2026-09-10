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
fn bare_lanes_resolve_the_host_at_every_name_depth() {
    let (_temp, host) = repository(true);
    let names = ["alpha", "feat/login", "feat/auth/login"];
    for name in names {
        let path = PathBuf::from(lane(&host, &["new", name]));
        assert_eq!(path, host.join(".lane/trees").join(name));
        assert_eq!(lane(&path.join("src"), &["exit"]), host.to_str().unwrap());
    }
    for name in names {
        let path = host.join(".lane/trees").join(name);
        let rows: Vec<Value> = serde_json::from_str(&lane(&path, &["ls", "--json"])).unwrap();
        assert_eq!(rows.len(), names.len());
        for name in names {
            assert!(rows.iter().any(|row| {
                row["name"] == name
                    && row["branch"] == name
                    && row["path"] == host.join(".lane/trees").join(name).to_str().unwrap()
            }));
        }
        assert_eq!(
            lane(&path, &["enter", "feat/auth/login"]),
            host.join(".lane/trees/feat/auth/login").to_str().unwrap()
        );
    }
}

#[test]
fn merge_removes_the_full_lane_name_and_keeps_its_basename_sibling() {
    for bare in [false, true] {
        let (_temp, host) = repository(bare);
        let sibling = PathBuf::from(lane(&host, &["new", "login"]));
        let path = PathBuf::from(lane(&host, &["new", "feat/auth/login"]));
        std::fs::write(path.join("src/file"), "changed\n").unwrap();
        git(&path, &["commit", "-qam", "change"]);

        lane(&path.join("src"), &["merge"]);

        assert_eq!(
            std::fs::read_to_string(host.join("src/file")).unwrap(),
            "changed\n"
        );
        assert!(!path.exists());
        assert!(git(&host, &["branch", "--list", "feat/auth/login"]).is_empty());
        assert!(sibling.is_dir());
        assert_eq!(git(&sibling, &["branch", "--show-current"]), "login");
    }
}

#[test]
fn prune_removes_the_full_lane_name_and_keeps_its_basename_sibling() {
    for bare in [false, true] {
        let (_temp, host) = repository(bare);
        let sibling = PathBuf::from(lane(&host, &["new", "login"]));
        let path = PathBuf::from(lane(&host, &["new", "feat/auth/login"]));
        std::fs::write(path.join("src/file"), "changed\n").unwrap();
        git(&path, &["commit", "-qam", "change"]);
        lane(&path, &["merge", "--keep"]);

        assert_eq!(
            lane(&host, &["prune", "--dry-run"]),
            "would remove feat/auth/login"
        );
        assert!(path.is_dir());
        assert_eq!(lane(&host, &["prune"]), "removed feat/auth/login");

        assert!(!path.exists());
        assert!(git(&host, &["branch", "--list", "feat/auth/login"]).is_empty());
        assert!(sibling.is_dir());
        assert_eq!(git(&sibling, &["branch", "--show-current"]), "login");
    }
}
