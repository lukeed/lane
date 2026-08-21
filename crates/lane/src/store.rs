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
pub const BRANCH: &str = "branch";
pub const PENDING: &str = "lane/pending.jsonl";

/// Per-worktree: git resolves an uncommon path inside .git/worktrees/<name> for a lane,
/// so a lane cannot inherit the queue its parent has not promoted yet.
pub fn pending_path(worktree: &Path) -> PathBuf {
    let resolved = git::try_git(
        &["rev-parse", "--path-format=absolute", "--git-path", PENDING],
        Some(worktree),
    );
    if resolved.is_empty() {
        worktree.join(".git").join(PENDING)
    } else {
        PathBuf::from(resolved)
    }
}

pub const FRESH: &str = "fresh";
pub const BODY: &str = "body-drift";
pub const SIG: &str = "signature-changed";
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

/// Per-note derived state. Disposable: losing it costs one recompute, never a wrong answer.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct NoteState {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sig: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checked: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub norm: String,
}

pub type State = HashMap<String, NoteState>;

fn branch_dir(root: &Path, branch: &str) -> PathBuf {
    root.join(LANE_DIR).join(BRANCH).join(slug(branch, 60))
}

fn state_file_for(root: &Path, branch: &str) -> PathBuf {
    branch_dir(root, branch).join("state.json")
}

fn log_file_for(root: &Path, branch: &str) -> PathBuf {
    branch_dir(root, branch).join("log.jsonl")
}

fn state_file(root: &Path) -> PathBuf {
    state_file_for(root, &git::current_branch())
}

fn read_state_file(path: &Path) -> State {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn all_state(root: &Path) -> Vec<State> {
    let dir = root.join(LANE_DIR).join(BRANCH);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path().join("state.json"))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files.iter().map(|p| read_state_file(p)).collect()
}

/// Newest `checked` wins. Order-independent, and a wrong pick costs one recheck.
pub fn load_state(root: &Path) -> State {
    let mut merged = State::new();
    for part in all_state(root) {
        for (id, entry) in part {
            match merged.get(&id) {
                Some(have) if have.checked >= entry.checked => {}
                _ => {
                    merged.insert(id, entry);
                }
            }
        }
    }
    merged
}

fn write_state_file(path: &Path, state: &State) -> Result<()> {
    let mut sorted: Vec<(&String, &NoteState)> = state.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let ordered: std::collections::BTreeMap<&String, &NoteState> = sorted.into_iter().collect();
    let text = serde_json::to_string_pretty(&ordered)? + "\n";

    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    tmp.write_all(text.as_bytes())?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Write this branch's file, and only if it changed.
pub fn save_state(root: &Path, state: &State) -> Result<()> {
    write_state_file(&state_file(root), state)
}

/// Append-only, one file per branch, so this is the one thing union merge is for.
pub fn append_log(root: &Path, entry: &serde_json::Value) -> Result<()> {
    use std::io::Write;
    let path = log_file_for(root, &git::current_branch());
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
    let mut state = own_state(root);

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
            supersede(root, old, &note, &mut state)?;
        }
        created.push(note);
        seen.insert(key);
    }

    save_state(root, &state)?;
    std::fs::remove_file(&pending)?;
    Ok(created)
}

pub fn supersede(root: &Path, old: &mut Note, fresh: &Note, state: &mut State) -> Result<()> {
    if let Some(entry) = state.get(&old.meta.id).cloned() {
        state.insert(fresh.meta.id.clone(), entry);
    }
    let reason = format!("superseded by {}", fresh.meta.id);
    evict(root, old, &reason)?;
    state.remove(&old.meta.id);
    Ok(())
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
            "at": now_iso(), "kind": "evict", "id": note.meta.id,
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
}

/// Parses each file once and reuses it across every note anchored to it.
pub struct Checker {
    root: PathBuf,
    cache: HashMap<String, Option<Source>>,
    state: State,
}

impl Checker {
    pub fn new(root: &Path) -> Self {
        Checker {
            root: root.to_path_buf(),
            cache: HashMap::new(),
            state: load_state(root),
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
        };
        // Nothing is known about a note we could not read, so nothing may act on it.
        if note.unreadable {
            return blank(FRESH);
        }

        // The newest confirmation wins; the note's creation fingerprint is the fallback.
        let base = self.state.get(&note.meta.id).cloned().unwrap_or(NoteState {
            sig: note.meta.sig.clone(),
            body_hash: note.meta.body_hash.clone(),
            raw_hash: note.meta.raw_hash.clone(),
            norm: note.meta.norm.clone(),
            ..Default::default()
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
            };
        }

        let tier = if sig != base.sig {
            SIG
        } else if body_hash != base.body_hash {
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

/// This branch's file alone; the read counts live here and must survive an audit.
pub fn own_state(root: &Path) -> State {
    read_state_file(&state_file(root))
}

/// Fold a lane's per-branch directory into its target and delete its own.
///
/// Safe because `done` rebases before it audits, so lanes serialise; without this the
/// store accumulates a file per lane forever.
pub fn roll_up(root: &Path, from: &str, into: &str) -> Result<()> {
    let (from_log, into_log) = (log_file_for(root, from), log_file_for(root, into));
    if let Ok(lines) = std::fs::read_to_string(&from_log) {
        use std::io::Write;
        if let Some(parent) = into_log.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&into_log)?;
        write!(file, "{lines}")?;
    }

    let from_state = state_file_for(root, from);
    if from_state.exists() {
        let mine = read_state_file(&from_state);
        let into_path = state_file_for(root, into);
        let mut theirs = read_state_file(&into_path);
        for (id, entry) in mine {
            let merged = match theirs.remove(&id) {
                Some(have) => {
                    if have.checked >= entry.checked {
                        have
                    } else {
                        entry
                    }
                }
                None => entry,
            };
            theirs.insert(id, merged);
        }
        write_state_file(&into_path, &theirs)?;
    }
    let from_dir = branch_dir(root, from);
    if from_dir.exists() {
        std::fs::remove_dir_all(from_dir)?;
    }
    Ok(())
}

/// Drop a branch's files outright; used when its work is discarded rather than landed.
pub fn discard_branch_files(root: &Path, branch: &str) {
    let _ = std::fs::remove_dir_all(branch_dir(root, branch));
}

/// Remove caches for branches that no longer exist. Only ever state, never the log.
pub fn gc_state(root: &Path) {
    let live: std::collections::HashSet<String> = git::try_git(
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        None,
    )
    .lines()
    .map(|b| slug(b, 60))
    .collect();
    let dir = root.join(LANE_DIR).join(BRANCH);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().map(|s| s.to_string_lossy().to_string());
        if name.is_some_and(|s| !live.contains(&s)) {
            let _ = std::fs::remove_file(path.join("state.json"));
        }
    }
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

    fn write_state(root: &Path, branch: &str, id: &str, entry: NoteState) {
        let path = state_file_for(root, branch);
        let mut existing = read_state_file(&path);
        existing.insert(id.to_string(), entry);
        write_state_file(&path, &existing).unwrap();
    }

    #[test]
    fn writing_identical_state_does_not_touch_the_file() {
        let root = tempfile::tempdir().unwrap();
        let path = state_file_for(root.path(), "main");
        let state = State::from([(
            "01M0A".into(),
            NoteState {
                sig: "same".into(),
                ..Default::default()
            },
        )]);

        write_state_file(&path, &state).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        write_state_file(&path, &state).unwrap();

        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before
        );
    }

    #[test]
    fn writing_changed_state_replaces_it_completely() {
        let root = tempfile::tempdir().unwrap();
        let path = state_file_for(root.path(), "main");
        let first = State::from([(
            "01M0A".into(),
            NoteState {
                sig: "old".into(),
                ..Default::default()
            },
        )]);
        let changed = State::from([(
            "01M0A".into(),
            NoteState {
                sig: "new".into(),
                ..Default::default()
            },
        )]);

        write_state_file(&path, &first).unwrap();
        write_state_file(&path, &changed).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            serde_json::from_str::<State>(&text).unwrap()["01M0A"].sig,
            "new"
        );
        assert!(!text.contains("old"));
    }

    #[test]
    fn writing_state_replaces_invalid_json() {
        let root = tempfile::tempdir().unwrap();
        let path = state_file_for(root.path(), "main");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{invalid old bytes").unwrap();
        let state = State::from([(
            "01M0A".into(),
            NoteState {
                sig: "good".into(),
                ..Default::default()
            },
        )]);

        write_state_file(&path, &state).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(serde_json::from_str::<State>(&text).is_ok());
        assert!(!text.contains("invalid old bytes"));
    }

    #[test]
    fn load_state_takes_the_newest_confirmation() {
        let root = tempfile::tempdir().unwrap();
        write_state(
            root.path(),
            "main",
            "01M0A",
            NoteState {
                sig: "old".into(),
                checked: "2026-01-01T00:00:00Z".into(),
                ..Default::default()
            },
        );
        write_state(
            root.path(),
            "lane-x",
            "01M0A",
            NoteState {
                sig: "new".into(),
                checked: "2026-06-01T00:00:00Z".into(),
                ..Default::default()
            },
        );
        assert_eq!(load_state(root.path())["01M0A"].sig, "new");
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
    fn a_state_entry_beats_a_stale_creation_fingerprint() {
        let root = tempfile::tempdir().unwrap();
        let note = fixture(root.path(), "pub fn v() {}\n");
        let live = Source::new("pub fn v() {}\n", "src/a.rs");
        let span = live.resolve("@file").unwrap();
        let (sig, body_hash, raw_hash) = live.hashes(span, "@file");

        write_state(
            root.path(),
            "main",
            "01M0A",
            NoteState {
                sig,
                body_hash,
                raw_hash,
                checked: "2026-06-01T00:00:00Z".into(),
                norm: crate::syntax::NORM_VERSION.into(),
                ..Default::default()
            },
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

        write_state(
            root.path(),
            "main",
            "01M0A",
            NoteState {
                sig: "from-an-older-normalizer".into(),
                raw_hash,
                checked: "2026-06-01T00:00:00Z".into(),
                norm: "0".into(),
                ..Default::default()
            },
        );
        let res = Checker::new(root.path()).check(&note);
        assert_eq!(res.tier, FRESH);
        assert!(!res.rebaselined, "identical bytes cannot have drifted");
    }

    #[test]
    fn an_old_normalization_is_reported_when_the_bytes_moved() {
        let root = tempfile::tempdir().unwrap();
        let note = fixture(root.path(), "pub fn v() {}\n");
        write_state(
            root.path(),
            "main",
            "01M0A",
            NoteState {
                sig: "from-an-older-normalizer".into(),
                raw_hash: "different".into(),
                checked: "2026-06-01T00:00:00Z".into(),
                norm: "0".into(),
                ..Default::default()
            },
        );
        let res = Checker::new(root.path()).check(&note);
        assert_eq!(res.tier, FRESH);
        assert!(res.rebaselined, "an unresolvable baseline must be reported");
    }

    #[test]
    fn a_first_fingerprint_is_not_drift() {
        // A note made when the language had no grammar has no baseline; adding one later
        // must not report every such note as signature-changed.
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
        write_state(root.path(), &branch, &old.meta.id, NoteState::default());
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
        let state = own_state(root.path());
        assert!(state.contains_key(&created[0].meta.id));
        assert!(!state.contains_key(&old.meta.id));
    }
}
