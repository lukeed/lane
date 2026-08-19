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
