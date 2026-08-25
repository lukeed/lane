//! Promote, re-anchor, rank, evict.

use crate::git;
use crate::note::Note;
use crate::store::{self, BODY, FRESH, MISSING, SIG, TIERS};
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

pub fn confirm(root: &Path, id: &str) -> Result<String> {
    let mut note = store::resolve_id(root, id)?;
    let id = note.meta.id.clone();
    if note.unreadable {
        anyhow::bail!("cannot confirm note {id}: frontmatter is unreadable");
    }
    let mut checker = store::Checker::new(root);
    let res = checker.check(&note);
    if res.span.is_none() {
        anyhow::bail!(
            "cannot confirm note {id}: anchor does not resolve ({})",
            res.tier
        );
    }

    store::confirm(&mut note, &res.sig, &res.body_hash, &res.raw_hash)?;
    Ok(id)
}

fn eviction_key(
    note: &Note,
    touched: &HashSet<String>,
    tiers: &HashMap<String, &'static str>,
) -> (u8, u8, u8, String) {
    (
        u8::from(!note.meta.pinned),
        u8::from(!touched.contains(&note.path())),
        store::tier_rank(tiers.get(&note.meta.id).copied().unwrap_or(FRESH)),
        note.meta.id.clone(),
    )
}

pub fn run(root: &Path, opts: &Options) -> Result<Outcome> {
    store::fold_legacy_log(root)?;
    let created = store::promote_pending(root)?;
    let touched: HashSet<String> = if opts.base.is_empty() {
        HashSet::new()
    } else {
        git::touched_paths(root, &opts.base)
    };

    let mut notes = store::load_notes(root, None);

    // Follow the code before judging it: a renamed file keeps its memory, only a deleted
    // one loses it. Costs a git call only when something is actually missing.
    let mut moved = Vec::new();
    if store::has_missing_sources(root, &notes) {
        for (old, new) in git::renames(root, &opts.base) {
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
    let mut tiers: HashMap<String, &'static str> = HashMap::new();
    let mut rebaselined = 0usize;

    for note in notes.iter_mut() {
        let res = checker.check(note);
        *stats.entry(res.tier).or_insert(0) += 1;
        tiers.insert(note.meta.id.clone(), res.tier);
        if res.rebaselined {
            rebaselined += 1;
        }
        // A normalization change adopts a new baseline without claiming a person vouched.
        if res.adopted {
            store::rebaseline(note, &res.sig, &res.body_hash, &res.raw_hash)?;
        }
        // Seeing drift is not enough to vouch for the new fingerprint, and there is
        // nowhere to vouch it into: the baseline stays whatever was last confirmed.
        if res.tier == BODY || res.tier == SIG {
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
            let missing = tiers.get(&note.meta.id) == Some(&MISSING);
            if missing && !note.meta.pinned {
                store::evict(root, &mut note, "anchor missing")?;
                evicted.push((note, "anchor missing".into()));
            } else {
                live.push(note);
            }
        }

        live.sort_by(|a, b| {
            eviction_key(a, &touched, &tiers).cmp(&eviction_key(b, &touched, &tiers))
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
        "memory: +{} new; checked {}: {} fresh, {} content-changed, {} contract-changed, {} missing",
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
    use crate::syntax::Source;

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

    fn edit(root: &Path, body: &str) {
        std::fs::write(root.join("src/auth.rs"), body).unwrap();
    }

    #[test]
    fn unresolved_drift_never_adopts_the_shape_it_reported() {
        let root = tempfile::tempdir().unwrap();
        let note = note_fixture(root.path(), "pub fn verify() {\n    old();\n}\n");
        edit(root.path(), "pub fn verify() {\n    new();\n}\n");

        let first = store::Checker::new(root.path()).check(&note);
        let second = store::Checker::new(root.path()).check(&note);

        assert_eq!(first.tier, BODY);
        assert_eq!(second.tier, BODY, "drift must stay flagged until resolved");
        assert_eq!(second.base, first.base, "the baseline must not move");
        assert!(note.meta.vouched.is_empty(), "a check writes nothing");
    }

    #[test]
    fn confirm_rewrites_the_note_with_the_fingerprint_it_vouched_for() {
        let root = tempfile::tempdir().unwrap();
        let note = note_fixture(root.path(), "pub fn verify() {\n    old();\n}\n");
        edit(root.path(), "pub fn verify() {\n    new();\n}\n");
        let drifted = store::Checker::new(root.path()).check(&note);
        assert_eq!(drifted.tier, BODY);

        confirm(root.path(), &note.meta.id).unwrap();

        let confirmed = store::resolve_id(root.path(), &note.meta.id).unwrap();
        assert_eq!(confirmed.meta.sig, drifted.sig);
        assert_eq!(confirmed.meta.body_hash, drifted.body_hash);
        assert_eq!(confirmed.meta.raw_hash, drifted.raw_hash);
        assert!(!confirmed.meta.vouched.is_empty());
        assert_eq!(
            store::Checker::new(root.path()).check(&confirmed).tier,
            FRESH
        );
    }

    #[test]
    fn a_confirmation_outlives_the_branch_that_made_it() {
        let root = tempfile::tempdir().unwrap();
        let note = note_fixture(root.path(), "pub fn verify() {\n    old();\n}\n");
        edit(root.path(), "pub fn verify() {\n    new();\n}\n");
        confirm(root.path(), &note.meta.id).unwrap();

        // The note is the whole record: nothing per-branch is left to garbage-collect.
        assert!(!root.path().join(store::LANE_DIR).join("branch").exists());
        let confirmed = store::resolve_id(root.path(), &note.meta.id).unwrap();
        assert_eq!(
            store::Checker::new(root.path()).check(&confirmed).tier,
            FRESH
        );
    }

    #[test]
    fn confirm_refuses_a_missing_anchor_and_records_nothing() {
        let root = tempfile::tempdir().unwrap();
        let note = note_fixture(root.path(), "pub fn verify() { old(); }\n");
        edit(root.path(), "pub const ENABLED: bool = true;\n");

        let error = confirm(root.path(), &note.meta.id).unwrap_err();

        assert!(error.to_string().contains(MISSING));
        assert!(note.meta.vouched.is_empty());
    }

    #[test]
    fn a_fresh_audit_does_not_rewrite_the_note() {
        let root = tempfile::tempdir().unwrap();
        note_fixture(root.path(), "pub fn verify() {\n    old();\n}\n");

        let out = run(
            root.path(),
            &Options {
                base: String::new(),
                max_notes: 5,
                max_chars: 1200,
            },
        )
        .unwrap();

        assert_eq!(out.stats[FRESH], 1);
        let note = store::load_notes(root.path(), None).pop().unwrap();
        assert!(note.meta.vouched.is_empty());
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
        let tiers = HashMap::new();
        let touched = HashSet::new();

        assert!(eviction_key(&older, &touched, &tiers) < eviction_key(&newer, &touched, &tiers));
    }
}
