//! One note, one file. Frontmatter goes through a real YAML serializer so values escape themselves.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Meta {
    pub id: String,
    pub anchor: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub norm: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sig: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vouched: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lines: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub supersedes: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
}

#[derive(Clone, Debug)]
pub struct Note {
    pub meta: Meta,
    pub body: String,
    pub file: Option<PathBuf>,
    /// The bytes this was parsed from, so audit can skip a write that changes nothing.
    pub raw: String,
    /// Frontmatter did not parse; we recovered what we could and must not rewrite it.
    pub unreadable: bool,
}

impl Note {
    /// The directory is the path: one source of truth, and a rename is a pure file move.
    pub fn path(&self) -> String {
        self.file
            .as_deref()
            .map(path_from_location)
            .unwrap_or_default()
    }

    pub fn new(meta: Meta, body: impl Into<String>) -> Self {
        Note {
            meta,
            body: body.into(),
            file: None,
            raw: String::new(),
            unreadable: false,
        }
    }

    pub fn render(&self) -> String {
        let yaml = serde_yaml_ng::to_string(&self.meta).unwrap_or_default();
        format!("---\n{}---\n\n{}\n", yaml, self.body.trim())
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.render())?;
        Ok(())
    }

    /// Remove provenance written by older Lane versions without disturbing any other
    /// frontmatter, including fields this binary may not know about yet.
    pub fn strip_legacy_branch(&mut self) -> Result<bool> {
        if self.unreadable {
            return Ok(false);
        }
        let Some(rest) = self.raw.strip_prefix("---\n") else {
            return Ok(false);
        };
        let Some(end) = rest.find("\n---") else {
            return Ok(false);
        };
        let front = &rest[..end];
        if !front.lines().any(|line| line.starts_with("branch:")) {
            return Ok(false);
        }
        let kept = front
            .lines()
            .filter(|line| !line.starts_with("branch:"))
            .collect::<Vec<_>>()
            .join("\n");
        let rewritten = format!("---\n{kept}{}", &rest[end..]);
        let file = self
            .file
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("note {} has no file", self.meta.id))?;
        std::fs::write(file, &rewritten)?;
        self.raw = rewritten;
        Ok(true)
    }
}

/// A note we cannot parse still has to be visible, so fall back to the whole file as body.
/// `.lane/memory/<path>/<ulid>-<slug>.md`, so the directory names the file the note is about.
fn path_from_location(file: &Path) -> String {
    let parts: Vec<String> = file
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .map(|part| part.as_os_str().to_string_lossy().to_string())
        .collect();
    let Some(store) = parts
        .iter()
        .rposition(|name| name == crate::store::LANE_DIR)
    else {
        return String::new();
    };
    // Exactly one component after the deepest .lane is ours; the rest is the user's path.
    parts
        .into_iter()
        .skip(store + 2)
        .collect::<Vec<_>>()
        .join("/")
}

pub fn parse(path: &Path) -> Result<Note> {
    let raw = std::fs::read_to_string(path)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // Recover what the filename and directory still tell us, so a note we cannot read is
    // still reported by name rather than as `#`.
    let recovered = || Meta {
        id: stem.split('-').next().unwrap_or(&stem).to_string(),
        ..Default::default()
    };
    let damaged = |body: String| Note {
        meta: recovered(),
        body,
        file: Some(path.to_path_buf()),
        raw: raw.clone(),
        unreadable: true,
    };

    let Some(rest) = raw.strip_prefix("---\n") else {
        return Ok(damaged(raw.clone()));
    };
    let Some(end) = rest.find("\n---") else {
        return Ok(damaged(raw.clone()));
    };
    let (front, tail) = rest.split_at(end);
    let body = tail
        .trim_start_matches("\n---")
        .trim_start_matches('\n')
        .to_string();

    match serde_yaml_ng::from_str::<Meta>(front) {
        Ok(meta) => Ok(Note {
            meta,
            body,
            file: Some(path.to_path_buf()),
            raw: raw.clone(),
            unreadable: false,
        }),
        Err(e) => {
            eprintln!(
                "warning: {} has unreadable frontmatter: {e}",
                path.display()
            );
            Ok(damaged(body))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_note(root: &Path, rel: &str, body: &str) -> PathBuf {
        let dir = crate::store::note_dir(root, rel);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("01M0AAAAAAAAAAAAAAAAAAAAAA-seed.md");
        let note = Note::new(
            Meta {
                id: "01M0AAAAAAAAAAAAAAAAAAAAAA".into(),
                anchor: "fn verify".into(),
                created: "2026-08-19T00:00:00Z".into(),
                ..Default::default()
            },
            body,
        );
        note.write(&file).unwrap();
        file
    }

    #[test]
    fn a_note_we_wrote_round_trips_to_the_same_bytes() {
        // This is what lets audit skip a write, and so what keeps merges clean.
        let root = tempfile::tempdir().unwrap();
        let file = write_note(root.path(), "src/auth.rs", "seed: constant time");
        let parsed = parse(&file).unwrap();
        assert!(!parsed.unreadable);
        assert_eq!(parsed.render(), parsed.raw);
    }

    #[test]
    fn legacy_branch_is_removed_without_losing_unknown_frontmatter() {
        let root = tempfile::tempdir().unwrap();
        let file = write_note(root.path(), "src/auth.rs", "seed: constant time");
        let text = std::fs::read_to_string(&file).unwrap().replacen(
            "created: 2026-08-19T00:00:00Z\n",
            "created: 2026-08-19T00:00:00Z\nbranch: fix-login\nfuture: preserved\n",
            1,
        );
        std::fs::write(&file, text).unwrap();

        let mut parsed = parse(&file).unwrap();
        assert!(parsed.strip_legacy_branch().unwrap());
        let migrated = std::fs::read_to_string(file).unwrap();
        assert!(!migrated.contains("branch:"));
        assert!(migrated.contains("future: preserved"));
        assert!(migrated.ends_with("seed: constant time\n"));
    }

    #[test]
    fn a_damaged_note_keeps_its_identity_and_body() {
        let root = tempfile::tempdir().unwrap();
        let file = write_note(root.path(), "src/auth.rs", "seed: constant time");
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(
            &file,
            text.replacen("created:", "created: 2099-01-01T00:00:00Z\ncreated:", 1),
        )
        .unwrap();

        let parsed = parse(&file).unwrap();
        assert!(parsed.unreadable, "duplicate keys must not parse silently");
        assert_eq!(parsed.path(), "src/auth.rs");
        assert_eq!(parsed.meta.id, "01M0AAAAAAAAAAAAAAAAAAAAAA");
        assert!(parsed.body.contains("seed: constant time"));
    }

    #[test]
    fn a_notes_path_comes_from_its_directory_attic_or_not() {
        let root = Path::new("/repo");
        let live = root.join(".lane/memory/src/auth.rs/01M0-x.md");
        let attic = root.join(".lane/attic/src/auth.rs/01M0-x.md");
        assert_eq!(path_from_location(&live), "src/auth.rs");
        assert_eq!(path_from_location(&attic), "src/auth.rs");
        // A repo may have its own attic/, and only our own leading component is dropped.
        let user_attic = root.join(".lane/memory/attic/f.txt/01M0-x.md");
        assert_eq!(path_from_location(&user_attic), "attic/f.txt");
    }

    #[test]
    fn a_lane_side_note_uses_the_deepest_store_root() {
        let file = Path::new("/repo/.lane/trees/work/.lane/memory/src/auth.rs/01M0-x.md");

        assert_eq!(path_from_location(file), "src/auth.rs");
    }
}
