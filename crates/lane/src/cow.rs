//! Copy-on-write cloning through the kernel primitives, so a silent full copy is detectable.
//!
//!   Linux  FICLONE ioctl   btrfs, XFS(reflink=1), bcachefs, some ZFS
//!   macOS  clonefile(2)    APFS
//!   else   byte copy

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum CloneError {
    /// The filesystem cannot share extents; the caller should fall back.
    Unsupported(String),
    /// The destination is already there. Cheaper to be told than to ask first.
    Exists,
    Io(std::io::Error),
}

impl fmt::Display for CloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloneError::Unsupported(why) => write!(f, "{why}"),
            CloneError::Exists => write!(f, "destination exists"),
            CloneError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// ENOTSUP EOPNOTSUPP ENOTTY EXDEV EINVAL EPERM ENOSYS: "this filesystem cannot".
///
/// Darwin splits ENOTSUP (45) from EOPNOTSUPP (102) where Linux makes them equal, and
/// clonefile returns the former — so omitting it turned "no reflink here" into a hard error.
#[cfg(target_os = "linux")]
const UNSUPPORTED: &[i32] = &[95, 95, 25, 18, 22, 1, 38];
#[cfg(target_os = "macos")]
const UNSUPPORTED: &[i32] = &[45, 102, 25, 18, 22, 1, 78];
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const UNSUPPORTED: &[i32] = &[];

/// Separate "fall back to a byte copy" from real failures like ENOSPC.
fn classify(err: std::io::Error) -> CloneError {
    if err.kind() == std::io::ErrorKind::AlreadyExists {
        return CloneError::Exists;
    }
    match err.raw_os_error() {
        Some(e) if UNSUPPORTED.contains(&e) => CloneError::Unsupported(err.to_string()),
        _ => CloneError::Io(err),
    }
}

/// An absolute link into the source tree must point into the clone instead.
fn retarget(link: &Path, src_roots: &[PathBuf], dst_root: &Path) -> Option<PathBuf> {
    src_roots
        .iter()
        .find_map(|root| link.strip_prefix(root).ok())
        .map(|rel| dst_root.join(rel))
}

fn root_spellings(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut roots = vec![root.to_path_buf()];
    let canonical = root.canonicalize()?;
    if canonical != root {
        roots.push(canonical.clone());
    }
    // Git reports /private paths even when symlinks retain the user's shorter spelling.
    #[cfg(target_os = "macos")]
    if let Ok(rel) = canonical.strip_prefix("/private") {
        let alias = Path::new("/").join(rel);
        if !roots.contains(&alias) && alias.canonicalize().ok().as_ref() == Some(&canonical) {
            roots.push(alias);
        }
    }
    Ok(roots)
}

/// A linked git worktree stores a `.git` *file* (gitdir pointer), not a directory.
pub(crate) fn is_nested_worktree(path: &Path) -> bool {
    path.join(".git")
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_file())
}

fn under_lane_trees(rel: &Path) -> bool {
    rel.components().as_path() == Path::new(".lane/trees")
        || rel.starts_with(Path::new(".lane/trees"))
}

/// Paths that must never be cloned into a new lane: other lanes and nested worktrees.
pub(crate) fn should_skip_clone_path(src_root: &Path, path: &Path, is_dir: bool) -> bool {
    let Ok(rel) = path.strip_prefix(src_root) else {
        return false;
    };
    if under_lane_trees(rel) {
        return true;
    }
    is_dir && is_nested_worktree(path)
}

/// Clone one regular file by reference.
#[cfg(target_os = "macos")]
pub fn clone_file(src: &Path, dst: &Path) -> Result<u64, CloneError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_src = CString::new(src.as_os_str().as_bytes())
        .map_err(|_| CloneError::Unsupported("path contains an interior NUL".into()))?;
    let c_dst = CString::new(dst.as_os_str().as_bytes())
        .map_err(|_| CloneError::Unsupported("path contains an interior NUL".into()))?;
    // CLONE_NOFOLLOW: clone the symlink itself, never its target.
    let rc = unsafe { libc::clonefile(c_src.as_ptr(), c_dst.as_ptr(), 1) };
    if rc == 0 {
        return Ok(fs::symlink_metadata(dst).map(|m| m.len()).unwrap_or(0));
    }
    Err(classify(std::io::Error::last_os_error()))
}

#[cfg(target_os = "linux")]
pub fn clone_file(src: &Path, dst: &Path) -> Result<u64, CloneError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let source = fs::File::open(src).map_err(CloneError::Io)?;
    let info = source.metadata().map_err(CloneError::Io)?;
    let (mode, size) = (info.permissions().mode(), info.len());
    let target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(dst)
        .map_err(classify)?;

    match rustix::fs::ioctl_ficlone(&target, &source) {
        Ok(()) => Ok(size),
        Err(e) => {
            drop(target);
            let _ = fs::remove_file(dst);
            Err(classify(std::io::Error::from_raw_os_error(
                e.raw_os_error(),
            )))
        }
    }
}

/// Clone a whole directory in one call.
///
/// Darwin's clonefile takes a directory and clones the tree under it; Linux's FICLONE takes
/// a file alone, which is why the per-file walk exists at all.
#[cfg(target_os = "macos")]
fn clone_dir(src: &Path, dst: &Path) -> Result<(), CloneError> {
    clone_file(src, dst).map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn clone_dir(_src: &Path, _dst: &Path) -> Result<(), CloneError> {
    Err(CloneError::Unsupported(
        "no directory clone primitive on this platform".into(),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn clone_file(_src: &Path, _dst: &Path) -> Result<u64, CloneError> {
    Err(CloneError::Unsupported(
        "no clone primitive on this platform".into(),
    ))
}

/// Whether the filesystem holding `path` can share extents.
pub fn probe(path: &Path) -> (bool, String) {
    let dir = match tempfile::Builder::new()
        .prefix(".lane-probe")
        .tempdir_in(path)
    {
        Ok(d) => d,
        Err(e) => return (false, e.to_string()),
    };
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    if let Err(e) = fs::write(&a, vec![0u8; 4096]) {
        return (false, e.to_string());
    }
    match clone_file(&a, &b) {
        Ok(_) => (true, "reflink available".into()),
        Err(e) => (false, e.to_string()),
    }
}

#[derive(Default)]
pub struct CloneStats {
    pub cloned: u64,
    pub copied: u64,
    pub links: u64,
    pub bytes_shared: u64,
    pub bytes_copied: u64,
}

impl fmt::Display for CloneStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mb = |n: u64| n as f64 / (1024.0 * 1024.0);
        if self.copied == 0 && self.cloned > 0 {
            write!(
                f,
                "{} files cloned ({:.1} MiB shared, 0 copied)",
                self.cloned,
                mb(self.bytes_shared)
            )
        } else {
            write!(
                f,
                "{} cloned ({:.1} MiB shared), {} copied ({:.1} MiB)",
                self.cloned,
                mb(self.bytes_shared),
                self.copied,
                mb(self.bytes_copied)
            )
        }
    }
}

/// Clone a directory whole where the kernel can, walking it only to fix what it copied
/// verbatim: an absolute symlink into the source still points at the source.
///
/// When the tree contains excluded paths (other lanes, nested worktrees), walk and skip
/// them before any clonefile call — a whole-dir clone would copy those subtrees too.
/// Falls back to the per-file walk when the tree cannot be cloned in one call.
pub fn clone_dir_tree(
    src: &Path,
    dst: &Path,
    src_root: &Path,
    dst_root: &Path,
) -> std::io::Result<CloneStats> {
    clone_dir_tree_with_skip(src, dst, src_root, dst_root, &|_, _| false)
}

pub(crate) fn clone_dir_tree_with_skip(
    src: &Path,
    dst: &Path,
    src_root: &Path,
    dst_root: &Path,
    skip: &dyn Fn(&str, bool) -> bool,
) -> std::io::Result<CloneStats> {
    let must_walk = dst.starts_with(src)
        || fs::symlink_metadata(dst).is_ok()
        || dir_contains_excluded(src, src_root, skip)?;
    if must_walk {
        return clone_tree_rooted(
            src,
            dst,
            &|rel, is_dir| {
                skip(rel, is_dir) || should_skip_clone_path(src_root, &src_root.join(rel), is_dir)
            },
            src_root,
            dst_root,
        );
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    match clone_dir(src, dst) {
        Ok(()) => {}
        Err(CloneError::Unsupported(_) | CloneError::Exists) => {
            return clone_tree_rooted(
                src,
                dst,
                &|rel, is_dir| {
                    skip(rel, is_dir)
                        || should_skip_clone_path(src_root, &src_root.join(rel), is_dir)
                },
                src_root,
                dst_root,
            );
        }
        Err(CloneError::Io(e)) => return Err(e),
    }
    fixup(dst, src_root, dst_root)
}

fn dir_contains_excluded(
    dir: &Path,
    src_root: &Path,
    skip: &dyn Fn(&str, bool) -> bool,
) -> std::io::Result<bool> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let is_dir = entry.file_type()?.is_dir();
        let rel = path
            .strip_prefix(src_root)
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_default();
        if skip(&rel, is_dir) || should_skip_clone_path(src_root, &path, is_dir) {
            return Ok(true);
        }
        if is_dir && dir_contains_excluded(&path, src_root, skip)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Count what was cloned and repoint links the kernel copied verbatim.
fn visit(
    path: &Path,
    kind: fs::FileType,
    src_roots: &[PathBuf],
    dst_root: &Path,
    stats: &mut CloneStats,
) -> std::io::Result<()> {
    if kind.is_symlink() {
        stats.links += 1;
        let target = fs::read_link(path)?;
        if let Some(retargeted) = retarget(&target, src_roots, dst_root) {
            fs::remove_file(path)?;
            std::os::unix::fs::symlink(retargeted, path)?;
        }
    } else if kind.is_file() {
        stats.cloned += 1;
        stats.bytes_shared += fs::symlink_metadata(path).map(|m| m.len()).unwrap_or(0);
    }
    Ok(())
}

/// Walk the clone on every core: readdir over a large tree is the cost, not the clone.
fn fixup(dst: &Path, src_root: &Path, dst_root: &Path) -> std::io::Result<CloneStats> {
    let src_roots = root_spellings(src_root)?;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut stats = CloneStats::default();

    // Descend breadth-first until there are enough subtrees to spread across the threads.
    let mut level = vec![dst.to_path_buf()];
    while level.len() < threads * 4 {
        let mut next = Vec::new();
        for dir in &level {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    next.push(entry.path());
                } else {
                    visit(&entry.path(), kind, &src_roots, dst_root, &mut stats)?;
                }
            }
        }
        if next.is_empty() {
            level.clear();
            break;
        }
        level = next;
    }
    if level.is_empty() {
        return Ok(stats);
    }

    let chunk = level.len().div_ceil(threads);
    let parts: Vec<std::io::Result<CloneStats>> = std::thread::scope(|scope| {
        let handles: Vec<_> = level
            .chunks(chunk)
            .map(|chunk| {
                let src_roots = &src_roots;
                scope.spawn(move || {
                    let mut stats = CloneStats::default();
                    for dir in chunk {
                        for entry in walkdir::WalkDir::new(dir) {
                            let Ok(entry) = entry else { continue };
                            let kind = entry.file_type();
                            if !kind.is_dir() {
                                visit(entry.path(), kind, src_roots, dst_root, &mut stats)?;
                            }
                        }
                    }
                    Ok(stats)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for part in parts {
        let part = part?;
        stats.cloned += part.cloned;
        stats.copied += part.copied;
        stats.links += part.links;
        stats.bytes_shared += part.bytes_shared;
        stats.bytes_copied += part.bytes_copied;
    }
    Ok(stats)
}

/// Recursively clone `src` into `dst`; `skip(relpath, is_dir)` prunes.
pub fn clone_tree(
    src: &Path,
    dst: &Path,
    skip: &dyn Fn(&str, bool) -> bool,
) -> std::io::Result<CloneStats> {
    clone_tree_rooted(src, dst, skip, src, dst)
}

/// Clone `src` into `dst`, retargeting absolute links relative to the containing trees.
pub fn clone_tree_rooted(
    src: &Path,
    dst: &Path,
    skip: &dyn Fn(&str, bool) -> bool,
    src_root: &Path,
    dst_root: &Path,
) -> std::io::Result<CloneStats> {
    let mut stats = CloneStats::default();
    let src_roots = root_spellings(src_root)?;
    // A destination inside the source would be walked into as it is written. Skip it here so
    // no caller has to remember, and so renaming the lanes directory cannot reintroduce it.
    let contained = dst.strip_prefix(src).ok().map(Path::to_path_buf);
    let walker = walkdir::WalkDir::new(src).into_iter().filter_entry(|e| {
        let rel = match e.path().strip_prefix(src) {
            Ok(r) => r,
            Err(_) => return true,
        };
        // The walk root itself is never a candidate for skipping.
        rel.as_os_str().is_empty()
            || (!contained
                .as_ref()
                .is_some_and(|path| rel == path || rel.starts_with(path))
                && !should_skip_clone_path(src_root, e.path(), e.file_type().is_dir())
                && !skip(&rel.to_string_lossy(), e.file_type().is_dir()))
    });

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let rel = match entry.path().strip_prefix(src) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let target = dst.join(rel);

        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        // No `create_dir_all` and no existence check per file: the walk yields a directory
        // before its contents, so the parent is already made, and the create below reports
        // an existing destination for free. Both cost a syscall each, per file.
        if entry.file_type().is_symlink() {
            let dest = fs::read_link(entry.path())?;
            let dest = retarget(&dest, &src_roots, dst_root).unwrap_or(dest);
            match std::os::unix::fs::symlink(dest, &target) {
                Ok(()) => stats.links += 1,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        match clone_file(entry.path(), &target) {
            Ok(size) => {
                stats.cloned += 1;
                stats.bytes_shared += size;
            }
            Err(CloneError::Exists) => continue,
            Err(CloneError::Unsupported(_)) => {
                stats.bytes_copied += fs::copy(entry.path(), &target)?;
                stats.copied += 1;
            }
            Err(CloneError::Io(e)) => return Err(e),
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_filesystem_that_cannot_clone_falls_back_rather_than_failing() {
        // Darwin's clonefile returns ENOTSUP (45), not EOPNOTSUPP (102); treating it as a
        // real error made lane fail hard on HFS+, exFAT and network mounts.
        for code in UNSUPPORTED {
            let err = std::io::Error::from_raw_os_error(*code);
            assert!(
                matches!(classify(err), CloneError::Unsupported(_)),
                "errno {code} must mean fall back, not fail"
            );
        }
        // ENOSPC is a real failure and must not be swallowed.
        assert!(matches!(
            classify(std::io::Error::from_raw_os_error(28)),
            CloneError::Io(_)
        ));
    }

    fn write_nested_worktree(root: &Path, rel: &str, marker: &[u8]) {
        let wt = root.join(rel);
        fs::create_dir_all(wt.join("node_modules/.bun")).unwrap();
        fs::write(wt.join(".git"), "gitdir: /tmp/fake-worktree\n").unwrap();
        fs::write(wt.join("node_modules/.bun/sentinel"), marker).unwrap();
    }

    #[test]
    fn should_skip_lane_trees_and_nested_worktrees_only() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        write_nested_worktree(root, ".claude/worktrees/agenda-unify", b"nested");
        fs::create_dir_all(root.join(".lane/trees/sibling/node_modules")).unwrap();
        fs::write(root.join(".lane/trees/sibling/node_modules/cache"), b"x").unwrap();
        fs::create_dir_all(root.join("node_modules/own")).unwrap();
        fs::write(root.join("node_modules/own/pkg"), b"keep").unwrap();

        assert!(should_skip_clone_path(
            root,
            &root.join(".lane/trees"),
            true
        ));
        assert!(should_skip_clone_path(
            root,
            &root.join(".lane/trees/sibling"),
            true
        ));
        assert!(should_skip_clone_path(
            root,
            &root.join(".claude/worktrees/agenda-unify"),
            true
        ));
        assert!(!should_skip_clone_path(
            root,
            &root.join("node_modules"),
            true
        ));
        assert!(!should_skip_clone_path(root, &root.join(".claude"), true));
    }

    #[test]
    fn clone_dir_tree_excludes_nested_worktrees_and_lane_trees_before_clone() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = dst.path().join("out");
        let src = src.path().canonicalize().unwrap();

        fs::create_dir_all(src.join("node_modules/own/.bun")).unwrap();
        fs::write(src.join("node_modules/own/.bun/sentinel"), b"keep").unwrap();
        fs::write(src.join(".env"), b"SECRET=1").unwrap();
        write_nested_worktree(&src, ".claude/worktrees/agenda-unify", b"exclude-me");
        fs::create_dir_all(src.join(".lane/trees/sibling/node_modules")).unwrap();
        fs::write(
            src.join(".lane/trees/sibling/node_modules/cache"),
            b"exclude-lane",
        )
        .unwrap();
        std::os::unix::fs::symlink(src.join(".env"), src.join("env.link")).unwrap();

        let stats = clone_dir_tree(&src, &out, &src, &out).unwrap();

        assert_eq!(
            fs::read(out.join("node_modules/own/.bun/sentinel")).unwrap(),
            b"keep"
        );
        assert_eq!(fs::read(out.join(".env")).unwrap(), b"SECRET=1");
        assert_eq!(
            fs::read_link(out.join("env.link")).unwrap(),
            out.join(".env")
        );
        assert!(
            !out.join(".claude/worktrees/agenda-unify").exists(),
            "nested worktree must be excluded before clone dispatch"
        );
        assert!(
            !out.join(".lane/trees/sibling").exists(),
            "sibling lane trees must be excluded before clone dispatch"
        );
        assert!(stats.cloned + stats.copied >= 2);
        assert_eq!(stats.links, 1);
    }

    #[test]
    fn clone_tree_also_excludes_nested_worktrees() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let out = dst.path().join("out");
        let src = src.path().canonicalize().unwrap();

        fs::write(src.join("keep.txt"), b"yes").unwrap();
        write_nested_worktree(&src, ".claude/worktrees/other", b"no");

        clone_tree(&src, &out, &|_, _| false).unwrap();

        assert!(out.join("keep.txt").exists());
        assert!(!out.join(".claude/worktrees/other").exists());
    }
}
