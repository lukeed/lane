//! The `.lane/` store: load, promote, check, and evict.

use crate::git;
use crate::note::{self, Meta, Note};
use crate::syntax::{Resolution, Source, Span};
use crate::util::{now_iso, slug, ulid};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const LANE_DIR: &str = ".lane";
/// Reserved: everything under it mirrors user paths, so a repo may have its own attic/.
pub const NOTES: &str = "memory";
pub const ATTIC: &str = "attic";
pub const PENDING: &str = "lane/pending.jsonl";
pub const LANE_ID: &str = "lane/id";
pub const LANDED: &str = "lane/landed";

/// Per-worktree: git resolves an uncommon path inside .git/worktrees/<name> for a lane,
/// so a lane cannot inherit the queue its parent has not promoted yet.
pub fn pending_path(worktree: &Path) -> PathBuf {
    git::layout(worktree)
        .map(|layout| layout.git_dir.join(PENDING))
        .unwrap_or_else(|_| worktree.join(".git").join(PENDING))
}

/// A lane's own identity, stamped at creation and never committed.
///
/// Per-worktree, so it dies with the lane. A landing marker names the branch, and branch
/// names are reused — `fix` twice in a week is normal — so the name alone would make a
/// fresh lane look like the landed one it was named after.
pub fn lane_id(worktree: &Path) -> String {
    let Ok(layout) = git::layout(worktree) else {
        return String::new();
    };
    std::fs::read_to_string(layout.git_dir.join(LANE_ID))
        .unwrap_or_default()
        .trim()
        .into()
}

pub fn stamp_lane_id(worktree: &Path) -> Result<()> {
    let Ok(layout) = git::layout(worktree) else {
        return Ok(());
    };
    let path = layout.git_dir.join(LANE_ID);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{}\n", ulid()))?;
    Ok(())
}

/// A landing is local to this worktree. A new worktree reusing the same branch gets a
/// different git directory, so it cannot inherit this marker.
pub fn landed_path(worktree: &Path) -> Option<PathBuf> {
    git::layout(worktree)
        .ok()
        .map(|layout| layout.git_dir.join(LANDED))
}

/// Records the tip as well, so what arrives after a landing is countable. Call this once
/// the memory commit exists, or the tip names a commit the landing never included.
pub fn mark_landed(worktree: &Path) -> Result<()> {
    let Some(path) = landed_path(worktree) else {
        return Ok(());
    };
    let id = lane_id(worktree);
    if id.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tip = git::try_git(&["rev-parse", "HEAD"], Some(worktree));
    let mut line = format!("{id} {}", now_iso());
    if !tip.is_empty() {
        line.push(' ');
        line.push_str(&tip);
    }
    line.push('\n');
    std::fs::write(path, line)?;
    Ok(())
}

pub fn is_landed(worktree: &Path) -> bool {
    landed_path(worktree).is_some_and(|path| path.is_file())
}

/// The tip at landing time. `None` for a marker written before tips were recorded, and
/// that is never the same answer as "nothing followed the landing".
pub fn landed_tip(worktree: &Path) -> Option<String> {
    let path = landed_path(worktree)?;
    let text = std::fs::read_to_string(path).ok()?;
    let tip = text.split_whitespace().nth(2)?;
    Some(tip.to_string())
}

/// The whole file. Its span has no declaration line, so its first line is an
/// import or a shebang and means nothing on its own.
pub const WHOLE_FILE: &str = "@file";

pub const FRESH: &str = "fresh";
// The tier for a changed body_hash. The constant keeps the name of the hash it
// comes from; the string is what a reader sees.
pub const BODY: &str = "content-changed";
// The tier for a changed declaration line. Only anchors that have one can
// reach it; see WHOLE_FILE.
pub const SIG: &str = "contract-changed";
pub const MISSING: &str = "anchor-missing";
pub const UNVERIFIABLE: &str = "unverifiable";

pub const TIERS: [&str; 5] = [FRESH, BODY, SIG, MISSING, UNVERIFIABLE];

pub fn tier_rank(tier: &str) -> u8 {
    match tier {
        BODY => 1,
        SIG => 2,
        MISSING => 3,
        // Ranks with fresh: we have no evidence against it, and retention must not evict
        // on ignorance any more than the audit does.
        UNVERIFIABLE => 0,
        _ => 0,
    }
}

pub fn note_dir(root: &Path, path: &str) -> PathBuf {
    root.join(LANE_DIR).join(NOTES).join(path)
}

pub fn attic_dir(root: &Path, path: &str) -> PathBuf {
    root.join(LANE_DIR).join(ATTIC).join(path)
}

/// Live notes only; the attic is a sibling of the reserved tree, never inside it.
///
/// A filter holds the notes at that path and every note beneath it, so a directory
/// reads as the whole subtree.
pub fn load_notes(root: &Path, filter: Option<&str>) -> Vec<Note> {
    load_note_tree(root, NOTES, filter)
}

pub fn load_retired(root: &Path, filter: Option<&str>) -> Vec<Note> {
    load_note_tree(root, ATTIC, filter)
}

fn load_note_tree(root: &Path, tree: &str, filter: Option<&str>) -> Vec<Note> {
    let base = root.join(LANE_DIR).join(tree);
    if !base.exists() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(&base)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();

    files
        .iter()
        .filter_map(|p| note::parse(p).ok())
        .filter(|n| filter.is_none_or(|f| within(&n.path(), f)))
        .collect()
}

/// Whether a note path is the filter itself or sits under it. The boundary is a
/// whole path component, so `src` never claims `src-gen/lib.rs`.
pub fn within(path: &str, filter: &str) -> bool {
    path == filter
        || path
            .strip_prefix(filter)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn field(entry: &serde_json::Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Re-vouch a readable note by moving its committed baseline in place.
pub fn confirm(note: &mut Note, sig: &str, body_hash: &str, raw_hash: &str) -> Result<()> {
    write_baseline(note, sig, body_hash, raw_hash, Some(now_iso()))
}

/// Adopt a baseline after a normalization change without claiming a human vouched for it.
pub fn rebaseline(note: &mut Note, sig: &str, body_hash: &str, raw_hash: &str) -> Result<()> {
    write_baseline(note, sig, body_hash, raw_hash, None)
}

fn write_baseline(
    note: &mut Note,
    sig: &str,
    body_hash: &str,
    raw_hash: &str,
    vouched: Option<String>,
) -> Result<()> {
    if note.unreadable {
        anyhow::bail!(
            "cannot confirm note {}: frontmatter is unreadable",
            note.meta.id
        );
    }
    let file = note
        .file
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("cannot confirm note {}: no note file", note.meta.id))?;
    note.meta.sig = sig.into();
    note.meta.body_hash = body_hash.into();
    note.meta.raw_hash = raw_hash.into();
    note.meta.norm = crate::syntax::NORM_VERSION.into();
    if let Some(vouched) = vouched {
        note.meta.vouched = vouched;
    }
    note.write(file)
}

/// Fold the old shared log once. Newest records win; malformed and non-baseline records
/// are deliberately ignored, and the file goes with them. It is kept only when a note it
/// vouches for cannot be rewritten, because that vouch has no other copy.
pub fn fold_legacy_log(root: &Path) -> Result<()> {
    let legacy = "log.jsonl";
    let path = root.join(LANE_DIR).join(legacy);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut entries: Vec<(usize, serde_json::Value)> = text
        .lines()
        .enumerate()
        .filter_map(|(i, line)| serde_json::from_str(line).ok().map(|entry| (i, entry)))
        .collect();
    entries.sort_by(|(ia, a), (ib, b)| field(b, "at").cmp(&field(a, "at")).then(ib.cmp(ia)));
    let mut seen = HashSet::new();
    let mut stranded = false;
    for (_, entry) in entries {
        let kind = field(&entry, "kind");
        if kind != "holds" && kind != "rebaseline" {
            continue;
        }
        let id = field(&entry, "id");
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        let sig = field(&entry, "sig");
        let body_hash = field(&entry, "body_hash");
        if sig.is_empty() && body_hash.is_empty() {
            continue;
        }
        let Ok(mut note) = resolve_id(root, &id) else {
            continue;
        };
        // A vouch is a human judgment with no other copy, so a note we cannot rewrite keeps
        // the log rather than losing it. The fold retries on the next audit.
        if note.unreadable {
            eprintln!("warning: keeping {legacy}: note {id} has unreadable frontmatter");
            stranded = true;
            continue;
        }
        let at = field(&entry, "at");
        if kind == "holds" {
            write_baseline(
                &mut note,
                &sig,
                &body_hash,
                &field(&entry, "raw_hash"),
                Some(at),
            )?;
        } else {
            rebaseline(&mut note, &sig, &body_hash, &field(&entry, "raw_hash"))?;
        }
        if !field(&entry, "norm").is_empty() {
            note.meta.norm = field(&entry, "norm");
            note.write(note.file.as_deref().expect("parsed note has a file"))?;
        }
    }
    if stranded {
        return Ok(());
    }
    std::fs::remove_file(&path)?;
    let attrs = root.join(".gitattributes");
    if let Ok(text) = std::fs::read_to_string(&attrs) {
        let rule = format!(".lane/{legacy} merge=union");
        let kept: Vec<_> = text
            .lines()
            .filter(|line| *line != rule && !line.trim().is_empty())
            .collect();
        if kept.is_empty() {
            std::fs::remove_file(attrs)?;
        } else {
            std::fs::write(attrs, format!("{}\n", kept.join("\n")))?;
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct PendingNote {
    pub text: String,
    pub path: String,
    pub anchor: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub supersedes: String,
}

/// Record a finding as a note, in the tree, where the note lives.
///
/// No fingerprints. The anchor is baselined by the next audit, after any rebase has moved
/// what it points at; taking one here would measure content a rebase then rewrites. Returns
/// `None` when a live note already says exactly this.
pub fn write_note(root: &Path, rec: &PendingNote) -> Result<Option<Note>> {
    let key = (
        rec.path.clone(),
        rec.anchor.clone(),
        rec.text.trim().to_string(),
        rec.supersedes.clone(),
    );
    let live = load_notes(root, None);
    let known = live.iter().any(|note| {
        (
            note.path(),
            note.meta.anchor.clone(),
            note.body.trim().to_string(),
            note.meta.supersedes.clone(),
        ) == key
    });
    if known {
        return Ok(None);
    }

    let meta = Meta {
        id: ulid(),
        anchor: rec.anchor.clone(),
        created: rec.at.clone(),
        norm: crate::syntax::NORM_VERSION.into(),
        supersedes: rec.supersedes.clone(),
        ..Default::default()
    };
    let file = note_dir(root, &rec.path).join(format!("{}-{}.md", meta.id, slug(&rec.text, 28)));
    let mut note = Note::new(meta, rec.text.clone());

    let mut predecessor = if rec.supersedes.is_empty() {
        None
    } else {
        live.into_iter().find(|old| old.meta.id == rec.supersedes)
    };
    // Refusing here would lose the finding over a link. Keep the note, drop the link.
    if predecessor.is_none() && !note.meta.supersedes.is_empty() {
        eprintln!(
            "warning: note supersedes {}, which is not live; keeping it as a new note",
            note.meta.supersedes
        );
        note.meta.supersedes.clear();
    }

    note.write(&file)?;
    note.file = Some(file);
    if let Some(old) = predecessor.as_mut() {
        supersede(root, old, &note)?;
    }
    Ok(Some(note))
}

/// Fold a queue written before notes were written where they live. Idempotent, and a no-op
/// for the normal case of no queue at all. The file goes with the last record.
pub fn fold_pending(root: &Path) -> Result<Vec<Note>> {
    let pending = pending_path(root);
    let Ok(text) = std::fs::read_to_string(&pending) else {
        return Ok(Vec::new());
    };
    let mut created = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let rec: PendingNote = match serde_json::from_str(line) {
            Ok(rec) => rec,
            Err(err) => {
                eprintln!("warning: skipping unreadable pending note: {err}");
                continue;
            }
        };
        if let Some(note) = write_note(root, &rec)? {
            created.push(note);
        }
    }
    std::fs::remove_file(&pending)?;
    Ok(created)
}

/// An id, or any unambiguous prefix of one. `lane why` prints ten characters of a
/// ULID, so what a reader can see has to be what the verbs accept.
pub fn resolve_id(root: &Path, id: &str) -> Result<Note> {
    resolve_from(load_notes(root, None), id, "live")
}

pub fn resolve_retired_id(root: &Path, id: &str) -> Result<Note> {
    resolve_from(load_retired(root, None), id, "retired")
}

fn resolve_from(notes: Vec<Note>, id: &str, state: &str) -> Result<Note> {
    let mut hits: Vec<Note> = notes
        .into_iter()
        .filter(|note| note.meta.id.starts_with(id))
        .collect();
    match hits.len() {
        0 => anyhow::bail!("{state} note {id} not found"),
        1 => Ok(hits.remove(0)),
        n => {
            let shown: Vec<String> = hits
                .iter()
                .take(5)
                .map(|note| format!("{} {}#{}", note.meta.id, note.path(), note.meta.anchor))
                .collect();
            anyhow::bail!(
                "{state} note {id} matches {n} notes:\n  {}",
                shown.join("\n  ")
            )
        }
    }
}

/// The replacement carries its own creation fingerprint, so nothing is inherited: a
/// confirmation of the old id vouched for the old sentence, not this one.
pub fn supersede(root: &Path, old: &mut Note, fresh: &Note) -> Result<()> {
    let reason = format!("superseded by {}", fresh.meta.id);
    evict(root, old, &reason)
}

/// Never delete: audit moves notes to the attic so every retirement stays inspectable.
pub fn evict(root: &Path, note: &mut Note, _reason: &str) -> Result<()> {
    move_note(root, note, NOTES, ATTIC)
}

pub fn restore(root: &Path, note: &mut Note) -> Result<()> {
    move_note(root, note, ATTIC, NOTES)
}

fn move_note(root: &Path, note: &mut Note, from: &str, to: &str) -> Result<()> {
    let Some(file) = note.file.clone() else {
        anyhow::bail!("cannot move note {}: no note file", note.meta.id);
    };
    let source = root.join(LANE_DIR).join(from);
    let rel = file.strip_prefix(&source).map_err(|_| {
        anyhow::anyhow!(
            "cannot move note {}: file is outside the {from} tree",
            note.meta.id
        )
    })?;
    let dest = root.join(LANE_DIR).join(to).join(rel);
    // A pure move: the note is retired, not rewritten.
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        anyhow::bail!(
            "cannot move note {}: {} already exists",
            note.meta.id,
            dest.display()
        );
    }
    std::fs::rename(&file, &dest)?;
    note.file = Some(dest);
    Ok(())
}

pub fn set_pinned(note: &mut Note, pinned: bool) -> Result<bool> {
    if note.unreadable {
        anyhow::bail!(
            "cannot {} note {}: frontmatter is unreadable",
            if pinned { "pin" } else { "unpin" },
            note.meta.id
        );
    }
    let file = note.file.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "cannot {} note {}: no note file",
            if pinned { "pin" } else { "unpin" },
            note.meta.id
        )
    })?;
    if note.meta.pinned == pinned {
        return Ok(false);
    }
    note.meta.pinned = pinned;
    note.write(file)?;
    Ok(true)
}

pub struct Check {
    pub tier: &'static str,
    pub sig: String,
    pub body_hash: String,
    pub raw_hash: String,
    /// The fingerprint this check compared against.
    pub base: (String, String, String),
    pub span: Option<Span>,
    /// The baseline was taken under a different normalization and could not be compared.
    pub rebaselined: bool,
    /// The baseline's normalization differed, comparably or not, so this run adopts a new
    /// one. Frontmatter is immutable, so without a record the note re-adopts forever.
    pub adopted: bool,
}

/// Parses each file once and reuses it across every note anchored to it.
pub struct Checker {
    root: PathBuf,
    cache: HashMap<String, Option<Source>>,
}

impl Checker {
    pub fn new(root: &Path) -> Self {
        Checker {
            root: root.to_path_buf(),
            cache: HashMap::new(),
        }
    }

    pub fn source(&mut self, rel: &str) -> Option<&Source> {
        if !self.cache.contains_key(rel) {
            let parsed = std::fs::read_to_string(self.root.join(rel))
                .ok()
                .map(|text| Source::new(&text, rel));
            self.cache.insert(rel.to_string(), parsed);
        }
        self.cache.get(rel).and_then(|slot| slot.as_ref())
    }

    pub fn span_text(&mut self, note: &Note) -> String {
        let (path, anchor) = (note.path(), note.meta.anchor.clone());
        let Some(src) = self.source(&path) else {
            return String::new();
        };
        let Some(span) = src.resolve(&anchor) else {
            return String::new();
        };
        let text = src.span_text(span);
        text.chars().take(4000).collect()
    }

    pub fn check(&mut self, note: &Note) -> Check {
        let blank = |tier| Check {
            tier,
            sig: String::new(),
            body_hash: String::new(),
            raw_hash: String::new(),
            base: (
                note.meta.sig.clone(),
                note.meta.body_hash.clone(),
                note.meta.raw_hash.clone(),
            ),
            span: None,
            rebaselined: false,
            adopted: false,
        };
        // Nothing is known about a note we could not read, so nothing may act on it.
        if note.unreadable {
            return blank(FRESH);
        }

        let base = note.meta.clone();

        let (path, anchor) = (note.path(), note.meta.anchor.clone());
        let Some(src) = self.source(&path) else {
            return blank(MISSING);
        };
        let span = match src.resolve_detail(&anchor) {
            Resolution::Found(span) => span,
            Resolution::NotFound => return blank(MISSING),
            Resolution::Unparsed => return blank(UNVERIFIABLE),
        };
        let (sig, body_hash, raw_hash) = src.hashes(span, &anchor);

        // Nothing to compare against yet, so this is a first fingerprint, not a change.
        // Adopted, because a note written before its baseline exists keeps none otherwise.
        if base.sig.is_empty() && base.body_hash.is_empty() {
            return Check {
                tier: FRESH,
                sig,
                body_hash,
                raw_hash,
                base: (
                    base.sig.clone(),
                    base.body_hash.clone(),
                    base.raw_hash.clone(),
                ),
                span: Some(span),
                rebaselined: false,
                adopted: true,
            };
        }

        // A baseline from a different normalization is not comparable. Identical bytes mean
        // drift is impossible, so adopt silently; otherwise adopt and say so.
        if base.norm != crate::syntax::NORM_VERSION {
            let unchanged = !base.raw_hash.is_empty() && base.raw_hash == raw_hash;
            return Check {
                tier: FRESH,
                sig,
                body_hash,
                raw_hash,
                base: (
                    base.sig.clone(),
                    base.body_hash.clone(),
                    base.raw_hash.clone(),
                ),
                span: Some(span),
                rebaselined: !unchanged,
                adopted: true,
            };
        }

        // SIG means the declaration line moved. @file has no declaration, so its
        // first line is an import or a shebang: promoting a change there above a
        // rewrite of everything under it would rank the file exactly backwards.
        let declared = note.meta.anchor != WHOLE_FILE;
        let tier = if declared && sig != base.sig {
            SIG
        } else if body_hash != base.body_hash || sig != base.sig {
            BODY
        } else {
            FRESH
        };
        Check {
            tier,
            sig,
            body_hash,
            raw_hash,
            base: (base.sig, base.body_hash, base.raw_hash),
            span: Some(span),
            rebaselined: false,
            adopted: false,
        }
    }
}

/// Absolute and free of `..`, resolving symlinks when the path exists.
///
/// `std::path::absolute` keeps `..` components, which `strip_prefix` then happily accepts.
fn resolved(path: &Path) -> Result<PathBuf> {
    if let Ok(real) = path.canonicalize() {
        return Ok(real);
    }
    let mut out = PathBuf::new();
    for part in std::path::absolute(path)?.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    Ok(out)
}

/// Repo-relative path, refusing anything that would land outside the store.
pub fn rel_to_repo(root: &Path, path: &str) -> Result<String> {
    rel_scope(root, path)?
        .ok_or_else(|| anyhow::anyhow!("{path} is the repository root, not a file"))
}

/// The same path for a read that accepts a directory, where the repository root is
/// `None` because it holds everything and filters nothing.
pub fn rel_scope(root: &Path, path: &str) -> Result<Option<String>> {
    let abs = resolved(Path::new(path))?;
    let base = resolved(root)?;
    let rel = abs
        .strip_prefix(&base)
        .map_err(|_| anyhow::anyhow!("{path} is outside the repository"))?;
    if rel.as_os_str().is_empty() {
        return Ok(None);
    }
    Ok(Some(rel.to_string_lossy().to_string()))
}

pub fn append_pending(root: &Path, rec: &PendingNote) -> Result<()> {
    use std::io::Write;
    let path = pending_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(rec)?)?;
    Ok(())
}

/// Notes this worktree holds and nobody else does: uncommitted, whether new or re-vouched.
pub fn pending_count(worktree: &Path) -> usize {
    let dir = format!("{LANE_DIR}/{NOTES}");
    crate::git::try_git(
        &["status", "--porcelain", "--untracked-files=all", "--", &dir],
        Some(worktree),
    )
    .lines()
    .filter(|line| line.trim_end().ends_with(".md"))
    .count()
        + fold_count(worktree)
}

/// A queue not yet folded still holds findings, and they are this worktree's alone.
fn fold_count(worktree: &Path) -> usize {
    std::fs::read_to_string(pending_path(worktree))
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// Move a path's notes to follow a renamed source file. Returns how many moved.
pub fn move_notes(root: &Path, old: &str, new: &str) -> Result<usize> {
    let from = note_dir(root, old);
    if !from.is_dir() {
        return Ok(0);
    }
    let to = note_dir(root, new);
    std::fs::create_dir_all(&to)?;

    let mut moved = 0;
    for entry in std::fs::read_dir(&from)? {
        let path = entry?.path();
        if path.extension().is_none_or(|x| x != "md") {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        // Per file, not the directory, so an existing destination is merged not clobbered.
        let dest = to.join(name);
        std::fs::rename(&path, &dest)?;
        moved += 1;
    }
    // Only prunes when nothing else was in there.
    let _ = std::fs::remove_dir(&from);
    Ok(moved)
}

/// Whether any note points at a file that is not there; the only reason to ask git
/// about renames.
pub fn has_missing_sources(root: &Path, notes: &[Note]) -> bool {
    notes.iter().any(|n| !root.join(n.path()).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_note(root: &Path, path: &str, id: &str, body: &str) {
        let note = crate::note::Note::new(
            crate::note::Meta {
                id: id.into(),
                anchor: "@file".into(),
                ..Default::default()
            },
            body,
        );
        note.write(&note_dir(root, path).join(format!("{id}-x.md")))
            .unwrap();
    }

    fn confirm(root: &Path, id: &str, sig: &str, body_hash: &str, raw_hash: &str) {
        let mut note = resolve_id(root, id).unwrap();
        super::confirm(&mut note, sig, body_hash, raw_hash).unwrap();
    }

    #[test]
    fn a_whole_file_note_never_reports_a_contract_change() {
        let root = tempfile::tempdir().unwrap();
        let src = "use std::io;\n\nfn verify(t: &str) -> bool {\n    eq(t)\n}\n";
        std::fs::write(root.path().join("lib.rs"), src).unwrap();
        let live = crate::syntax::Source::new(src, "lib.rs");
        let span = live.resolve(WHOLE_FILE).unwrap();
        let (sig, body_hash, raw_hash) = live.hashes(span, WHOLE_FILE);
        seed_note(
            root.path(),
            "lib.rs",
            "01M0A",
            "keep this file dependency-free",
        );
        confirm(root.path(), "01M0A", &sig, &body_hash, &raw_hash);

        // line one of a file is an import; changing it is not a contract change
        std::fs::write(
            root.path().join("lib.rs"),
            "use std::fmt;\n\nfn verify(t: &str) -> bool {\n    eq(t)\n}\n",
        )
        .unwrap();
        let note = load_notes(root.path(), None).into_iter().next().unwrap();
        assert_eq!(Checker::new(root.path()).check(&note).tier, BODY);
    }

    #[test]
    fn resolve_id_takes_a_prefix_and_refuses_an_ambiguous_one() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/a.rs", "01M0AKEEP", "keep");
        seed_note(root.path(), "src/b.rs", "01M0BDROP", "drop");

        // what `lane check` and `lane why` print is a prefix, so it has to work
        assert_eq!(
            resolve_id(root.path(), "01M0AK").unwrap().meta.id,
            "01M0AKEEP"
        );
        assert_eq!(
            resolve_id(root.path(), "01M0AKEEP").unwrap().meta.id,
            "01M0AKEEP"
        );

        let ambiguous = resolve_id(root.path(), "01M0").unwrap_err().to_string();
        assert!(ambiguous.contains("matches 2 notes"), "{ambiguous}");
        assert!(ambiguous.contains("01M0AKEEP"), "{ambiguous}");

        let unknown = resolve_id(root.path(), "ZZZ").unwrap_err().to_string();
        assert!(unknown.contains("not found"), "{unknown}");
    }

    #[test]
    fn live_resolution_excludes_the_attic() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/a.rs", "01M0ALIVE", "keep");
        let mut note = resolve_id(root.path(), "01M0A").unwrap();
        evict(root.path(), &mut note, "test").unwrap();

        assert!(resolve_id(root.path(), "01M0A").is_err());
        assert_eq!(
            resolve_retired_id(root.path(), "01M0A").unwrap().meta.id,
            "01M0ALIVE"
        );
    }

    #[test]
    fn retired_resolution_excludes_live_memory() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/a.rs", "01M0ALIVE", "keep");

        assert!(resolve_retired_id(root.path(), "01M0A").is_err());
        assert_eq!(
            resolve_id(root.path(), "01M0A").unwrap().meta.id,
            "01M0ALIVE"
        );
    }

    #[test]
    fn retire_and_restore_preserve_exact_bytes_and_refuse_collisions() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/a.rs", "01M0A", "keep");
        let mut note = resolve_id(root.path(), "01M0A").unwrap();
        let live = note.file.clone().unwrap();
        let bytes = std::fs::read(&live).unwrap();

        evict(root.path(), &mut note, "test").unwrap();
        let retired = note.file.clone().unwrap();
        assert_eq!(std::fs::read(&retired).unwrap(), bytes);
        restore(root.path(), &mut note).unwrap();
        assert_eq!(note.file.as_deref(), Some(live.as_path()));
        assert_eq!(std::fs::read(&live).unwrap(), bytes);

        std::fs::create_dir_all(retired.parent().unwrap()).unwrap();
        std::fs::write(&retired, b"collision").unwrap();
        assert!(evict(root.path(), &mut note, "test").is_err());
        assert_eq!(std::fs::read(&live).unwrap(), bytes);
        assert_eq!(std::fs::read(&retired).unwrap(), b"collision");
    }

    #[test]
    fn ambiguous_prefixes_are_local_to_each_state() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/live.rs", "01M0ALIVE1", "live");
        seed_note(root.path(), "src/old.rs", "01M0AOLD1", "old");
        let mut old = resolve_id(root.path(), "01M0AOLD1").unwrap();
        evict(root.path(), &mut old, "test").unwrap();

        assert_eq!(
            resolve_id(root.path(), "01M0A").unwrap().meta.id,
            "01M0ALIVE1"
        );
        assert_eq!(
            resolve_retired_id(root.path(), "01M0A").unwrap().meta.id,
            "01M0AOLD1"
        );

        seed_note(root.path(), "src/live2.rs", "01M0ALIVE2", "live two");
        seed_note(root.path(), "src/old2.rs", "01M0AOLD2", "old two");
        let mut old_two = resolve_id(root.path(), "01M0AOLD2").unwrap();
        evict(root.path(), &mut old_two, "test").unwrap();

        let live = resolve_id(root.path(), "01M0A").unwrap_err().to_string();
        assert!(
            live.starts_with("live note 01M0A matches 2 notes:"),
            "{live}"
        );
        let retired = resolve_retired_id(root.path(), "01M0A")
            .unwrap_err()
            .to_string();
        assert!(
            retired.starts_with("retired note 01M0A matches 2 notes:"),
            "{retired}"
        );
    }

    #[test]
    fn pin_and_unpin_render_correctly_and_are_idempotent() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/a.rs", "01M0A", "keep");
        let mut note = resolve_id(root.path(), "01M0A").unwrap();
        let file = note.file.clone().unwrap();

        assert!(set_pinned(&mut note, true).unwrap());
        let pinned = std::fs::read_to_string(&file).unwrap();
        assert!(pinned.contains("pinned: true"));
        assert!(!set_pinned(&mut note, true).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), pinned);

        assert!(set_pinned(&mut note, false).unwrap());
        assert!(!std::fs::read_to_string(&file).unwrap().contains("pinned:"));
        assert!(!set_pinned(&mut note, false).unwrap());
    }

    #[test]
    fn unreadable_notes_refuse_pin_mutations() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/a.rs", "01M0A", "keep");
        let file = resolve_id(root.path(), "01M0A").unwrap().file.unwrap();
        let damaged = std::fs::read_to_string(&file).unwrap().replacen(
            "anchor:",
            "anchor: first\nanchor:",
            1,
        );
        std::fs::write(&file, &damaged).unwrap();
        let mut note = resolve_id(root.path(), "01M0A").unwrap();

        assert!(note.unreadable);
        assert!(set_pinned(&mut note, true).is_err());
        assert!(set_pinned(&mut note, false).is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), damaged);
    }

    #[test]
    fn a_note_re_vouch_updates_its_own_baseline_once() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/a.rs", "01M0A", "keep");
        let before = resolve_id(root.path(), "01M0A").unwrap();
        assert!(before.meta.vouched.is_empty());
        confirm(root.path(), "01M0A", "first", "body", "raw");
        let once = resolve_id(root.path(), "01M0A").unwrap();
        assert_eq!(once.meta.sig, "first");
        assert!(!once.meta.vouched.is_empty());
        assert_eq!(once.meta.created, before.meta.created);
        confirm(root.path(), "01M0A", "second", "body2", "raw2");
        let twice = resolve_id(root.path(), "01M0A").unwrap();
        assert_eq!(twice.meta.sig, "second");
        assert_eq!(twice.meta.created, before.meta.created);
    }

    /// A note plus a source file, ready to check.
    fn fixture(root: &Path, body: &str) -> Note {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), body).unwrap();
        let note = Note::new(
            Meta {
                id: "01M0A".into(),
                anchor: "@file".into(),
                norm: crate::syntax::NORM_VERSION.into(),
                ..Default::default()
            },
            "a note",
        );
        let file = note_dir(root, "src/a.rs").join("01M0A-a-note.md");
        note.write(&file).unwrap();
        note::parse(&file).unwrap()
    }

    #[test]
    fn a_confirmation_beats_a_stale_creation_fingerprint() {
        let root = tempfile::tempdir().unwrap();
        let note = fixture(root.path(), "pub fn v() {}\n");
        let live = Source::new("pub fn v() {}\n", "src/a.rs");
        let span = live.resolve("@file").unwrap();
        let (sig, body_hash, raw_hash) = live.hashes(span, "@file");

        confirm(root.path(), "01M0A", &sig, &body_hash, &raw_hash);
        // The note's own fingerprint is empty, which would otherwise read as drift.
        assert_eq!(Checker::new(root.path()).check(&note).tier, FRESH);
    }

    #[test]
    fn an_old_normalization_rebaselines_silently_when_the_bytes_match() {
        let root = tempfile::tempdir().unwrap();
        let _note = fixture(root.path(), "pub fn v() {}\n");
        let live = Source::new("pub fn v() {}\n", "src/a.rs");
        let span = live.resolve("@file").unwrap();
        let (_, _, raw_hash) = live.hashes(span, "@file");

        confirm(
            root.path(),
            "01M0A",
            "from-an-older-normalizer",
            "",
            &raw_hash,
        );
        let mut note = resolve_id(root.path(), "01M0A").unwrap();
        note.meta.norm = "0".into();
        note.write(note.file.as_deref().unwrap()).unwrap();
        let res = Checker::new(root.path()).check(&note);
        assert_eq!(res.tier, FRESH);
        assert!(!res.rebaselined, "identical bytes cannot have drifted");
        assert!(res.adopted, "the new normalization must be recorded");
    }

    #[test]
    fn an_old_normalization_is_reported_when_the_bytes_moved() {
        let root = tempfile::tempdir().unwrap();
        let _note = fixture(root.path(), "pub fn v() {}\n");
        confirm(
            root.path(),
            "01M0A",
            "from-an-older-normalizer",
            "",
            "different",
        );
        let mut note = resolve_id(root.path(), "01M0A").unwrap();
        note.meta.norm = "0".into();
        note.write(note.file.as_deref().unwrap()).unwrap();
        let res = Checker::new(root.path()).check(&note);
        assert_eq!(res.tier, FRESH);
        assert!(res.rebaselined, "an unresolvable baseline must be reported");
        assert!(res.adopted);
    }

    #[test]
    fn a_first_fingerprint_is_not_drift() {
        // A note made when the language had no grammar has no baseline; adding one later
        // must not report every such note as contract-changed.
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/a.rs"), "pub fn v() {}\n").unwrap();
        let note = Note::new(
            Meta {
                id: "01M0A".into(),
                anchor: "fn v".into(),
                norm: crate::syntax::NORM_VERSION.into(),
                ..Default::default()
            },
            "a note",
        );
        let file = note_dir(root.path(), "src/a.rs").join("01M0A-a-note.md");
        note.write(&file).unwrap();
        let note = note::parse(&file).unwrap();

        let res = Checker::new(root.path()).check(&note);
        assert_eq!(res.tier, FRESH);
        assert!(!res.sig.is_empty(), "the fingerprint must be adopted");
    }

    #[test]
    fn an_unverifiable_note_is_not_the_first_evicted() {
        // Retention ranks it with fresh, not below drift.
        assert_eq!(tier_rank(UNVERIFIABLE), tier_rank(FRESH));
        assert!(tier_rank(UNVERIFIABLE) < tier_rank(BODY));
    }

    #[test]
    fn an_unparsed_anchor_is_unverifiable() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/Auth.swift"), "func verify() {}\n").unwrap();
        let note = Note::new(
            Meta {
                id: "01M0A".into(),
                anchor: "func verify".into(),
                norm: crate::syntax::NORM_VERSION.into(),
                ..Default::default()
            },
            "a note",
        );
        let file = note_dir(root.path(), "src/Auth.swift").join("01M0A-a-note.md");
        note.write(&file).unwrap();
        let note = note::parse(&file).unwrap();

        assert_eq!(Checker::new(root.path()).check(&note).tier, UNVERIFIABLE);
    }

    #[test]
    fn move_notes_follows_a_rename_and_rewrites_the_path() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/auth.rs", "01M0A", "constant time");

        let moved = move_notes(root.path(), "src/auth.rs", "src/token.rs").unwrap();
        assert_eq!(moved, 1);
        assert!(!note_dir(root.path(), "src/auth.rs").exists());

        let notes = load_notes(root.path(), Some("src/token.rs"));
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].path(), "src/token.rs");
        assert_eq!(notes[0].body.trim(), "constant time");
    }

    #[test]
    fn move_notes_merges_into_an_existing_destination() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/auth.rs", "01M0A", "from the old path");
        seed_note(
            root.path(),
            "src/token.rs",
            "01M0B",
            "already at the new path",
        );

        assert_eq!(
            move_notes(root.path(), "src/auth.rs", "src/token.rs").unwrap(),
            1
        );
        assert_eq!(load_notes(root.path(), Some("src/token.rs")).len(), 2);
    }

    #[test]
    fn a_directory_filter_holds_every_note_beneath_it() {
        let root = tempfile::tempdir().unwrap();
        seed_note(root.path(), "src/auth.rs", "01M0A", "at the top");
        seed_note(root.path(), "src/db/pool.rs", "01M0B", "further down");
        seed_note(root.path(), "src-gen/lib.rs", "01M0C", "a different tree");
        seed_note(root.path(), "docs/api.md", "01M0D", "elsewhere");

        let paths = |filter: &str| {
            let mut found: Vec<String> = load_notes(root.path(), Some(filter))
                .iter()
                .map(Note::path)
                .collect();
            found.sort();
            found
        };
        assert_eq!(paths("src"), ["src/auth.rs", "src/db/pool.rs"]);
        assert_eq!(paths("src/db"), ["src/db/pool.rs"]);
        assert_eq!(paths("src/auth.rs"), ["src/auth.rs"]);
        assert!(paths("src/au").is_empty());
    }

    #[test]
    fn the_repository_root_scopes_to_everything() {
        let root = tempfile::tempdir().unwrap();
        let inside = root.path().join("src");
        std::fs::create_dir_all(&inside).unwrap();

        assert_eq!(
            rel_scope(root.path(), root.path().to_str().unwrap()).unwrap(),
            None
        );
        assert_eq!(
            rel_scope(root.path(), inside.to_str().unwrap()).unwrap(),
            Some("src".into())
        );
        assert!(rel_to_repo(root.path(), root.path().to_str().unwrap()).is_err());
    }

    #[test]
    fn a_path_outside_the_repo_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().parent().unwrap().join("escape.txt");
        std::fs::write(&outside, b"x").unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/a.rs"), b"x").unwrap();

        assert!(rel_to_repo(root.path(), "../escape.txt").is_err());
        assert!(rel_to_repo(root.path(), outside.to_str().unwrap()).is_err());
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn a_path_inside_the_repo_is_kept_relative() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/a.rs"), b"x").unwrap();
        let inside = root.path().join("src/a.rs");
        assert_eq!(
            rel_to_repo(root.path(), inside.to_str().unwrap()).unwrap(),
            "src/a.rs"
        );
    }

    #[test]
    fn promoting_the_same_pending_note_twice_creates_one_note() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/auth.rs"), "pub fn verify() {}\n").unwrap();
        let pending = PendingNote {
            text: "early return leaks token length".into(),
            path: "src/auth.rs".into(),
            anchor: "fn verify".into(),
            at: "2026-08-19T00:00:00Z".into(),
            supersedes: String::new(),
        };

        assert!(write_note(root.path(), &pending).unwrap().is_some());
        assert!(
            write_note(root.path(), &pending).unwrap().is_none(),
            "the same finding twice is one note"
        );
        assert_eq!(load_notes(root.path(), Some("src/auth.rs")).len(), 1);
    }

    #[test]
    fn pending_supersede_links_and_attics_its_predecessor() {
        let root = tempfile::tempdir().unwrap();
        let old = fixture(root.path(), "pub fn v() {}\n");
        let created = write_note(
            root.path(),
            &PendingNote {
                text: "replacement note".into(),
                path: "src/a.rs".into(),
                anchor: "@file".into(),
                at: "2026-08-21T00:00:00Z".into(),
                supersedes: old.meta.id.clone(),
            },
        )
        .unwrap();

        let created = created.expect("the replacement is written");
        assert_eq!(created.meta.supersedes, old.meta.id);
        assert_eq!(load_notes(root.path(), Some("src/a.rs")).len(), 1);
        assert!(
            attic_dir(root.path(), "src/a.rs")
                .join("01M0A-a-note.md")
                .is_file()
        );
        // The replacement inherits nothing, and carries no baseline until an audit takes one.
        assert!(created.meta.vouched.is_empty());
        assert!(created.meta.sig.is_empty());
    }
}
