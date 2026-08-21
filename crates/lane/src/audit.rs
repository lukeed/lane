//! Promote, re-anchor, rank, evict.

use crate::git;
use crate::note::Note;
use crate::store::{self, BODY, FRESH, MISSING, SIG, TIERS};
use crate::util::now_iso;
use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::Path;

pub struct Options {
    pub base: String,
    pub max_notes: usize,
    pub max_chars: usize,
}

pub struct Outcome {
    pub created: Vec<Note>,
    /// (old path, new path, notes moved) for source files that were renamed, not deleted.
    pub moved: Vec<(String, String, usize)>,
    /// Notes whose baseline predated a normalization change and could not be compared.
    pub rebaselined: usize,
    pub stats: HashMap<&'static str, usize>,
    pub drifted: Vec<Note>,
    pub evicted: Vec<(Note, String)>,
}

fn record_state(state: &mut store::State, id: &str, res: &store::Check) -> bool {
    let unresolved = res.tier == BODY || res.tier == SIG;
    let previous = state.get(id).cloned().unwrap_or_default();
    let (sig, body_hash, raw_hash) = if unresolved {
        // Seeing drift is not enough to vouch for the new fingerprint.
        res.base.clone()
    } else {
        (res.sig.clone(), res.body_hash.clone(), res.raw_hash.clone())
    };
    let unchanged = previous.sig == sig
        && previous.body_hash == body_hash
        && previous.raw_hash == raw_hash
        && previous.status == res.tier
        && previous.norm == crate::syntax::NORM_VERSION;
    state.insert(
        id.to_string(),
        store::NoteState {
            sig,
            body_hash,
            raw_hash,
            status: res.tier.into(),
            // Only advances when something moved, so a no-op audit writes nothing.
            checked: if unchanged {
                previous.checked
            } else {
                now_iso()
            },
            norm: crate::syntax::NORM_VERSION.into(),
        },
    );
    unresolved
}

pub fn refresh_holds(entry: &mut store::NoteState, res: &store::Check) {
    entry.sig = res.sig.clone();
    entry.body_hash = res.body_hash.clone();
    entry.raw_hash = res.raw_hash.clone();
    entry.status = FRESH.into();
    entry.checked = now_iso();
    entry.norm = crate::syntax::NORM_VERSION.into();
}

pub fn holds(root: &Path, id: &str) -> Result<()> {
    let note = store::load_notes(root, None)
        .into_iter()
        .find(|note| note.meta.id == id)
        .ok_or_else(|| anyhow::anyhow!("live note {id} not found"))?;
    let mut checker = store::Checker::new(root);
    let res = checker.check(&note);
    if res.span.is_none() {
        anyhow::bail!(
            "cannot hold note {id}: anchor does not resolve ({})",
            res.tier
        );
    }

    let mut state = store::own_state(root);
    refresh_holds(state.entry(id.to_string()).or_default(), &res);
    store::save_state(root, &state)?;
    store::append_log(
        root,
        &serde_json::json!({
            "at": now_iso(), "kind": "holds", "id": id,
            "path": note.path(), "anchor": note.meta.anchor,
            "branch": git::current_branch(),
        }),
    )?;
    Ok(())
}

fn eviction_key(
    note: &Note,
    touched: &HashSet<String>,
    state: &store::State,
) -> (u8, u8, u8, String) {
    (
        u8::from(!note.meta.pinned),
        u8::from(!touched.contains(&note.path())),
        store::tier_rank(
            state
                .get(&note.meta.id)
                .map(|entry| entry.status.as_str())
                .unwrap_or(FRESH),
        ),
        note.meta.id.clone(),
    )
}

pub fn run(root: &Path, opts: &Options) -> Result<Outcome> {
    let created = store::promote_pending(root)?;
    let touched: HashSet<String> = if opts.base.is_empty() {
        HashSet::new()
    } else {
        git::touched_paths(&opts.base)
    };

    let mut notes = store::load_notes(root, None);

    // Follow the code before judging it: a renamed file keeps its memory, only a deleted
    // one loses it. Costs a git call only when something is actually missing.
    let mut moved = Vec::new();
    if store::has_missing_sources(root, &notes) {
        for (old, new) in git::renames(&opts.base) {
            let count = store::move_notes(root, &old, &new)?;
            if count > 0 {
                moved.push((old, new, count));
            }
        }
        if !moved.is_empty() {
            notes = store::load_notes(root, None);
        }
    }

    let mut checker = store::Checker::new(root);
    let mut stats: HashMap<&'static str, usize> = TIERS.iter().map(|t| (*t, 0)).collect();
    let mut drifted: Vec<Note> = Vec::new();
    // Start from this branch's own file so `lane why`'s read counts survive the audit.
    let mut state = store::own_state(root);
    let mut rebaselined = 0usize;

    for note in notes.iter_mut() {
        let res = checker.check(note);
        *stats.entry(res.tier).or_insert(0) += 1;
        if res.rebaselined {
            rebaselined += 1;
        }
        if record_state(&mut state, &note.meta.id, &res) {
            drifted.push(note.clone());
        }
    }

    let mut evicted = Vec::new();

    let mut by_anchor: BTreeMap<(String, String), Vec<Note>> = BTreeMap::new();
    for note in notes {
        by_anchor
            .entry((note.path(), note.meta.anchor.clone()))
            .or_default()
            .push(note);
    }

    for group in by_anchor.values_mut() {
        let mut live = Vec::new();
        for mut note in group.drain(..) {
            let missing = state
                .get(&note.meta.id)
                .is_some_and(|st| st.status == MISSING);
            if missing && !note.meta.pinned {
                store::evict(root, &mut note, "anchor missing")?;
                evicted.push((note, "anchor missing".into()));
            } else {
                live.push(note);
            }
        }

        live.sort_by(|a, b| {
            eviction_key(a, &touched, &state).cmp(&eviction_key(b, &touched, &state))
        });

        let (mut kept, mut chars) = (0usize, 0usize);
        for mut note in live {
            let over = kept >= opts.max_notes || chars + note.body.len() > opts.max_chars;
            if !note.meta.pinned && over {
                store::evict(root, &mut note, "budget")?;
                evicted.push((note, "budget".into()));
                continue;
            }
            kept += 1;
            chars += note.body.len();
        }
    }

    // Drop cache entries for notes that are no longer live, then write once.
    let live: std::collections::HashSet<String> = store::load_notes(root, None)
        .iter()
        .map(|n| n.meta.id.clone())
        .collect();
    state.retain(|id, _| live.contains(id));
    store::save_state(root, &state)?;
    store::gc_state(root);

    Ok(Outcome {
        created,
        moved,
        stats,
        rebaselined,
        drifted,
        evicted,
    })
}

pub fn report(out: &Outcome, w: &mut dyn Write) -> std::io::Result<()> {
    let n = |tier: &str| out.stats.get(tier).copied().unwrap_or(0);
    writeln!(
        w,
        "memory: +{} new; checked {}: {} fresh, {} body-drift, {} signature-changed, {} missing",
        out.created.len(),
        out.stats.values().sum::<usize>(),
        n(FRESH),
        n(BODY),
        n(SIG),
        n(MISSING)
    )?;

    if out.rebaselined > 0 {
        writeln!(
            w,
            "  re-baselined {} note(s) after a normalization change",
            out.rebaselined
        )?;
    }

    for (old, new, count) in &out.moved {
        writeln!(w, "  moved   {old} -> {new}  ({count} note(s))")?;
    }

    for note in &out.drifted {
        writeln!(w, "  drift   {}#{}", note.path(), note.meta.anchor)?;
    }

    for (note, why) in &out.evicted {
        writeln!(w, "  evict   {}#{}  ({why})", note.path(), note.meta.anchor)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::Meta;
    use crate::store::Check;
    use crate::syntax::Source;

    fn check(tier: &'static str, suffix: &str) -> Check {
        Check {
            tier,
            sig: format!("sig-{suffix}"),
            body_hash: format!("body-{suffix}"),
            raw_hash: format!("raw-{suffix}"),
            base: (
                format!("sig-{suffix}"),
                format!("body-{suffix}"),
                format!("raw-{suffix}"),
            ),
            span: None,
            rebaselined: false,
        }
    }

    fn old_state() -> store::NoteState {
        store::NoteState {
            sig: "sig-old".into(),
            body_hash: "body-old".into(),
            raw_hash: "raw-old".into(),
            status: FRESH.into(),
            checked: "2026-01-01T00:00:00Z".into(),
            norm: crate::syntax::NORM_VERSION.into(),
        }
    }

    fn note_fixture(root: &Path, source: &str) -> Note {
        let rel = "src/auth.rs";
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join(rel), source).unwrap();
        let parsed = Source::new(source, rel);
        let span = parsed.resolve("fn verify").unwrap();
        let (sig, body_hash, raw_hash) = parsed.hashes(span, "fn verify");
        let note = Note::new(
            Meta {
                id: "01M0A".into(),
                anchor: "fn verify".into(),
                norm: crate::syntax::NORM_VERSION.into(),
                sig,
                body_hash,
                raw_hash,
                ..Default::default()
            },
            "must stay constant-time",
        );
        let file = store::note_dir(root, rel).join("01M0A-note.md");
        note.write(&file).unwrap();
        crate::note::parse(&file).unwrap()
    }

    #[test]
    fn unresolved_drift_preserves_the_merged_baseline() {
        let mut state = store::State::new();
        let mut current = check(BODY, "new");
        current.base = (
            "sig-merged".into(),
            "body-merged".into(),
            "raw-merged".into(),
        );

        assert!(record_state(&mut state, "01M0A", &current));

        let saved = state["01M0A"].clone();
        assert_eq!(saved.sig, "sig-merged");
        assert_eq!(saved.body_hash, "body-merged");
        assert_eq!(saved.raw_hash, "raw-merged");
        assert_eq!(saved.status, BODY);
    }

    #[test]
    fn holds_refreshes_the_vouched_fingerprint() {
        let old = old_state();
        let mut state = HashMap::from([("01M0A".into(), old.clone())]);
        let current = check(BODY, "new");
        record_state(&mut state, "01M0A", &current);
        refresh_holds(state.get_mut("01M0A").unwrap(), &current);

        let saved = state["01M0A"].clone();
        assert_ne!(saved.body_hash, old.body_hash);
        assert_eq!(saved.status, FRESH);
    }

    #[test]
    fn holds_refreshes_all_hashes_and_sets_fresh() {
        let root = tempfile::tempdir().unwrap();
        let note = note_fixture(
            root.path(),
            "pub fn verify() {\n    println!(\"old\");\n}\n",
        );
        std::fs::write(
            root.path().join("src/auth.rs"),
            "pub fn verify() {\n    println!(\"new\");\n}\n",
        )
        .unwrap();
        let current = store::Checker::new(root.path()).check(&note);
        assert_eq!(current.tier, BODY);

        holds(root.path(), &note.meta.id).unwrap();

        let saved = store::load_state(root.path())[&note.meta.id].clone();
        assert_eq!(saved.sig, current.sig);
        assert_eq!(saved.body_hash, current.body_hash);
        assert_eq!(saved.raw_hash, current.raw_hash);
        assert_eq!(saved.status, FRESH);
        assert_eq!(store::Checker::new(root.path()).check(&note).tier, FRESH);
    }

    #[test]
    fn holds_refuses_missing_anchor_without_changing_state() {
        let root = tempfile::tempdir().unwrap();
        let note = note_fixture(root.path(), "pub fn verify() { old(); }\n");
        let mut state = store::State::from([(note.meta.id.clone(), old_state())]);
        store::save_state(root.path(), &state).unwrap();
        let before = serde_json::to_value(&state).unwrap();
        std::fs::write(
            root.path().join("src/auth.rs"),
            "pub const ENABLED: bool = true;\n",
        )
        .unwrap();

        let error = holds(root.path(), &note.meta.id).unwrap_err();

        state = store::own_state(root.path());
        assert!(error.to_string().contains(MISSING));
        assert_eq!(serde_json::to_value(state).unwrap(), before);
    }

    #[test]
    fn a_fresh_note_updates_its_fingerprint() {
        let mut state = store::State::new();
        let current = check(FRESH, "current");

        assert!(!record_state(&mut state, "01M0A", &current));

        let saved = state["01M0A"].clone();
        assert_eq!(saved.sig, current.sig);
        assert_eq!(saved.body_hash, current.body_hash);
        assert_eq!(saved.raw_hash, current.raw_hash);
        assert_eq!(saved.status, FRESH);
    }

    #[test]
    fn unchanged_state_keeps_its_checked_time() {
        let current = check(FRESH, "current");
        let mut state = HashMap::from([(
            "01M0A".into(),
            store::NoteState {
                sig: current.sig.clone(),
                body_hash: current.body_hash.clone(),
                raw_hash: current.raw_hash.clone(),
                status: FRESH.into(),
                checked: "2026-01-01T00:00:00Z".into(),
                norm: crate::syntax::NORM_VERSION.into(),
            },
        )]);
        let checked = state["01M0A"].checked.clone();

        assert!(!record_state(&mut state, "01M0A", &current));
        assert_eq!(state["01M0A"].checked, checked);
    }

    #[test]
    fn budget_uses_age_when_the_other_terms_tie() {
        let older = Note::new(
            Meta {
                id: "01M0A".into(),
                ..Default::default()
            },
            "older",
        );
        let newer = Note::new(
            Meta {
                id: "01M0B".into(),
                ..Default::default()
            },
            "newer",
        );
        let state = store::State::new();
        let touched = HashSet::new();

        assert!(eviction_key(&older, &touched, &state) < eviction_key(&newer, &touched, &state));
    }
}
