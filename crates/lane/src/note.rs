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
    pub path: String,
    pub anchor: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sig: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lines: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub checked: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reviewed: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verdict: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub supersedes: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub evicted: String,
}

#[derive(Clone, Debug)]
pub struct Note {
    pub meta: Meta,
    pub body: String,
    pub file: Option<PathBuf>,
}

impl Note {
    pub fn new(meta: Meta, body: impl Into<String>) -> Self {
        Note {
            meta,
            body: body.into(),
            file: None,
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
}

/// A note we cannot parse still has to be visible, so fall back to the whole file as body.
pub fn parse(path: &Path) -> Result<Note> {
    let raw = std::fs::read_to_string(path)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let fallback = || Note {
        meta: Meta {
            id: stem.clone(),
            ..Default::default()
        },
        body: raw.clone(),
        file: Some(path.to_path_buf()),
    };

    let Some(rest) = raw.strip_prefix("---\n") else {
        return Ok(fallback());
    };
    let Some(end) = rest.find("\n---") else {
        return Ok(fallback());
    };
    let (front, tail) = rest.split_at(end);
    let body = tail.trim_start_matches("\n---").trim_start_matches('\n');

    match serde_yaml_ng::from_str::<Meta>(front) {
        Ok(meta) => Ok(Note {
            meta,
            body: body.to_string(),
            file: Some(path.to_path_buf()),
        }),
        Err(e) => {
            eprintln!(
                "warning: {} has unreadable frontmatter: {e}",
                path.display()
            );
            Ok(fallback())
        }
    }
}
