//! The `.context/` store: load, promote, check, evict, and the read ledger.

use crate::note::{self, Meta, Note};
use crate::syntax::{Source, Span};
use crate::util::{now_iso, slug, ulid};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const CONTEXT_DIR: &str = ".context";
pub const ATTIC: &str = ".attic";
pub const READS: &str = ".reads.jsonl";
pub const PENDING: &str = ".wt/pending.jsonl";

pub const FRESH: &str = "fresh";
pub const BODY: &str = "body-drift";
pub const SIG: &str = "signature-changed";
pub const MISSING: &str = "anchor-missing";

pub const TIERS: [&str; 4] = [FRESH, BODY, SIG, MISSING];

pub fn tier_rank(tier: &str) -> u8 {
    match tier {
        BODY => 1,
        SIG => 2,
        MISSING => 3,
        _ => 0,
    }
}

pub fn note_dir(root: &Path, path: &str) -> PathBuf {
    root.join(CONTEXT_DIR).join(path)
}

pub fn load_notes(root: &Path, filter: Option<&str>) -> Vec<Note> {
    let base = root.join(CONTEXT_DIR);
    if !base.exists() {
        return Vec::new();
    }
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(&base)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter(|p| {
            !p.strip_prefix(&base)
                .map(|r| {
                    r.components()
                        .next()
                        .is_some_and(|c| c.as_os_str() == ATTIC)
                })
                .unwrap_or(false)
        })
        .collect();
    files.sort();

    files
        .iter()
        .filter_map(|p| note::parse(p).ok())
        .filter(|n| filter.is_none_or(|f| n.meta.path == f))
        .collect()
}

pub fn bump_reads(root: &Path, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    use std::io::Write;
    let path = root.join(CONTEXT_DIR).join(READS);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for id in ids {
        writeln!(file, r#"{{"id":"{id}","at":"{}"}}"#, now_iso())?;
    }
    Ok(())
}

pub fn read_counts(root: &Path) -> HashMap<String, u32> {
    let mut counts = HashMap::new();
    let Ok(text) = std::fs::read_to_string(root.join(CONTEXT_DIR).join(READS)) else {
        return counts;
    };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(rec) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(id) = rec.get("id").and_then(|v| v.as_str())
        {
            *counts.entry(id.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

#[derive(Serialize, Deserialize)]
pub struct PendingNote {
    pub text: String,
    pub path: String,
    pub anchor: String,
    pub branch: String,
    pub at: String,
}

/// Pending notes are resolved and fingerprinted here, never at write time, so a rebase
/// can never leave a note anchored to a commit it rewrote.
pub fn promote_pending(root: &Path) -> Result<Vec<Note>> {
    let pending = root.join(PENDING);
    if !pending.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&pending)?;
    let mut created = Vec::new();

    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let rec: PendingNote = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("warning: skipping unreadable pending note: {e}");
                continue;
            }
        };
        let mut meta = Meta {
            id: ulid(),
            path: rec.path.clone(),
            anchor: rec.anchor.clone(),
            created: rec.at,
            branch: rec.branch,
            status: MISSING.into(),
            checked: now_iso(),
            ..Default::default()
        };
        if let Ok(body_text) = std::fs::read_to_string(root.join(&rec.path)) {
            let src = Source::new(&body_text, &rec.path);
            if let Some(span) = src.resolve(&rec.anchor) {
                let (sig, body_hash) = src.hashes(span, &rec.anchor);
                meta.sig = sig;
                meta.body_hash = body_hash;
                meta.lines = format!("{}-{}", span.start, span.end);
                meta.status = FRESH.into();
            }
        }
        let file =
            note_dir(root, &rec.path).join(format!("{}-{}.md", meta.id, slug(&rec.text, 28)));
        let mut note = Note::new(meta, rec.text);
        note.write(&file)?;
        note.file = Some(file);
        created.push(note);
    }

    std::fs::remove_file(&pending)?;
    Ok(created)
}

/// Never delete: the audit is the only writer here without a reviewer, so it stays inspectable.
pub fn evict(root: &Path, note: &mut Note, reason: &str) -> Result<()> {
    let Some(file) = note.file.clone() else {
        return Ok(());
    };
    let base = root.join(CONTEXT_DIR);
    let rel = file.strip_prefix(&base).unwrap_or(&file);
    let dest = base.join(ATTIC).join(rel);
    note.meta.evicted = format!("{} ({reason})", now_iso());
    note.write(&dest)?;
    std::fs::remove_file(&file)?;
    note.file = None;
    Ok(())
}

pub struct Check {
    pub tier: &'static str,
    pub sig: String,
    pub body_hash: String,
    pub span: Option<Span>,
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
        let anchor = note.meta.anchor.clone();
        let Some(src) = self.source(&note.meta.path) else {
            return String::new();
        };
        let Some(span) = src.resolve(&anchor) else {
            return String::new();
        };
        let text = src.span_text(span);
        text.chars().take(4000).collect()
    }

    pub fn check(&mut self, note: &Note) -> Check {
        let missing = |tier| Check {
            tier,
            sig: String::new(),
            body_hash: String::new(),
            span: None,
        };
        let anchor = note.meta.anchor.clone();
        let (want_sig, want_body) = (note.meta.sig.clone(), note.meta.body_hash.clone());

        let Some(src) = self.source(&note.meta.path) else {
            return missing(MISSING);
        };
        let Some(span) = src.resolve(&anchor) else {
            return missing(MISSING);
        };
        let (sig, body_hash) = src.hashes(span, &anchor);
        let tier = if sig != want_sig {
            SIG
        } else if body_hash != want_body {
            BODY
        } else {
            FRESH
        };
        Check {
            tier,
            sig,
            body_hash,
            span: Some(span),
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
    let path = root.join(PENDING);
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
    std::fs::read_to_string(worktree.join(PENDING))
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::rel_to_repo;

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
}
