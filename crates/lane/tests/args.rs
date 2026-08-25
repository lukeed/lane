//! The argument parser, driven the way a shell drives it: a list of words in,
//! one `Parsed` out, and no process in between.
//!
//! These live outside `args.rs` because they are longer than the parser they
//! cover, and only reach its public surface anyway.

use anyhow::Result;
use lane::args::*;
use lane::help::Help;
use std::ffi::OsString;

/// The parser is the whole surface now, so the tests drive it the way a
/// shell does: a list of words, with no process in between.
fn parse_words(words: &[&str]) -> Result<Parsed> {
    parse(words.iter().map(OsString::from).collect())
}

fn ok(words: &[&str]) -> Parsed {
    parse_words(words).expect("parses")
}

fn err(words: &[&str]) -> String {
    format!("{:#}", parse_words(words).expect_err("refuses"))
}

#[test]
fn bare_lane_and_the_help_flags_reach_the_root_screen() {
    assert_eq!(ok(&[]), Parsed::Help(Help::Root));
    assert_eq!(ok(&["-h"]), Parsed::Help(Help::Root));
    assert_eq!(ok(&["--help"]), Parsed::Help(Help::Root));
}

#[test]
fn version_has_clap_s_spelling() {
    assert_eq!(ok(&["-V"]), Parsed::Version);
    assert_eq!(ok(&["--version"]), Parsed::Version);
}

#[test]
fn every_command_answers_its_own_help_flag() {
    for (word, screen) in [
        ("init", Help::Init),
        ("new", Help::New),
        ("ls", Help::Ls),
        ("path", Help::Path),
        ("anchors", Help::Anchors),
        ("note", Help::Note),
        ("install", Help::Install),
        ("uninstall", Help::Uninstall),
        ("why", Help::Why),
        ("check", Help::Check),
        ("audit", Help::Audit),
        ("merge", Help::Merge),
        ("prune", Help::Prune),
        ("rm", Help::Rm),
        ("shellenv", Help::Shellenv),
    ] {
        assert_eq!(ok(&[word, "--help"]), Parsed::Help(screen), "{word} --help");
        assert_eq!(ok(&[word, "-h"]), Parsed::Help(screen), "{word} -h");
    }
}

#[test]
fn help_wins_over_the_arguments_beside_it() {
    assert_eq!(ok(&["new", "some-lane", "-h"]), Parsed::Help(Help::New));
    assert_eq!(ok(&["rm", "--force", "-h"]), Parsed::Help(Help::Rm));
}

#[test]
fn commands_without_arguments_take_none() {
    assert_eq!(ok(&["init"]), Parsed::Init);
    assert_eq!(ok(&["ls"]), Parsed::Ls { json: false });
    assert_eq!(ok(&["shellenv"]), Parsed::Shellenv);
    assert!(err(&["ls", "extra"]).contains("unexpected argument 'extra' found"));
}

#[test]
fn anchors_takes_one_path_and_an_optional_json_switch() {
    assert_eq!(
        ok(&["anchors", "src/auth.rs"]),
        Parsed::Anchors {
            path: "src/auth.rs".into(),
            json: false,
        }
    );
    let json = Parsed::Anchors {
        path: "src/auth.rs".into(),
        json: true,
    };
    assert_eq!(ok(&["anchors", "--json", "src/auth.rs"]), json);
    assert_eq!(ok(&["anchors", "src/auth.rs", "--json"]), json);
    assert_eq!(
        ok(&["anchors", "missing", "--help"]),
        Parsed::Help(Help::Anchors)
    );
    assert!(err(&["anchors"]).contains("<PATH>"));
    assert!(err(&["anchors", "one", "two"]).contains("unexpected argument 'two' found"));
    assert!(err(&["anchors", "--jsonn", "one"]).contains("unexpected argument '--jsonn' found"));
}

#[test]
fn new_reads_its_flags_before_or_after_the_name() {
    let expected = Parsed::New(NewArgs {
        name: "fix-login".into(),
        base: Some("main".into()),
        dirty: true,
        cd: true,
    });
    assert_eq!(
        ok(&["new", "fix-login", "--base", "main", "--dirty", "--cd"]),
        expected
    );
    assert_eq!(
        ok(&["new", "--cd", "--base", "main", "--dirty", "fix-login"]),
        expected
    );
}

#[test]
fn new_defaults_the_flags_it_was_not_given() {
    assert_eq!(
        ok(&["new", "spike"]),
        Parsed::New(NewArgs {
            name: "spike".into(),
            base: None,
            dirty: false,
            cd: false,
        })
    );
}

#[test]
fn the_shell_function_s_own_invocation_still_parses() {
    // `lane shellenv` writes `command lane new --cd "$@"`, so the flag
    // arrives before the name and must not swallow it.
    assert_eq!(
        ok(&["new", "--cd", "fix-login"]),
        Parsed::New(NewArgs {
            name: "fix-login".into(),
            base: None,
            dirty: false,
            cd: true,
        })
    );
    assert_eq!(
        ok(&["merge", "--cd"]),
        Parsed::Merge(MergeArgs {
            base: None,
            keep: false,
            cd: true,
            squash: false,
            budget: Budget::default(),
        })
    );
}

#[test]
fn a_missing_name_names_itself() {
    let message = err(&["new"]);
    assert!(message.contains("the following required arguments were not provided"));
    assert!(message.contains("<NAME>"));
    assert!(message.contains("Usage: lane new [OPTIONS] <NAME>"));
    assert!(message.contains("try 'lane new --help'"));
}

/// Just the names a "required arguments were not provided" error lists.
/// The usage line below it repeats them, so the whole message cannot say
/// which ones were actually absent.
fn absent(words: &[&str]) -> Vec<String> {
    let message = err(words);
    let (_, listed) = message
        .split_once("the following required arguments were not provided:\n")
        .unwrap_or_else(|| panic!("not a required-argument error:\n{message}"));
    listed
        .lines()
        .take_while(|line| !line.is_empty())
        .map(|line| line.trim().to_string())
        .collect()
}

#[test]
fn note_requires_both_its_text_and_its_path() {
    assert_eq!(absent(&["note"]), ["<COMMAND>"]);
    assert_eq!(absent(&["note", "add"]), ["<PATH>"]);
    assert_eq!(absent(&["note", "replace"]), ["<ID>"]);
}

#[test]
fn note_defaults_the_anchor_to_the_whole_file() {
    assert_eq!(
        ok(&["note", "add", "src/auth.rs", "a finding"]),
        Parsed::Note(NoteCommand::Add(NoteAddArgs {
            path: "src/auth.rs".into(),
            text: Some("a finding".into()),
            anchor: None,
        }))
    );
}

#[test]
fn note_takes_the_long_and_short_spellings_alike() {
    let expected = Parsed::Note(NoteCommand::Add(NoteAddArgs {
        path: "src/auth.rs".into(),
        text: Some("a finding".into()),
        anchor: Some("fn verify".into()),
    }));
    assert_eq!(
        ok(&["note", "add", "src/auth.rs", "-a", "fn verify", "a finding",]),
        expected
    );
    assert_eq!(
        ok(&[
            "note",
            "add",
            "src/auth.rs",
            "--anchor",
            "fn verify",
            "a finding",
        ]),
        expected
    );
}

#[test]
fn note_family_and_leaf_help_route_to_their_own_screens() {
    assert_eq!(ok(&["note", "--help"]), Parsed::Help(Help::Note));
    for (verb, screen) in [
        ("add", Help::NoteAdd),
        ("replace", Help::NoteReplace),
        ("confirm", Help::NoteConfirm),
        ("retire", Help::NoteRetire),
        ("restore", Help::NoteRestore),
        ("pin", Help::NotePin),
        ("unpin", Help::NoteUnpin),
    ] {
        assert_eq!(
            ok(&["note", verb, "ignored", "--help"]),
            Parsed::Help(screen)
        );
    }
}

#[test]
fn note_add_parses_a_path_optional_text_and_anchor() {
    assert_eq!(
        ok(&["note", "add", "src/auth.rs"]),
        Parsed::Note(NoteCommand::Add(NoteAddArgs {
            path: "src/auth.rs".into(),
            text: None,
            anchor: None,
        }))
    );
    assert_eq!(
        ok(&[
            "note",
            "add",
            "src/auth.rs",
            "--anchor",
            "fn verify",
            "finding",
        ]),
        Parsed::Note(NoteCommand::Add(NoteAddArgs {
            path: "src/auth.rs".into(),
            text: Some("finding".into()),
            anchor: Some("fn verify".into()),
        }))
    );
}

#[test]
fn note_replace_parses_an_id_optional_text_and_overrides() {
    assert_eq!(
        ok(&["note", "replace", "01M0G2"]),
        Parsed::Note(NoteCommand::Replace(NoteReplaceArgs {
            id: "01M0G2".into(),
            text: None,
            path: None,
            anchor: None,
        }))
    );
    assert_eq!(
        ok(&[
            "note",
            "replace",
            "01M0G2",
            "-p",
            "src/auth.rs",
            "-a",
            "fn verify",
            "rewrite",
        ]),
        Parsed::Note(NoteCommand::Replace(NoteReplaceArgs {
            id: "01M0G2".into(),
            text: Some("rewrite".into()),
            path: Some("src/auth.rs".into()),
            anchor: Some("fn verify".into()),
        }))
    );
}

#[test]
fn note_id_verbs_take_exactly_one_id() {
    for (verb, expected) in [
        (
            "confirm",
            NoteCommand::Confirm {
                id: "01M0G2".into(),
            },
        ),
        (
            "retire",
            NoteCommand::Retire {
                id: "01M0G2".into(),
            },
        ),
        (
            "restore",
            NoteCommand::Restore {
                id: "01M0G2".into(),
            },
        ),
        (
            "pin",
            NoteCommand::Pin {
                id: "01M0G2".into(),
            },
        ),
        (
            "unpin",
            NoteCommand::Unpin {
                id: "01M0G2".into(),
            },
        ),
    ] {
        assert_eq!(ok(&["note", verb, "01M0G2"]), Parsed::Note(expected));
    }
}

#[test]
fn note_double_dash_preserves_text_that_starts_with_a_dash() {
    assert_eq!(
        ok(&["note", "add", "README.md", "--", "--not-a-flag"]),
        Parsed::Note(NoteCommand::Add(NoteAddArgs {
            path: "README.md".into(),
            text: Some("--not-a-flag".into()),
            anchor: None,
        }))
    );
}

#[test]
fn note_commands_refuse_missing_and_extra_values() {
    assert_eq!(absent(&["note", "add"]), ["<PATH>"]);
    assert_eq!(absent(&["note", "confirm"]), ["<ID>"]);
    assert!(err(&["note", "add", "one", "two", "three"]).contains("'three'"));
    assert!(err(&["note", "pin", "one", "two"]).contains("'two'"));
    assert!(err(&["note", "replace", "one", "two", "three"]).contains("'three'"));
    assert!(err(&["note", "retier", "01M0G2"]).contains("'retire'"));
}

#[test]
fn legacy_note_and_holds_spellings_are_refused() {
    assert!(parse_words(&["note", "-p", "src/auth.rs", "finding"]).is_err());
    assert!(
        parse_words(&[
            "note",
            "add",
            "src/auth.rs",
            "--supersedes",
            "01M0G2",
            "finding",
        ])
        .is_err()
    );
    assert!(parse_words(&["holds", "01M0G2"]).is_err());
}

#[test]
fn joined_values_parse_the_way_clap_read_them() {
    assert_eq!(
        ok(&["note", "add", "--anchor=fn verify", "src/auth.rs", "text"]),
        Parsed::Note(NoteCommand::Add(NoteAddArgs {
            path: "src/auth.rs".into(),
            text: Some("text".into()),
            anchor: Some("fn verify".into()),
        }))
    );
    assert_eq!(
        ok(&["note", "replace", "-psrc/auth.rs", "01M0G2", "text"]),
        Parsed::Note(NoteCommand::Replace(NoteReplaceArgs {
            id: "01M0G2".into(),
            text: Some("text".into()),
            path: Some("src/auth.rs".into()),
            anchor: None,
        }))
    );
    let Parsed::Audit(audit) = ok(&["audit", "--base=HEAD~3", "--max-notes=3"]) else {
        panic!("expected audit");
    };
    assert_eq!(audit.base, "HEAD~3");
    assert_eq!(audit.budget.max_notes, 3);
}

#[test]
fn a_double_dash_lets_a_finding_start_with_one() {
    assert_eq!(
        ok(&[
            "note",
            "add",
            "README.md",
            "--",
            "--dirty is not the default"
        ]),
        Parsed::Note(NoteCommand::Add(NoteAddArgs {
            path: "README.md".into(),
            text: Some("--dirty is not the default".into()),
            anchor: None,
        }))
    );
}

#[test]
fn install_and_uninstall_take_hooks_or_skill() {
    assert_eq!(
        ok(&["install", "skill"]),
        Parsed::Install(Installable::Skill)
    );
    assert_eq!(
        ok(&["install", "hooks"]),
        Parsed::Install(Installable::Hooks)
    );
    assert_eq!(
        ok(&["uninstall", "hooks"]),
        Parsed::Uninstall(Installable::Hooks)
    );
    assert_eq!(
        ok(&["uninstall", "skill"]),
        Parsed::Uninstall(Installable::Skill)
    );
}

#[test]
fn both_ways_of_getting_an_integration_wrong_say_what_they_mean() {
    // Named once on `Help`, so the reader who typed neither and the reader
    // who typed the wrong one cannot be told different things.
    let tip = Help::Install.tip().trim();
    assert!(err(&["install"]).contains(tip));
    assert!(err(&["install", "hook"]).contains(tip));
    assert!(err(&["uninstall", "hook"]).contains(Help::Uninstall.tip().trim()));
    // The tip sits under the list it explains, not after the closing line.
    let message = err(&["install"]);
    assert!(
        message.find(tip) < message.find("For more information"),
        "{message}"
    );
    assert!(Help::New.tip().is_empty());
}

#[test]
fn a_word_that_only_looks_like_a_flag_is_told_where_to_go() {
    let message = err(&["new", "spike", "--bogus"]);
    assert!(
        message.contains("tip: to pass '--bogus' as a value, use '-- --bogus'"),
        "{message}"
    );
    // A plain word is not a flag, so it gets no advice about `--`.
    assert!(!err(&["ls", "extra"]).contains("tip:"));
}

#[test]
fn an_integration_lane_does_not_ship_is_named_back() {
    let message = err(&["install", "hook"]);
    assert!(
        message.contains("unrecognized integration 'hook'"),
        "{message}"
    );
    assert!(
        message.contains("Usage: lane install <hooks|skill>"),
        "{message}"
    );
    assert!(err(&["install"]).contains("<hooks|skill>"));
}

#[test]
fn the_old_hooks_install_spelling_no_longer_parses() {
    assert!(parse_words(&["hooks", "install"]).is_err());
}

#[test]
fn the_old_done_command_no_longer_parses() {
    assert!(parse_words(&["done"]).is_err());
}

#[test]
fn why_takes_an_optional_path_and_anchor() {
    assert_eq!(
        ok(&["why"]),
        Parsed::Why(WhyArgs {
            path: None,
            anchor: None,
            json: false,
        })
    );
    assert_eq!(
        ok(&["why", "src/auth.rs", "-a", "fn verify"]),
        Parsed::Why(WhyArgs {
            path: Some("src/auth.rs".into()),
            anchor: Some("fn verify".into()),
            json: false,
        })
    );
    assert!(err(&["why", "one", "two"]).contains("unexpected argument 'two' found"));
}

#[test]
fn structured_read_commands_take_json() {
    assert_eq!(ok(&["ls", "--json"]), Parsed::Ls { json: true });
    assert_eq!(
        ok(&["why", "--json"]),
        Parsed::Why(WhyArgs {
            path: None,
            anchor: None,
            json: true,
        })
    );
    assert_eq!(
        ok(&["why", "src/auth.rs", "-a", "fn verify", "--json"]),
        Parsed::Why(WhyArgs {
            path: Some("src/auth.rs".into()),
            anchor: Some("fn verify".into()),
            json: true,
        })
    );
    assert!(err(&["ls", "--jsonn"]).contains("unexpected argument '--jsonn' found"));
    assert!(err(&["why", "--jsonn"]).contains("unexpected argument '--jsonn' found"));
}

#[test]
fn the_budget_flags_default_and_override_together() {
    let Parsed::Audit(audit) = ok(&["audit"]) else {
        panic!("expected audit");
    };
    assert_eq!(audit.budget, Budget::default());
    assert_eq!(audit.budget.max_notes, 5);
    assert_eq!(audit.budget.max_chars, 1200);
    assert_eq!(audit.base, "");
    assert!(!audit.json);

    let Parsed::Merge(merge) = ok(&["merge", "--max-chars", "80"]) else {
        panic!("expected merge");
    };
    assert_eq!(merge.budget.max_notes, 5);
    assert_eq!(merge.budget.max_chars, 80);
}

#[test]
fn merge_and_rm_read_the_rest_of_their_flags() {
    assert_eq!(
        ok(&["merge", "--base", "release", "--keep", "--squash"]),
        Parsed::Merge(MergeArgs {
            base: Some("release".into()),
            keep: true,
            cd: false,
            squash: true,
            budget: Budget::default(),
        })
    );
    assert_eq!(
        ok(&["rm", "spike", "--force"]),
        Parsed::Rm(RmArgs {
            name: "spike".into(),
            force: true,
        })
    );
    assert_eq!(ok(&["check", "--json"]), Parsed::Check { json: true });
    assert_eq!(
        ok(&["path", "spike"]),
        Parsed::Path {
            name: "spike".into()
        }
    );
}

#[test]
fn capture_stays_hidden_but_still_reads_its_revision() {
    assert_eq!(
        ok(&["capture", "HEAD"]),
        Parsed::Capture { rev: "HEAD".into() }
    );
    assert!(!Help::Root.text().contains("capture"));
}

#[test]
fn a_flag_no_command_owns_is_refused() {
    assert!(err(&["new", "spike", "--bogus"]).contains("unexpected argument '--bogus' found"));
    assert!(err(&["--bogus"]).contains("unexpected argument '--bogus' found"));
    assert!(err(&["check", "--jsonn"]).contains("unexpected argument '--jsonn' found"));
}

#[test]
fn a_number_that_is_not_one_says_so() {
    let message = err(&["audit", "--max-notes", "many"]);
    assert!(message.contains("failed to parse 'many'"), "{message}");
}

#[test]
fn an_unknown_command_offers_the_one_that_was_meant() {
    let message = err(&["nope"]);
    assert!(
        message.contains("unrecognized subcommand 'nope'"),
        "{message}"
    );
    assert!(
        message.contains("tip: a similar subcommand exists: 'note'"),
        "{message}"
    );
    assert!(err(&["nwe"]).contains("'new'"));
    // Nothing within two edits: a guess would be noise, so none is offered.
    assert!(!err(&["frobnicate"]).contains("tip:"));
}

#[test]
fn every_command_the_root_screen_lists_parses() {
    // Read out of the screen rather than out of a second list: a command added
    // to one and not the other is exactly what this is here to catch.
    let listed: Vec<&str> = Help::Root
        .text()
        .split("  Commands\n")
        .nth(1)
        .expect("the root screen lists commands")
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert_eq!(listed.len(), 16, "{listed:?}");
    for name in listed {
        assert!(matches!(ok(&[name, "--help"]), Parsed::Help(_)), "{name}");
    }
}

#[test]
fn every_screen_quotes_a_usage_line_it_agrees_with() {
    for screen in [
        Help::Root,
        Help::Init,
        Help::New,
        Help::Ls,
        Help::Path,
        Help::Anchors,
        Help::Note,
        Help::NoteAdd,
        Help::NoteReplace,
        Help::NoteConfirm,
        Help::NoteRetire,
        Help::NoteRestore,
        Help::NotePin,
        Help::NoteUnpin,
        Help::Install,
        Help::Uninstall,
        Help::Why,
        Help::Check,
        Help::Audit,
        Help::Merge,
        Help::Push,
        Help::Prune,
        Help::Rm,
        Help::Shellenv,
    ] {
        let text = screen.text();
        assert!(text.starts_with('\n'), "{screen:?} opens with a blank line");
        assert!(text.contains("  Usage\n"), "{screen:?} has a usage section");
        assert!(text.contains("-h, --help"), "{screen:?} documents --help");
        assert!(
            screen.usage().starts_with(screen.invocation()),
            "{screen:?} usage and invocation disagree"
        );
    }
}

#[test]
fn push_takes_a_base_and_budget() {
    assert_eq!(
        ok(&["push", "--base", "release", "--max-notes", "3"]),
        Parsed::Push(PushArgs {
            base: Some("release".into()),
            budget: Budget {
                max_notes: 3,
                max_chars: 1200,
            },
        })
    );
}

#[test]
fn prune_takes_only_its_dry_run() {
    assert_eq!(ok(&["prune"]), Parsed::Prune { dry_run: false });
    assert_eq!(ok(&["prune", "--dry-run"]), Parsed::Prune { dry_run: true });
    assert_eq!(ok(&["prune", "--help"]), Parsed::Help(Help::Prune));
    assert!(err(&["prune", "extra"]).contains("unexpected argument 'extra' found"));
    assert!(err(&["prune", "--dry"]).contains("unexpected argument '--dry' found"));
    assert!(err(&["sweep"]).contains("unrecognized subcommand 'sweep'"));
}
