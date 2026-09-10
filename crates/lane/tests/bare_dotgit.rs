use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn run(program: &str, root: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
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
        .envs(env.iter().copied())
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
    run("git", root, args, &[])
}

fn lane(root: &Path, args: &[&str]) -> String {
    run(env!("CARGO_BIN_EXE_lane"), root, args, &[])
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
fn bare_dotgit_checks_the_common_config_with_inherited_git_paths() {
    let (_temp, root, host) = bare_dotgit();
    let git_dir = git(&host, &["rev-parse", "--absolute-git-dir"]);
    git(&root, &["config", "extensions.worktreeConfig", "true"]);
    git(&root, &["config", "--unset", "core.bare"]);
    fs::write(root.join(".git/config.worktree"), "[core]\n\tbare = true\n").unwrap();
    fs::write(
        Path::new(&git_dir).join("config.worktree"),
        "[core]\n\tbare = false\n",
    )
    .unwrap();

    let destination = run(
        env!("CARGO_BIN_EXE_lane"),
        &host,
        &["exit"],
        &[
            ("GIT_DIR", &git_dir),
            ("GIT_COMMON_DIR", "../.git"),
            ("GIT_WORK_TREE", host.to_str().unwrap()),
        ],
    );
    assert_eq!(PathBuf::from(destination), host);
}

#[test]
fn primary_root_uses_inherited_config_overrides() {
    let (_temp, root, host) = bare_dotgit();
    let config = root.join("override.config");
    fs::write(&config, "[core]\n\tbare = false\n").unwrap();
    for overrides in [
        vec![
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "core.bare"),
            ("GIT_CONFIG_VALUE_0", "false"),
        ],
        vec![("GIT_CONFIG_PARAMETERS", "'core.bare=false'")],
        vec![("GIT_CONFIG", config.to_str().unwrap())],
    ] {
        let destination = run(env!("CARGO_BIN_EXE_lane"), &host, &["exit"], &overrides);
        assert_eq!(PathBuf::from(destination), root);
    }
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

#[test]
fn local_bare_values_need_no_git_process() {
    let (_temp, root, host) = bare_dotgit();
    let trace = root.join("trace.json");
    let global = root.join("global.config");
    fs::write(&global, "[core]\n\tbare = false\n").unwrap();
    for (value, expected) in [
        ("true", &host),
        ("false", &root),
        ("yes", &host),
        ("no", &root),
        ("1", &host),
        ("0", &root),
    ] {
        git(&root, &["config", "core.bare", value]);
        let destination = run(
            env!("CARGO_BIN_EXE_lane"),
            &host,
            &["exit"],
            &[
                ("GIT_TRACE2_EVENT", trace.to_str().unwrap()),
                ("GIT_CONFIG_GLOBAL", global.to_str().unwrap()),
            ],
        );
        assert_eq!(PathBuf::from(destination), *expected);
        assert!(!trace.exists(), "layout spawned Git for core.bare={value}");
    }
}

#[test]
fn bare_dotgit_uses_conditional_includes_for_the_common_directory() {
    let (_temp, root, host) = bare_dotgit();
    git(&root, &["config", "core.bare", "false"]);
    fs::write(root.join(".git/bare.config"), "[core]\n\tbare = true\n").unwrap();
    let key = format!("includeIf.gitdir:{}.path", root.join(".git").display());
    git(&root, &["config", &key, "bare.config"]);
    assert_eq!(PathBuf::from(lane(&host, &["exit"])), host);
}

#[test]
fn missing_local_bare_uses_global_configuration() {
    let (_temp, root, host) = bare_dotgit();
    git(&root, &["config", "--unset", "core.bare"]);
    let global = root.join("global.config");
    fs::write(&global, "[core]\n\tbare = true\n").unwrap();
    let destination = run(
        env!("CARGO_BIN_EXE_lane"),
        &host,
        &["exit"],
        &[("GIT_CONFIG_GLOBAL", global.to_str().unwrap())],
    );
    assert_eq!(PathBuf::from(destination), host);
}

#[test]
fn global_worktree_extension_does_not_override_local_bare() {
    let (_temp, root, host) = bare_dotgit();
    git(&root, &["config", "core.bare", "false"]);
    let global = root.join("global.config");
    fs::write(&global, "[extensions]\n\tworktreeConfig = true\n").unwrap();
    fs::write(root.join(".git/config.worktree"), "[core]\n\tbare = true\n").unwrap();
    let destination = run(
        env!("CARGO_BIN_EXE_lane"),
        &host,
        &["exit"],
        &[("GIT_CONFIG_GLOBAL", global.to_str().unwrap())],
    );
    assert_eq!(PathBuf::from(destination), root);
}
