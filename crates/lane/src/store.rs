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
pub const LOG: &str = "log.jsonl";
pub const PENDING: &str = "lane/pending.jsonl";
pub const LANE_ID: &str = "lane/id";

/// Log record kinds. `holds` and `rebaseline` both move a baseline and are distinguished
/// so a machine's automatic re-anchor never reads as a person's vouch.
pub const HOLDS: &str = "holds";
pub const REBASELINE: &str = "rebaseline";
pub const LANDING: &str = "landing";
pub const EVICT: &str = "evict";

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
pub fn load_notes(root: &Path, filter: Option<&str>) -> Vec<Note> {
    let base = root.join(LANE_DIR).join(NOTES);
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
        .filter(|n| filter.is_none_or(|f| n.path() == f))
        .collect()
}

/// A re-confirmation of a note's baseline: the shape of the code that someone, or a
/// normalization change, last vouched for. Committed, so it survives a merge and a clone.
#[derive(Clone, Debug, Default)]
pub struct Confirmation {
    pub sig: String,
    pub body_hash: String,
    pub raw_hash: String,
    pub norm: String,
    pub at: String,
}

pub fn log_path(root: &Path) -> PathBuf {
    root.join(LANE_DIR).join(LOG)
}

fn log_lines(root: &Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(log_path(root)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn field(entry: &serde_json::Value, key: &str) -> String {
    entry
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The newest re-confirmation of each note. Union merge can interleave two branches'
/// appends, so `at` orders them and file position breaks a tie.
pub fn confirmations(root: &Path) -> HashMap<String, Confirmation> {
    let mut out: HashMap<String, Confirmation> = HashMap::new();
    for entry in log_lines(root) {
        let kind = field(&entry, "kind");
        if kind != HOLDS && kind != REBASELINE {
            continue;
        }
        let id = field(&entry, "id");
        if id.is_empty() {
            continue;
        }
        let fresh = Confirmation {
            sig: field(&entry, "sig"),
            body_hash: field(&entry, "body_hash"),
            raw_hash: field(&entry, "raw_hash"),
            norm: field(&entry, "norm"),
            at: field(&entry, "at"),
        };
        // A record with no fingerprint vouches for nothing. `check` reads an empty baseline
        // as a first fingerprint and reports fresh, so a truncated line would silence a
        // note forever; fall back to the creation fingerprint instead.
        if fresh.sig.is_empty() && fresh.body_hash.is_empty() {
            continue;
        }
        match out.get(&id) {
            Some(have) if have.at > fresh.at => {}
            _ => {
                out.insert(id, fresh);
            }
        }
    }
    out
}

/// Every landing writes one of these. Its presence in *trunk's* copy of the log is how a
/// merged branch is recognised, whichever way the merge was made: it is tree content, not
/// commit identity, so a squash cannot hide it the way it hides a SHA.
pub fn landings(log: &str) -> HashSet<String> {
    log.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| field(entry, "kind") == LANDING)
        .map(|entry| field(&entry, "lane"))
        .filter(|lane| !lane.is_empty())
        .collect()
}

/// Append-only, one file, so this is the one thing union merge is for. Every record
/// carries the branch that wrote it; nothing else records provenance now.
pub fn append_log(root: &Path, entry: &serde_json::Value) -> Result<()> {
    use std::io::Write;
    let mut entry = entry.clone();
    if let Some(map) = entry.as_object_mut() {
        map.entry("branch")
            .or_insert_with(|| git::current_branch().into());
    }
    let path = log_path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{entry}")?;
    Ok(())
}

/// A vouch, by a person or by a normalization change, recorded with the fingerprint it
/// confirmed. Without those four fields the record says a note was vouched for but not
/// for which shape of the code, which is the whole content of the decision.
pub fn append_confirmation(
    root: &Path,
    kind: &str,
    note: &Note,
    sig: &str,
    body_hash: &str,
    raw_hash: &str,
) -> Result<()> {
    append_log(
        root,
        &serde_json::json!({
            "at": now_iso(), "kind": kind, "id": note.meta.id,
            "path": note.path(), "anchor": note.meta.anchor,
            "sig": sig, "body_hash": body_hash, "raw_hash": raw_hash,
            "norm": crate::syntax::NORM_VERSION,
        }),
    )
}

#[derive(Serialize, Deserialize)]
pub struct PendingNote {
    pub text: String,
    pub path: String,
    pub anchor: String,
    pub branch: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub supersedes: String,
}

/// Pending notes are resolved and fingerprinted here, never at write time, so a rebase
/// can never leave a note anchored to a commit it rewrote.
pub fn promote_pending(root: &Path) -> Result<Vec<Note>> {
    let pending = pending_path(root);
    if !pending.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&pending)?;
    let mut created = Vec::new();
    let mut seen: HashSet<(String, String, String, String)> = load_notes(root, None)
        .into_iter()
        .map(|note| {
            (
                note.path(),
                note.meta.anchor,
                note.body.trim().to_string(),
                note.meta.supersedes,
            )
        })
        .collect();

    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let rec: PendingNote = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("warning: skipping unreadable pending note: {e}");
                continue;
            }
        };
        let key = (
            rec.path.clone(),
            rec.anchor.clone(),
            rec.text.trim().to_string(),
            rec.supersedes.clone(),
        );
        if seen.contains(&key) {
            continue;
        }
        let mut meta = Meta {
            id: ulid(),
            anchor: rec.anchor.clone(),
            created: rec.at,
            branch: rec.branch,
            norm: crate::syntax::NORM_VERSION.into(),
            supersedes: rec.supersedes.clone(),
            ..Default::default()
        };
        if let Ok(body_text) = std::fs::read_to_string(root.join(&rec.path)) {
            let src = Source::new(&body_text, &rec.path);
            if let Some(span) = src.resolve(&rec.anchor) {
                let (sig, body_hash, raw_hash) = src.hashes(span, &rec.anchor);
                meta.sig = sig;
                meta.body_hash = body_hash;
                meta.raw_hash = raw_hash;
                meta.lines = format!("{}-{}", span.start, span.end);
            }
        }
        let file =
            note_dir(root, &rec.path).join(format!("{}-{}.md", meta.id, slug(&rec.text, 28)));
        let mut note = Note::new(meta, rec.text);
        let mut predecessor = if rec.supersedes.is_empty() {
            None
        } else {
            Some(
                load_notes(root, None)
                    .into_iter()
                    .find(|old| old.meta.id == rec.supersedes)
                    .ok_or_else(|| anyhow::anyhow!("live note {} not found", rec.supersedes))?,
            )
        };
        note.write(&file)?;
        note.file = Some(file);
        if let Some(old) = predecessor.as_mut() {
            supersede(root, old, &note)?;
        }
        created.push(note);
        seen.insert(key);
    }

    std::fs::remove_file(&pending)?;
    Ok(created)
}

/// An id, or any unambiguous prefix of one. `lane why` prints ten characters of a
/// ULID, so what a reader can see has to be what the verbs accept.
pub fn resolve_id(root: &Path, id: &str) -> Result<Note> {
    let mut hits: Vec<Note> = load_notes(root, None)
        .into_iter()
        .filter(|note| note.meta.id.starts_with(id))
        .collect();
    match hits.len() {
        0 => anyhow::bail!("live note {id} not found"),
        1 => Ok(hits.remove(0)),
        n => {
            let shown: Vec<String> = hits
                .iter()
                .take(5)
                .map(|note| format!("{} {}#{}", note.meta.id, note.path(), note.meta.anchor))
                .collect();
            anyhow::bail!("{id} matches {n} notes:\n  {}", shown.join("\n  "))
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
pub fn evict(root: &Path, note: &mut Note, reason: &str) -> Result<()> {
    let Some(file) = note.file.clone() else {
        return Ok(());
    };
    let live = root.join(LANE_DIR).join(NOTES);
    let rel = file.strip_prefix(&live).unwrap_or(&file);
    let dest = root.join(LANE_DIR).join(ATTIC).join(rel);
    // A pure move: the note is retired, not rewritten. The reason goes to the log.
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&file, &dest)?;
    append_log(
        root,
        &serde_json::json!({
            "at": now_iso(), "kind": EVICT, "id": note.meta.id,
            "path": note.path(), "anchor": note.meta.anchor, "reason": reason,
        }),
    )?;
    note.file = Some(dest);
    Ok(())
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
    confirmed: HashMap<String, Confirmation>,
}

impl Checker {
    pub fn new(root: &Path) -> Self {
        Checker {
            root: root.to_path_buf(),
            cache: HashMap::new(),
            confirmed: confirmations(root),
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

        // The newest confirmation wins; the note's creation fingerprint is the fallback.
        // Both are committed, so every machine compares against the same baseline.
        let base = self
            .confirmed
            .get(&note.meta.id)
            .cloned()
            .unwrap_or(Confirmation {
                sig: note.meta.sig.clone(),
                body_hash: note.meta.body_hash.clone(),
                raw_hash: note.meta.raw_hash.clone(),
                norm: note.meta.norm.clone(),
                at: String::new(),
            });

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
                adopted: false,
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
    let abs = resolved(Path::new(path))?;
    let base = resolved(root)?;
    let rel = abs
        .strip_prefix(&base)
        .map_err(|_| anyhow::anyhow!("{path} is outside the repository"))?;
    if rel.as_os_str().is_empty() {
        anyhow::bail!("{path} is the repository root, not a file");
    }
    Ok(rel.to_string_lossy().to_string())
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

pub fn pending_count(worktree: &Path) -> usize {
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

    fn confirm(root: &Path, id: &str, at: &str, sig: &str, body_hash: &str, raw_hash: &str) {
        confirm_as(
            root,
            id,
            at,
            sig,
            body_hash,
            raw_hash,
            crate::syntax::NORM_VERSION,
        );
    }

    fn confirm_as(
        root: &Path,
        id: &str,
        at: &str,
        sig: &str,
        body_hash: &str,
        raw_hash: &str,
        norm: &str,
    ) {
        append_log(
            root,
            &serde_json::json!({
                "at": at, "kind": HOLDS, "id": id, "sig": sig,
                "body_hash": body_hash, "raw_hash": raw_hash, "norm": norm,
            }),
        )
        .unwrap();
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
        confirm(
            root.path(),
            "01M0A",
            "2026-01-01T00:00:00Z",
            &sig,
            &body_hash,
            &raw_hash,
        );

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
    fn a_fingerprintless_record_vouches_for_nothing() {
        let root = tempfile::tempdir().unwrap();
        append_log(
            root.path(),
            &serde_json::json!({"at": "2026-06-01T00:00:00Z", "kind": HOLDS, "id": "01M0A"}),
        )
        .unwrap();
        assert!(!confirmations(root.path()).contains_key("01M0A"));
    }

    #[test]
    fn confirmations_take_the_newest_record() {
        let root = tempfile::tempdir().unwrap();
        confirm(root.path(), "01M0A", "2026-01-01T00:00:00Z", "old", "", "");
        confirm(root.path(), "01M0A", "2026-06-01T00:00:00Z", "new", "", "");

        // Union merge interleaves two branches' appends, so `at` decides, not position.
        confirm(
            root.path(),
            "01M0A",
            "2026-03-01T00:00:00Z",
            "middle",
            "",
            "",
        );

        assert_eq!(confirmations(root.path())["01M0A"].sig, "new");
    }

    #[test]
    fn a_landing_marker_names_the_lane_not_the_branch() {
        let log = concat!(
            r#"{"kind":"evict","branch":"fix","lane":"01M0NOISE"}"#,
            "\n",
            r#"{"kind":"landing","branch":"fix","lane":"01M0FIRST"}"#,
            "\n",
            r#"{"kind":"landing","branch":"other"}"#,
            "\n",
        );
        let landed = landings(log);
        assert!(landed.contains("01M0FIRST"));
        // Branch names are reused; a second lane called `fix` has landed nothing.
        assert!(!landed.contains("fix"));
        assert!(!landed.contains("01M0NOISE"));
        // A marker with no lane id matches nothing rather than everything.
        assert!(!landed.contains(""));
    }

    #[test]
    fn every_record_carries_the_branch_that_wrote_it() {
        let root = tempfile::tempdir().unwrap();
        append_log(
            root.path(),
            &serde_json::json!({"kind": EVICT, "id": "01M0A"}),
        )
        .unwrap();
        let line = std::fs::read_to_string(log_path(root.path())).unwrap();
        let entry: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert!(!entry["branch"].as_str().unwrap_or_default().is_empty());
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

        confirm(
            root.path(),
            "01M0A",
            "2026-06-01T00:00:00Z",
            &sig,
            &body_hash,
            &raw_hash,
        );
        // The note's own fingerprint is empty, which would otherwise read as drift.
        assert_eq!(Checker::new(root.path()).check(&note).tier, FRESH);
    }

    #[test]
    fn an_old_normalization_rebaselines_silently_when_the_bytes_match() {
        let root = tempfile::tempdir().unwrap();
        let note = fixture(root.path(), "pub fn v() {}\n");
        let live = Source::new("pub fn v() {}\n", "src/a.rs");
        let span = live.resolve("@file").unwrap();
        let (_, _, raw_hash) = live.hashes(span, "@file");

        confirm_as(
            root.path(),
            "01M0A",
            "2026-06-01T00:00:00Z",
            "from-an-older-normalizer",
            "",
            &raw_hash,
            "0",
        );
        let res = Checker::new(root.path()).check(&note);
        assert_eq!(res.tier, FRESH);
        assert!(!res.rebaselined, "identical bytes cannot have drifted");
        assert!(res.adopted, "the new normalization must be recorded");
    }

    #[test]
    fn an_old_normalization_is_reported_when_the_bytes_moved() {
        let root = tempfile::tempdir().unwrap();
        let note = fixture(root.path(), "pub fn v() {}\n");
        confirm_as(
            root.path(),
            "01M0A",
            "2026-06-01T00:00:00Z",
            "from-an-older-normalizer",
            "",
            "different",
            "0",
        );
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
            branch: "main".into(),
            at: "2026-08-19T00:00:00Z".into(),
            supersedes: String::new(),
        };

        append_pending(root.path(), &pending).unwrap();
        assert_eq!(promote_pending(root.path()).unwrap().len(), 1);
        append_pending(root.path(), &pending).unwrap();
        assert!(promote_pending(root.path()).unwrap().is_empty());
        assert_eq!(load_notes(root.path(), Some("src/auth.rs")).len(), 1);
    }

    #[test]
    fn pending_supersede_links_and_attics_its_predecessor() {
        let root = tempfile::tempdir().unwrap();
        let old = fixture(root.path(), "pub fn v() {}\n");
        let branch = git::current_branch();
        append_pending(
            root.path(),
            &PendingNote {
                text: "replacement note".into(),
                path: "src/a.rs".into(),
                anchor: "@file".into(),
                branch,
                at: "2026-08-21T00:00:00Z".into(),
                supersedes: old.meta.id.clone(),
            },
        )
        .unwrap();

        let created = promote_pending(root.path()).unwrap();

        assert_eq!(created.len(), 1);
        assert_eq!(created[0].meta.supersedes, old.meta.id);
        assert_eq!(load_notes(root.path(), Some("src/a.rs")).len(), 1);
        assert!(
            attic_dir(root.path(), "src/a.rs")
                .join("01M0A-a-note.md")
                .is_file()
        );
        // The replacement inherits nothing: its own creation fingerprint is the baseline,
        // and the eviction is the only thing the log records.
        assert!(!confirmations(root.path()).contains_key(&created[0].meta.id));
        assert!(!confirmations(root.path()).contains_key(&old.meta.id));
    }
}
