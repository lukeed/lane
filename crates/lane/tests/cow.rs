//! The clone layer against the real filesystem, including whether extents are actually shared.

use lane::cow::{self, CloneError};
use std::fs;

/// Available bytes on the filesystem holding `path`, via df so no platform code is needed.
fn free_bytes(path: &std::path::Path) -> u64 {
    let out = std::process::Command::new("df")
        .arg("-k")
        .arg(path)
        .output()
        .expect("df");
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().nth(1).unwrap_or_default();
    let avail: u64 = line
        .split_whitespace()
        .nth(3)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    avail * 1024
}

#[test]
fn probe_returns_a_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let (ok, detail) = cow::probe(dir.path());
    assert!(!detail.is_empty(), "probe must explain itself");
    if ok {
        assert_eq!(detail, "reflink available");
    }
}

#[test]
/// Symlinks survive, and absolute in-repo symlinks follow the clone.
fn fallback_tree_is_byte_identical_and_symlinks_survive() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let out = dst.path().join("out");

    fs::create_dir_all(src.path().join("sub")).unwrap();
    fs::write(src.path().join("a.bin"), vec![7u8; 4096]).unwrap();
    fs::write(src.path().join("sub/b.bin"), vec![9u8; 4096]).unwrap();
    std::os::unix::fs::symlink("a.bin", src.path().join("link")).unwrap();

    let stats = cow::clone_tree(src.path(), &out, &|_, _| false).unwrap();

    assert_eq!(
        fs::read(src.path().join("a.bin")).unwrap(),
        fs::read(out.join("a.bin")).unwrap()
    );
    assert_eq!(
        fs::read(src.path().join("sub/b.bin")).unwrap(),
        fs::read(out.join("sub/b.bin")).unwrap()
    );
    assert!(fs::symlink_metadata(out.join("link")).unwrap().is_symlink());
    assert_eq!(stats.links, 1);
    assert_eq!(stats.cloned + stats.copied, 2);
}

#[test]
fn absolute_in_repo_symlink_follows_the_clone() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let out = dst.path().join("out");
    let source_file = src.path().join("tool");

    fs::write(&source_file, b"before clone").unwrap();
    std::os::unix::fs::symlink(&source_file, src.path().join("link")).unwrap();

    let canonical_src = src.path().canonicalize().unwrap();
    let stats = cow::clone_tree(&canonical_src, &out, &|_, _| false).unwrap();

    assert_eq!(fs::read_link(out.join("link")).unwrap(), out.join("tool"));
    fs::write(&source_file, b"source copy").unwrap();
    fs::write(out.join("tool"), b"destination copy").unwrap();
    assert_eq!(fs::read(out.join("link")).unwrap(), b"destination copy");

    let (supported, detail) = cow::probe(src.path());
    if supported {
        assert_eq!(stats.cloned, 1, "reflink clone must be exercised");
    } else {
        assert_eq!(stats.copied, 1, "fallback copy must be exercised: {detail}");
    }
}

#[test]
fn absolute_in_repo_symlink_outside_cloned_subdirectory_follows_the_clone() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let out = dst.path().join("out");
    let source_file = src.path().join("real/tool");
    let destination_file = out.join("real/tool");

    fs::create_dir_all(src.path().join("build")).unwrap();
    fs::create_dir_all(source_file.parent().unwrap()).unwrap();
    fs::create_dir_all(destination_file.parent().unwrap()).unwrap();
    fs::write(&source_file, b"parent copy").unwrap();
    fs::write(&destination_file, b"lane copy").unwrap();
    std::os::unix::fs::symlink(&source_file, src.path().join("build/link")).unwrap();

    cow::clone_tree_rooted(
        &src.path().join("build"),
        &out.join("build"),
        &|_, _| false,
        src.path(),
        &out,
    )
    .unwrap();

    assert_eq!(fs::read(out.join("build/link")).unwrap(), b"lane copy");
}

#[test]
fn absolute_external_symlink_is_byte_identical() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let out = dst.path().join("out");
    let external_file = external.path().join("tool");

    fs::write(&external_file, b"external").unwrap();
    std::os::unix::fs::symlink(&external_file, src.path().join("link")).unwrap();

    cow::clone_tree(src.path(), &out, &|_, _| false).unwrap();

    assert_eq!(
        fs::read_link(src.path().join("link")).unwrap(),
        fs::read_link(out.join("link")).unwrap()
    );
}

#[test]
fn relative_symlink_is_byte_identical() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let out = dst.path().join("out");

    fs::write(src.path().join("tool"), b"local").unwrap();
    std::os::unix::fs::symlink("tool", src.path().join("link")).unwrap();

    cow::clone_tree(src.path(), &out, &|_, _| false).unwrap();

    assert_eq!(
        fs::read_link(src.path().join("link")).unwrap(),
        fs::read_link(out.join("link")).unwrap()
    );
}

#[test]
fn skip_prunes_directories_and_files() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let out = dst.path().join("out");

    fs::create_dir_all(src.path().join("keep")).unwrap();
    fs::create_dir_all(src.path().join("drop")).unwrap();
    fs::write(src.path().join("keep/yes"), b"y").unwrap();
    fs::write(src.path().join("drop/no"), b"n").unwrap();

    cow::clone_tree(src.path(), &out, &|rel, _| rel.starts_with("drop")).unwrap();

    assert!(out.join("keep/yes").exists());
    assert!(!out.join("drop").exists());
}

/// A successful clone_file only proves the syscall was accepted; this proves the
/// filesystem did not spend the bytes. Skipped where reflink is unavailable.
#[test]
fn clone_shares_extents_where_supported() {
    let dir = tempfile::tempdir().unwrap();
    let (supported, detail) = cow::probe(dir.path());
    if !supported {
        eprintln!("skipped: no reflink here ({detail})");
        return;
    }

    let big = dir.path().join("big.bin");
    fs::write(&big, vec![0xABu8; 64 * 1024 * 1024]).unwrap();
    let _ = std::process::Command::new("sync").status();

    let before = free_bytes(dir.path());
    cow::clone_file(&big, &dir.path().join("clone.bin")).expect("clone");
    let _ = std::process::Command::new("sync").status();
    let spent = before.saturating_sub(free_bytes(dir.path()));

    assert!(
        spent < 16 * 1024 * 1024,
        "64 MiB clone spent {spent} bytes; extents were not shared"
    );
    assert_eq!(
        fs::read(&big).unwrap(),
        fs::read(dir.path().join("clone.bin")).unwrap()
    );
}

#[test]
fn cloning_onto_an_existing_path_is_not_reported_as_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    fs::write(&a, b"hello").unwrap();
    fs::write(&b, b"there").unwrap();

    // EEXIST is a real failure, not a reason to believe the filesystem cannot clone.
    match cow::clone_file(&a, &b) {
        Err(CloneError::Unsupported(_)) => {
            let (supported, _) = cow::probe(dir.path());
            assert!(!supported, "unsupported here is only valid without reflink");
        }
        Err(CloneError::Io(_)) | Ok(()) => {}
    }
}
