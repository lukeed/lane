//! Model-in-the-loop review of drifted notes.
//!
//! A hash says a span changed; it cannot say whether the note about it is still true.
//! Off unless configured, and only drifted notes are ever sent.

use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, Instant};

pub const VERDICTS: [&str; 4] = ["holds", "superseded", "contradicted", "unsure"];

const SYSTEM: &str = concat!(
    "You audit code annotations against the code they describe. For each item ",
    "you receive a note and the current text of the span it is anchored to. ",
    "Decide whether the note is still accurate.\n\n",
    "Reply with ONLY a JSON object, no prose and no markdown fences:\n",
    r#"{"verdicts":[{"id":"<id>","verdict":"holds|superseded|contradicted|unsure","#,
    r#""rewrite":"<new note text, only when superseded>","reason":"<one short clause>"}]}"#,
    "\n\nholds: still true, even if the implementation moved.\n",
    "superseded: the underlying point survives but the wording is now wrong or ",
    "misleading; supply a rewrite that is one or two sentences, concrete, and ",
    "says why rather than what.\n",
    "contradicted: the code now does the opposite of what the note claims.\n",
    "unsure: the span alone is not enough to judge. Prefer this over guessing."
);

pub struct Item {
    pub id: String,
    pub path: String,
    pub anchor: String,
    pub note: String,
    pub span: String,
}

#[derive(Clone, Debug)]
pub struct Verdict {
    pub verdict: String,
    pub rewrite: String,
    pub reason: String,
}

pub trait Reviewer {
    fn name(&self) -> String;
    fn enabled(&self) -> bool {
        true
    }
    fn review(&self, items: &[Item]) -> HashMap<String, Verdict>;
}

fn payload(items: &[Item]) -> String {
    let reviews: Vec<serde_json::Value> = items
        .iter()
        .map(|i| {
            serde_json::json!({
                "id": i.id, "path": i.path, "anchor": i.anchor,
                "note": i.note, "span": i.span,
            })
        })
        .collect();
    serde_json::json!({ "reviews": reviews }).to_string()
}

/// Tolerant of fences and of a bare array, because models produce both.
pub fn parse_response(text: &str) -> HashMap<String, Verdict> {
    let mut out = HashMap::new();
    let trimmed = text.trim();
    let unfenced: String = trimmed
        .lines()
        .filter(|l| !l.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n");

    let parsed = serde_json::from_str::<serde_json::Value>(unfenced.trim()).or_else(|_| {
        let start = unfenced.find(['[', '{']);
        let end = unfenced.rfind([']', '}']);
        match (start, end) {
            (Some(a), Some(b)) if b > a => serde_json::from_str(&unfenced[a..=b]),
            _ => serde_json::from_str("null"),
        }
    });
    let Ok(value) = parsed else {
        return out;
    };

    let list = if value.is_array() {
        value.clone()
    } else {
        value
            .get("verdicts")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    let Some(items) = list.as_array() else {
        return out;
    };

    for entry in items {
        let Some(id) = entry.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let raw = entry
            .get("verdict")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let verdict = if VERDICTS.contains(&raw.as_str()) {
            raw
        } else {
            "unsure".into()
        };
        let field = |k: &str| {
            entry
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        out.insert(
            id.to_string(),
            Verdict {
                verdict,
                rewrite: field("rewrite"),
                reason: field("reason").chars().take(200).collect(),
            },
        );
    }
    out
}

pub struct Null;

impl Reviewer for Null {
    fn name(&self) -> String {
        "none".into()
    }
    fn enabled(&self) -> bool {
        false
    }
    fn review(&self, _items: &[Item]) -> HashMap<String, Verdict> {
        HashMap::new()
    }
}

/// Any command that reads JSON on stdin and writes JSON on stdout.
pub struct Cmd {
    pub cmd: String,
    pub timeout: Duration,
}

impl Reviewer for Cmd {
    fn name(&self) -> String {
        let head = self.cmd.split_whitespace().next().unwrap_or("cmd");
        format!("cmd({head})")
    }

    fn review(&self, items: &[Item]) -> HashMap<String, Verdict> {
        let stdin = format!("{SYSTEM}\n\n{}", payload(items));
        let child = std::process::Command::new("sh")
            .arg("-c")
            .arg(&self.cmd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            return HashMap::new();
        };
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(stdin.as_bytes());
        }

        // A hung reviewer must not hang `lane done`.
        let deadline = Instant::now() + self.timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if !status.success() => return HashMap::new(),
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    return HashMap::new();
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => return HashMap::new(),
            }
        }
        match child.wait_with_output() {
            Ok(out) => parse_response(&String::from_utf8_lossy(&out.stdout)),
            Err(_) => HashMap::new(),
        }
    }
}

pub struct Anthropic {
    pub key: String,
    pub model: String,
    pub timeout: Duration,
}

impl Reviewer for Anthropic {
    fn name(&self) -> String {
        format!("anthropic({})", self.model)
    }

    fn review(&self, items: &[Item]) -> HashMap<String, Verdict> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 2000,
            "system": SYSTEM,
            "messages": [{ "role": "user", "content": payload(items) }],
        });
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .build()
            .into();

        // Any failure means no verdicts: `lane done` still has to work on a plane.
        let Ok(mut resp) = agent
            .post("https://api.anthropic.com/v1/messages")
            .header("content-type", "application/json")
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
            .send_json(&body)
        else {
            return HashMap::new();
        };
        let Ok(data) = resp.body_mut().read_json::<serde_json::Value>() else {
            return HashMap::new();
        };
        let text: String = data
            .get("content")
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<String>()
            })
            .unwrap_or_default();
        parse_response(&text)
    }
}

/// Resolution order: explicit flag, LANE_REVIEW_CMD, ANTHROPIC_API_KEY, off.
///
/// Off by default matters: `lane done` must never silently start spending money.
pub fn build(mode: Option<&str>, cmd: Option<&str>) -> Box<dyn Reviewer> {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let mode = mode
        .map(str::to_string)
        .unwrap_or_else(|| match env("LANE_REVIEW").as_str() {
            "" => "auto".into(),
            other => other.into(),
        });
    let cmd = cmd
        .map(str::to_string)
        .unwrap_or_else(|| env("LANE_REVIEW_CMD"));
    let timeout = Duration::from_secs(120);

    match mode.as_str() {
        "none" => Box::new(Null),
        "cmd" => {
            if cmd.is_empty() {
                Box::new(Null)
            } else {
                Box::new(Cmd { cmd, timeout })
            }
        }
        "auto" if !cmd.is_empty() => Box::new(Cmd { cmd, timeout }),
        "anthropic" | "auto" => {
            let key = env("ANTHROPIC_API_KEY");
            if key.is_empty() {
                return Box::new(Null);
            }
            let model = match env("LANE_REVIEW_MODEL").as_str() {
                "" => "claude-haiku-4-5-20251001".into(),
                other => other.to_string(),
            };
            Box::new(Anthropic {
                key,
                model,
                timeout,
            })
        }
        _ => Box::new(Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerates_fenced_json() {
        let out =
            parse_response("```json\n{\"verdicts\":[{\"id\":\"a\",\"verdict\":\"holds\"}]}\n```");
        assert_eq!(out["a"].verdict, "holds");
    }

    #[test]
    fn tolerates_a_bare_array() {
        let out = parse_response(r#"[{"id":"b","verdict":"contradicted","reason":"retries now"}]"#);
        assert_eq!(out["b"].verdict, "contradicted");
        assert_eq!(out["b"].reason, "retries now");
    }

    #[test]
    fn unknown_verdicts_become_unsure() {
        let out = parse_response(r#"{"verdicts":[{"id":"c","verdict":"maybe"}]}"#);
        assert_eq!(out["c"].verdict, "unsure");
    }

    #[test]
    fn garbage_yields_nothing() {
        assert!(parse_response("sorry, I cannot").is_empty());
    }
}
