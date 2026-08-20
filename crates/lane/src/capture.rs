//! Commit-trailer capture into the pending note queue.

use crate::git::{self, git};
use crate::store;
use crate::syntax::{Resolution, Source};
use crate::util::now_iso;
use anyhow::Result;
use std::collections::HashSet;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug)]
pub struct Captured {
    pub path: String,
    pub anchor: String,
    pub text: String,
}

/// Parse `Why: <path>[#<anchor>] | <text>` trailers from a commit message.
pub fn parse_trailers(message: &str) -> Vec<Result<Captured, String>> {
    let mut child = match Command::new("git")
        .args(["interpret-trailers", "--parse"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return vec![Err(format!("could not parse trailers: {error}"))],
    };
    let Some(mut stdin) = child.stdin.take() else {
        return vec![Err("could not open git interpret-trailers stdin".into())];
    };
    if let Err(error) = stdin.write_all(message.as_bytes()) {
        return vec![Err(format!(
            "could not send commit message to git: {error}"
        ))];
    }
    drop(stdin);
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => return vec![Err(format!("could not parse trailers: {error}"))],
    };
    if !output.status.success() {
        return vec![Err(format!(
            "git interpret-trailers failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))];
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case("why")
                .then(|| parse_value(value.trim_start()))
        })
        .collect()
}

fn parse_value(value: &str) -> Result<Captured, String> {
    let Some((target, text)) = value.split_once(" | ") else {
        return Err("expected Why: <path>[#<anchor>] | <text>".into());
    };
    let (path, anchor) = match target.trim().split_once('#') {
        Some((path, anchor)) => (path.trim(), anchor.trim()),
        None => (target.trim(), "@file"),
    };
    if path.is_empty() {
        return Err("Why target path is required".into());
    }
    if text.trim().is_empty() {
        return Err("Why text is required".into());
    }
    Ok(Captured {
        path: path.to_string(),
        anchor: anchor.to_string(),
        text: text.trim().to_string(),
    })
}

/// Word overlap between the trailer and the commit subject. A pasted subject scores 1.0.
fn restates(subject: &str, text: &str) -> bool {
    fn words(value: &str) -> HashSet<String> {
        value
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 3)
            .map(str::to_lowercase)
            .collect()
    }

    let subject = words(subject);
    let text = words(text);
    if subject.is_empty() || text.is_empty() {
        return false;
    }
    let intersection = subject.intersection(&text).count() as f64;
    let union = subject.union(&text).count() as f64;
    intersection / union >= 0.6
}

pub fn capture(rev: &str) {
    if let Err(error) = capture_rev(rev) {
        eprintln!("warning: could not capture Why trailers: {error}");
    }
}

fn capture_rev(rev: &str) -> Result<()> {
    let message = git(&["log", "-1", "--format=%B", rev], None)?;
    let subject = git(&["log", "-1", "--format=%s", rev], None)?;
    let root = git::repo_root()?;
    let branch = git::current_branch();

    for result in parse_trailers(&message) {
        let captured = match result {
            Ok(captured) => captured,
            Err(error) => {
                eprintln!("warning: rejected Why trailer: {error}");
                continue;
            }
        };
        if restates(&subject, &captured.text) {
            eprintln!(
                "warning: rejected Why trailer: record why it must stay true, not the commit subject"
            );
            continue;
        }
        let rel = match store::rel_to_repo(&root, &captured.path) {
            Ok(rel) => rel,
            Err(error) => {
                eprintln!("warning: rejected Why trailer: {error}");
                continue;
            }
        };
        if !root.join(&rel).exists() {
            eprintln!("warning: rejected Why trailer: {rel} does not exist");
            continue;
        }
        let source_text = match std::fs::read_to_string(root.join(&rel)) {
            Ok(source_text) => source_text,
            Err(error) => {
                eprintln!("warning: rejected Why trailer: could not read {rel}: {error}");
                continue;
            }
        };
        let source = Source::new(&source_text, &rel);
        match source.resolve_detail(&captured.anchor) {
            Resolution::Found(_) => {}
            Resolution::NotFound => eprintln!(
                "warning: anchor {:?} not found in {rel}; note recorded anyway",
                captured.anchor
            ),
            Resolution::Unparsed => eprintln!(
                "warning: {rel} has no grammar; note will be kept but not checked for drift"
            ),
        }
        let pending = store::PendingNote {
            text: captured.text,
            path: rel.clone(),
            anchor: captured.anchor.clone(),
            branch: branch.clone(),
            at: now_iso(),
        };
        if let Err(error) = store::append_pending(&root, &pending) {
            eprintln!("warning: could not record Why trailer for {rel}: {error}");
            continue;
        }
        eprintln!("captured -> {rel}#{}", captured.anchor);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_rejects_bracket_keys_and_malformed_blocks() {
        assert!(parse_trailers("subject\n\nWhy[src/a.rs#fn a]: text\n").is_empty());
        assert!(
            parse_trailers("subject\n\nWhy: src/a.rs | one\nnot a trailer\nWhy: src/b.rs | two\n")
                .is_empty()
        );
    }

    #[test]
    fn repeated_case_insensitive_trailers_preserve_values() {
        let parsed = parse_trailers(
            "subject\n\nWhy: src/a.rs#fn a | value: with || marks\nwhy: src/b.rs | second\n",
        );
        assert_eq!(parsed.len(), 2);
        let first = parsed[0].as_ref().unwrap();
        assert_eq!(first.path, "src/a.rs");
        assert_eq!(first.anchor, "fn a");
        assert_eq!(first.text, "value: with || marks");
        let second = parsed[1].as_ref().unwrap();
        assert_eq!(second.anchor, "@file");
    }

    #[test]
    fn markdown_anchors_split_at_the_first_hash_and_require_a_target() {
        let parsed = parse_trailers(
            "Why: ignored.rs | outside\n\nbody\n\nWhy: docs/g.md### Rate limiting | text\n",
        );
        assert_eq!(parsed.len(), 1);
        let note = parsed[0].as_ref().unwrap();
        assert_eq!(note.path, "docs/g.md");
        assert_eq!(note.anchor, "## Rate limiting");

        let bad = parse_trailers("subject\n\nWhy: refactor the parser\n");
        assert!(bad[0].as_ref().unwrap_err().contains("<path>"));
        let untargeted = parse_trailers("subject\n\nWhy: #fn parser | reason\n");
        assert!(untargeted[0].as_ref().unwrap_err().contains("path"));
    }

    #[test]
    fn an_exact_subject_paste_is_refused() {
        assert!(restates("refactor the parser", "refactor the parser"));
    }

    #[test]
    fn a_durable_reason_does_not_restate_the_subject() {
        assert!(!restates(
            "make verify constant-time",
            "must stay constant-time; early return leaks token length"
        ));
    }
}
