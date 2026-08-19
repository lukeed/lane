# SPDX-License-Identifier: MIT
"""Copy-on-write file cloning.

Uses the kernel primitives directly rather than shelling out to `cp
--reflink`, because we need to know per-file whether we got real sharing or a
silent full copy, and cp will not tell us.

  Linux  FICLONE ioctl        btrfs, XFS(reflink=1), bcachefs, some ZFS
  macOS  clonefile(2)         APFS
  else   fallback full copy
"""

from __future__ import annotations

import ctypes
import errno
import fcntl
import os
import platform
import shutil
import stat
import tempfile

# _IOW(0x94, 9, int)
FICLONE = 0x40049409

_clonefile = None
if platform.system() == "Darwin":
    try:
        _libc = ctypes.CDLL("libSystem.dylib", use_errno=True)
        _clonefile = _libc.clonefile
        _clonefile.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint32]
        _clonefile.restype = ctypes.c_int
    except OSError:
        _clonefile = None


class CloneUnsupported(Exception):
    pass


def clone_file(src: str, dst: str) -> None:
    """Clone one regular file by reference. Raises CloneUnsupported if the
    filesystem cannot do it, so the caller can decide about fallback."""
    if _clonefile is not None:
        if _clonefile(src.encode(), dst.encode(), 1) == 0:  # CLONE_NOFOLLOW
            return
        raise CloneUnsupported(os.strerror(ctypes.get_errno()))

    try:
        with open(src, "rb") as s:
            flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
            fd = os.open(dst, flags, stat.S_IMODE(os.fstat(s.fileno()).st_mode))
            try:
                fcntl.ioctl(fd, FICLONE, s.fileno())
            finally:
                os.close(fd)
    except OSError as e:
        if os.path.exists(dst):
            try:
                os.unlink(dst)
            except OSError:
                pass
        if e.errno in (errno.EOPNOTSUPP, errno.ENOTTY, errno.EXDEV,
                       errno.EINVAL, errno.EPERM):
            raise CloneUnsupported(os.strerror(e.errno))
        raise


def probe(path: str) -> tuple:
    """(supported, detail) for the filesystem holding `path`."""
    d = tempfile.mkdtemp(dir=path)
    a, b = os.path.join(d, "a"), os.path.join(d, "b")
    try:
        with open(a, "wb") as f:
            f.write(b"probe" * 1024)
        try:
            clone_file(a, b)
            return (True, "reflink available")
        except CloneUnsupported as e:
            return (False, str(e))
    finally:
        shutil.rmtree(d, ignore_errors=True)


class CloneStats:
    def __init__(self):
        self.cloned = 0
        self.copied = 0
        self.links = 0
        self.bytes_shared = 0
        self.bytes_copied = 0

    def __str__(self):
        mb = lambda n: n / (1024.0 * 1024.0)
        if self.copied == 0 and self.cloned:
            return "%d files cloned (%.1f MiB shared, 0 copied)" % (
                self.cloned, mb(self.bytes_shared))
        return "%d cloned (%.1f MiB shared), %d copied (%.1f MiB)" % (
            self.cloned, mb(self.bytes_shared), self.copied, mb(self.bytes_copied))


def clone_tree(src: str, dst: str, skip=None, stats: CloneStats = None) -> CloneStats:
    """Recursively clone src into dst. `skip(relpath, is_dir) -> bool`.

    Falls back to a byte copy per file, so a partially-capable filesystem
    still produces a correct tree — just a more expensive one.
    """
    stats = stats or CloneStats()
    skip = skip or (lambda rel, is_dir: False)

    for dirpath, dirnames, filenames in os.walk(src):
        rel_dir = os.path.relpath(dirpath, src)
        rel_dir = "" if rel_dir == "." else rel_dir

        dirnames[:] = [
            d for d in dirnames
            if not skip(os.path.join(rel_dir, d) if rel_dir else d, True)
        ]

        target_dir = os.path.join(dst, rel_dir) if rel_dir else dst
        os.makedirs(target_dir, exist_ok=True)

        for name in filenames:
            rel = os.path.join(rel_dir, name) if rel_dir else name
            if skip(rel, False):
                continue
            s = os.path.join(dirpath, name)
            t = os.path.join(dst, rel)
            if os.path.lexists(t):
                continue

            if os.path.islink(s):
                os.symlink(os.readlink(s), t)
                stats.links += 1
                continue
            if not os.path.isfile(s):
                continue

            size = os.path.getsize(s)
            try:
                clone_file(s, t)
                stats.cloned += 1
                stats.bytes_shared += size
            except CloneUnsupported:
                shutil.copy2(s, t)
                stats.copied += 1
                stats.bytes_copied += size
    return stats
