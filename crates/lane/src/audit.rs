//! Promote, re-anchor, review, rank, evict.

use crate::git;
use crate::note::{Meta, Note};
use crate::review::{Item, Reviewer};
use crate::store::{self, BODY, FRESH, MISSING, SIG, TIERS};
use crate::util::{now_iso, slug, ulid};
use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::Path;

pub struct Options {
    pub base: String,
    pub max_notes: usize,
    pub max_chars: usize,
    pub review_limit: usize,
}

pub struct Outcome {
    pub created: Vec<Note>,
    /// (old path, new path, notes moved) for source files that were renamed, not deleted.
    pub moved: Vec<(String, String, usize)>,
    /// Notes whose baseline predated a normalization change and could not be compared.
    pub rebaselined: usize,
    pub stats: HashMap<&'static str, usize>,
    pub review: Vec<Note>,
    pub evicted: Vec<(Note, String)>,
    pub reviewed: Vec<(Note, String, Option<Note>)>,
    pub reviewer: String,
}

pub fn run(root: &Path, opts: &Options, reviewer: &dyn Reviewer) -> Result<Outcome> {
    let created = store::promote_pending(root)?;
    let counts = store::read_counts(root);
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
        if res.tier == BODY || res.tier == SIG {
            drifted.push(note.clone());
        }

        let previous = state.get(&note.meta.id).cloned().unwrap_or_default();
        let unchanged = previous.sig == res.sig
            && previous.body_hash == res.body_hash
            && previous.raw_hash == res.raw_hash
            && previous.status == res.tier
            && previous.norm == crate::syntax::NORM_VERSION;
        state.insert(
            note.meta.id.clone(),
            store::NoteState {
                sig: res.sig,
                body_hash: res.body_hash,
                raw_hash: res.raw_hash,
                status: res.tier.into(),
                // Only advances when something moved, so a no-op audit writes nothing.
                checked: if unchanged {
                    previous.checked
                } else {
                    now_iso()
                },
                norm: crate::syntax::NORM_VERSION.into(),
                reads: previous.reads,
            },
        );
    }

    let reviewed = apply_review(
        root,
        &mut drifted,
        reviewer,
        &mut checker,
        &mut state,
        opts.review_limit,
    )?;
    if !reviewed.is_empty() {
        // Supersede added files and contradicted removed them.
        notes = store::load_notes(root, None);
    }

    let mut evicted: Vec<(Note, String)> = reviewed
        .iter()
        .filter(|(_, why, _)| why == "contradicted")
        .map(|(n, why, _)| (n.clone(), why.clone()))
        .collect();
    evicted.extend(
        reviewed
            .iter()
            .filter(|(_, why, _)| why == "superseded")
            .map(|(n, _, _)| (n.clone(), "superseded".to_string())),
    );

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
            let key = |n: &Note| {
                (
                    u8::from(!n.meta.pinned),
                    std::cmp::Reverse(counts.get(&n.meta.id).copied().unwrap_or(0)),
                    u8::from(!touched.contains(&n.path())),
                    store::tier_rank(
                        state
                            .get(&n.meta.id)
                            .map(|st| st.status.as_str())
                            .unwrap_or(FRESH),
                    ),
                    n.meta.id.clone(),
                )
            };
            key(a).cmp(&key(b))
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
        review: drifted,
        evicted,
        reviewed,
        reviewer: reviewer.name(),
    })
}

/// Supersede writes a NEW note rather than editing the old one: mutation would break the
/// union-merge invariant that lets parallel lanes write memory without conflicting.
fn apply_review(
    root: &Path,
    drifted: &mut [Note],
    reviewer: &dyn Reviewer,
    checker: &mut store::Checker,
    state: &mut store::State,
    limit: usize,
) -> Result<Vec<(Note, String, Option<Note>)>> {
    if drifted.is_empty() || !reviewer.enabled() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for note in drifted.iter().take(limit) {
        let span = checker.span_text(note);
        if span.is_empty() {
            continue;
        }
        items.push(Item {
            id: note.meta.id.clone(),
            path: note.path(),
            anchor: note.meta.anchor.clone(),
            note: note.body.trim().to_string(),
            span,
        });
    }
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let verdicts = reviewer.review(&items);
    let mut applied = Vec::new();

    for note in drifted.iter_mut() {
        let Some(v) = verdicts.get(&note.meta.id) else {
            continue;
        };
        // The verdict is a decision worth keeping: a model call paid for once.
        store::append_log(
            root,
            &serde_json::json!({
                "at": now_iso(), "kind": "verdict", "id": note.meta.id,
                "path": note.path(), "anchor": note.meta.anchor,
                "verdict": v.verdict, "reason": v.reason,
            }),
        )?;

        match v.verdict.as_str() {
            "superseded" if !v.rewrite.is_empty() => {
                let meta = Meta {
                    id: ulid(),
                    anchor: note.meta.anchor.clone(),
                    created: now_iso(),
                    branch: git::current_branch(),
                    norm: crate::syntax::NORM_VERSION.into(),
                    sig: note.meta.sig.clone(),
                    body_hash: note.meta.body_hash.clone(),
                    raw_hash: note.meta.raw_hash.clone(),
                    lines: note.meta.lines.clone(),
                    supersedes: note.meta.id.clone(),
                    pinned: false,
                };
                let file = store::note_dir(root, &note.path()).join(format!(
                    "{}-{}.md",
                    meta.id,
                    slug(&v.rewrite, 28)
                ));
                let mut fresh = Note::new(meta, v.rewrite.clone());
                fresh.write(&file)?;
                fresh.file = Some(file);
                if let Some(entry) = state.get(&note.meta.id).cloned() {
                    state.insert(fresh.meta.id.clone(), entry);
                }
                let reason = format!("superseded by {}", fresh.meta.id);
                store::evict(root, note, &reason)?;
                state.remove(&note.meta.id);
                applied.push((note.clone(), "superseded".into(), Some(fresh)));
            }
            "contradicted" => {
                // A confidently wrong note is worse than none; the attic keeps it reversible.
                let reason = if v.reason.is_empty() {
                    "code disagrees".to_string()
                } else {
                    v.reason.clone()
                };
                store::evict(root, note, &format!("contradicted: {reason}"))?;
                state.remove(&note.meta.id);
                applied.push((note.clone(), "contradicted".into(), None));
            }
            other => {
                if other == "holds"
                    && let Some(entry) = state.get_mut(&note.meta.id)
                {
                    entry.status = FRESH.into();
                    entry.checked = now_iso();
                }
                applied.push((note.clone(), other.to_string(), None));
            }
        }
    }
    Ok(applied)
}

pub fn report(out: &Outcome, w: &mut dyn Write) -> std::io::Result<()> {
    let n = |tier: &str| out.stats.get(tier).copied().unwrap_or(0);
    writeln!(
        w,
        "memory: +{} new, {} fresh, {} body-drift, {} signature-changed, {} missing",
        out.created.len(),
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

    if out.reviewed.is_empty() {
        for note in &out.review {
            writeln!(w, "  review  {}#{}", note.path(), note.meta.anchor)?;
        }
    } else {
        writeln!(
            w,
            "  reviewed {} drifted note(s) via {}",
            out.reviewed.len(),
            out.reviewer
        )?;
        for (note, verdict, replacement) in &out.reviewed {
            let extra = replacement
                .as_ref()
                .map(|n| format!(" -> {}", &n.meta.id[..10.min(n.meta.id.len())]))
                .unwrap_or_default();
            writeln!(
                w,
                "  {verdict:<13} {}#{}{extra}",
                note.path(),
                note.meta.anchor
            )?;
        }
    }

    for (note, why) in &out.evicted {
        writeln!(w, "  evict   {}#{}  ({why})", note.path(), note.meta.anchor)?;
    }
    Ok(())
}
