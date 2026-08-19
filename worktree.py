# SPDX-License-Identifier: MIT
"""Lane lifecycle: create, list, done, remove.

A lane is a git worktree whose expensive contents arrive by reference instead
of by copy, plus the memory that gets distilled out of it when it closes.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path

from . import cow
from .memory import git, now_iso

LANES_DIRNAME = ".lanes"
WARM_DEFAULT = ["node_modules", "target", ".venv", "vendor", "dist",
                ".next", ".turbo", ".gradle", "build", ".cargo"]


def main_root() -> Path:
    """Root of the primary worktree, even when called from inside a lane."""
    common = Path(git("rev-parse", "--path-format=absolute", "--git-common-dir"))
    return common.parent


def in_lane() -> bool:
    git_path = Path(git("rev-parse", "--path-format=absolute", "--git-dir"))
    common = Path(git("rev-parse", "--path-format=absolute", "--git-common-dir"))
    return git_path != common


def trunk_name(root: Path) -> str:
    for cand in ("main", "master", "trunk"):
        if git("rev-parse", "--verify", "--quiet", cand, cwd=root, check=False):
            return cand
    return git("rev-parse", "--abbrev-ref", "HEAD", cwd=root)


def lanes_dir(root: Path) -> Path:
    return root.parent / (LANES_DIRNAME + "-" + root.name)


def is_dirty(path: Path) -> bool:
    return bool(git("status", "--porcelain", cwd=path, check=False).strip())


def tracked_set(root: Path) -> set:
    out = git("ls-files", "-z", cwd=root)
    return {p for p in out.split("\0") if p}


def list_lanes(root: Path):
    out = git("worktree", "list", "--porcelain", cwd=root)
    lanes, cur = [], {}
    for line in out.splitlines():
        if not line.strip():
            if cur:
                lanes.append(cur)
                cur = {}
            continue
        k, _, v = line.partition(" ")
        cur[k] = v
    if cur:
        lanes.append(cur)
    return [w for w in lanes if Path(w.get("worktree", "")) != root]


# --------------------------------------------------------------------------


def create(name: str, base: str = None, fork: bool = False, warm=None,
           quiet: bool = False):
    """Create a lane. Returns (path, CloneStats, notes).

    Two materialization strategies:

      default  git checks out tracked files; we clone only the untracked and
               ignored ones (node_modules, target, ...). Predictable, and the
               build cache is what actually costs time.

      --fork   nothing is checked out; the entire working tree is cloned by
               reference, uncommitted changes included, then the index is
               reconstructed without touching the files. Maximal sharing, and
               your dirty state comes along.
    """
    root = main_root()
    base = base or trunk_name(root)
    warm = warm if warm is not None else WARM_DEFAULT
    dest = lanes_dir(root) / name
    if dest.exists():
        raise RuntimeError("lane %s already exists at %s" % (name, dest))
    dest.parent.mkdir(parents=True, exist_ok=True)

    supported, detail = cow.probe(str(root))
    notes = ["reflink: %s (%s)" % ("yes" if supported else "no", detail)]

    if fork:
        git("worktree", "add", "--no-checkout", "-b", name, str(dest), base, cwd=root)
        skip = lambda rel, is_dir: rel == ".git" or rel.startswith(".git/")
        stats = cow.clone_tree(str(root), str(dest), skip=skip)
        # Rebuild the index from the base tree without rewriting the files,
        # then re-stat so unchanged files are not reported as modified.
        git("reset", "--mixed", "--quiet", base, cwd=dest)
        git("update-index", "--refresh", cwd=dest, check=False)
        carried = len(git("status", "--porcelain", cwd=dest, check=False).splitlines())
        if carried:
            notes.append("carried %d uncommitted change(s) from the parent tree" % carried)
    else:
        git("worktree", "add", "-b", name, str(dest), base, cwd=root)
        tracked = tracked_set(root)
        warm_set = set(warm)

        def skip(rel, is_dir):
            top = rel.split(os.sep)[0]
            if top == ".git":
                return True
            if is_dir:
                # descend only into warm dirs at the top level
                return os.sep not in rel and top not in warm_set
            return rel in tracked or top not in warm_set

        stats = cow.clone_tree(str(root), str(dest), skip=skip)

    return dest, stats, notes


def remove(name: str, force: bool = False):
    root = main_root()
    dest = lanes_dir(root) / name
    git("worktree", "remove", *(["--force"] if force else []), str(dest), cwd=root)
    git("branch", "-D", name, cwd=root, check=False)
    if dest.exists():
        shutil.rmtree(dest, ignore_errors=True)


def fast_forward(root: Path, trunk: str, branch: str):
    """Advance trunk to branch. Uses merge --ff-only when trunk is checked
    out in the main worktree, update-ref otherwise (it may be checked out
    nowhere, and git refuses to merge into a ref it is not on)."""
    head = git("rev-parse", "--abbrev-ref", "HEAD", cwd=root)
    target = git("rev-parse", branch, cwd=root)
    if head == trunk:
        git("merge", "--ff-only", branch, cwd=root)
    else:
        base = git("merge-base", trunk, branch, cwd=root)
        if base != git("rev-parse", trunk, cwd=root):
            raise RuntimeError(
                "trunk %s has diverged from %s; rebase first" % (trunk, branch))
        git("update-ref", "refs/heads/%s" % trunk, target, cwd=root)
