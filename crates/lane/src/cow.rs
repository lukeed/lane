//! Copy-on-write cloning through the kernel primitives, so a silent full copy is detectable.
//!
//!   Linux  FICLONE ioctl   btrfs, XFS(reflink=1), bcachefs, some ZFS
//!   macOS  clonefile(2)    APFS
//!   else   byte copy

use std::fmt;
use std::fs;
use std::path::Path;

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

/// Recursively clone `src` into `dst`; `skip(relpath, is_dir)` prunes.
pub fn clone_tree(
    src: &Path,
    dst: &Path,
    skip: &dyn Fn(&str, bool) -> bool,
) -> std::io::Result<CloneStats> {
    let mut stats = CloneStats::default();
    let walker = walkdir::WalkDir::new(src).into_iter().filter_entry(|e| {
        let rel = match e.path().strip_prefix(src) {
            Ok(r) => r,
            Err(_) => return true,
        };
        // The walk root itself is never a candidate for skipping.
        rel.as_os_str().is_empty() || !skip(&rel.to_string_lossy(), e.file_type().is_dir())
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
