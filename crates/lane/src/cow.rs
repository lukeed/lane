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
    Io(std::io::Error),
}

impl fmt::Display for CloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CloneError::Unsupported(why) => write!(f, "{why}"),
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

/// Clone one regular file by reference.
#[cfg(target_os = "macos")]
pub fn clone_file(src: &Path, dst: &Path) -> Result<(), CloneError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_src = CString::new(src.as_os_str().as_bytes())
        .map_err(|_| CloneError::Unsupported("path contains an interior NUL".into()))?;
    let c_dst = CString::new(dst.as_os_str().as_bytes())
        .map_err(|_| CloneError::Unsupported("path contains an interior NUL".into()))?;
    // CLONE_NOFOLLOW: clone the symlink itself, never its target.
    let rc = unsafe { libc::clonefile(c_src.as_ptr(), c_dst.as_ptr(), 1) };
    if rc == 0 {
        return Ok(());
    }
    Err(classify(std::io::Error::last_os_error()))
}

#[cfg(target_os = "linux")]
pub fn clone_file(src: &Path, dst: &Path) -> Result<(), CloneError> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let source = fs::File::open(src).map_err(CloneError::Io)?;
    let mode = source
        .metadata()
        .map_err(CloneError::Io)?
        .permissions()
        .mode();
    let target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(dst)
        .map_err(CloneError::Io)?;

    match rustix::fs::ioctl_ficlone(&target, &source) {
        Ok(()) => Ok(()),
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
    clone_file(src, dst)
}

#[cfg(not(target_os = "macos"))]
fn clone_dir(_src: &Path, _dst: &Path) -> Result<(), CloneError> {
    Err(CloneError::Unsupported(
        "no directory clone primitive on this platform".into(),
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn clone_file(_src: &Path, _dst: &Path) -> Result<(), CloneError> {
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
        Ok(()) => (true, "reflink available".into()),
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
/// Falls back to the per-file walk when the tree cannot be cloned in one call.
pub fn clone_dir_tree(
    src: &Path,
    dst: &Path,
    src_root: &Path,
    dst_root: &Path,
) -> std::io::Result<CloneStats> {
    let walk = || clone_tree_rooted(src, dst, &|_, _| false, src_root, dst_root);
    // clonefile refuses an existing destination, and a destination inside the source needs
    // the walk's pruning to avoid cloning the clone.
    if fs::symlink_metadata(dst).is_ok() || dst.starts_with(src) {
        return walk();
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    match clone_dir(src, dst) {
        Ok(()) => {}
        Err(CloneError::Unsupported(_)) => return walk(),
        Err(CloneError::Io(e)) => return Err(e),
    }

    let src_roots = root_spellings(src_root)?;
    let mut stats = CloneStats::default();
    for entry in walkdir::WalkDir::new(dst) {
        let Ok(entry) = entry else { continue };
        let kind = entry.file_type();
        if kind.is_dir() {
            continue;
        }
        if kind.is_symlink() {
            stats.links += 1;
            let target = fs::read_link(entry.path())?;
            if let Some(retargeted) = retarget(&target, &src_roots, dst_root) {
                fs::remove_file(entry.path())?;
                std::os::unix::fs::symlink(retargeted, entry.path())?;
            }
            continue;
        }
        if kind.is_file() {
            stats.cloned += 1;
            stats.bytes_shared += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
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
        if fs::symlink_metadata(&target).is_ok() {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if entry.file_type().is_symlink() {
            let dest = fs::read_link(entry.path())?;
            let dest = retarget(&dest, &src_roots, dst_root).unwrap_or(dest);
            std::os::unix::fs::symlink(dest, &target)?;
            stats.links += 1;
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        match clone_file(entry.path(), &target) {
            Ok(()) => {
                stats.cloned += 1;
                stats.bytes_shared += size;
            }
            Err(CloneError::Unsupported(_)) => {
                fs::copy(entry.path(), &target)?;
                stats.copied += 1;
                stats.bytes_copied += size;
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
}
