//! Small shared helpers.

pub fn now_iso() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// Lexicographic sort equals creation order, with no coordination between worktrees.
pub fn ulid() -> String {
    ulid::Ulid::generate().to_string()
}

pub fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    let cut = trimmed
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(trimmed.len()))
        .take_while(|i| *i <= max)
        .last()
        .unwrap_or(0);
    let slug = trimmed[..cut].trim_matches('-');
    if slug.is_empty() {
        "note".into()
    } else {
        slug.into()
    }
}
