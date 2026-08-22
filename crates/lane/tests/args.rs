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
        ("note", Help::Note),
        ("install", Help::Install),
        ("uninstall", Help::Uninstall),
        ("why", Help::Why),
        ("holds", Help::Holds),
        ("check", Help::Check),
        ("audit", Help::Audit),
        ("done", Help::Done),
        ("sweep", Help::Sweep),
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
    assert_eq!(ok(&["ls"]), Parsed::Ls);
    assert_eq!(ok(&["shellenv"]), Parsed::Shellenv);
    assert!(err(&["ls", "extra"]).contains("unexpected argument 'extra' found"));
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
        ok(&["done", "--cd"]),
        Parsed::Done(DoneArgs {
            trunk: None,
            keep: false,
            cd: true,
            squash: false,
            no_merge: false,
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
    assert_eq!(absent(&["note"]), ["--path <PATH>", "<TEXT>"]);
    assert_eq!(absent(&["note", "a finding"]), ["--path <PATH>"]);
    assert_eq!(absent(&["note", "-p", "src/auth.rs"]), ["<TEXT>"]);
}

#[test]
fn note_defaults_the_anchor_to_the_whole_file() {
    assert_eq!(
        ok(&["note", "-p", "src/auth.rs", "a finding"]),
        Parsed::Note(NoteArgs {
            text: "a finding".into(),
            path: "src/auth.rs".into(),
            anchor: "@file".into(),
            supersedes: None,
        })
    );
}

#[test]
fn note_takes_the_long_and_short_spellings_alike() {
    let expected = Parsed::Note(NoteArgs {
        text: "a finding".into(),
        path: "src/auth.rs".into(),
        anchor: "fn verify".into(),
        supersedes: Some("01M0G2".into()),
    });
    assert_eq!(
        ok(&[
            "note",
            "-p",
            "src/auth.rs",
            "-a",
            "fn verify",
            "--supersedes",
            "01M0G2",
            "a finding",
        ]),
        expected
    );
    assert_eq!(
        ok(&[
            "note",
            "--path",
            "src/auth.rs",
            "--anchor",
            "fn verify",
            "--supersedes",
            "01M0G2",
            "a finding",
        ]),
        expected
    );
}

#[test]
fn joined_values_parse_the_way_clap_read_them() {
    assert_eq!(
        ok(&["note", "--path=src/auth.rs", "--anchor=fn verify", "text"]),
        Parsed::Note(NoteArgs {
            text: "text".into(),
            path: "src/auth.rs".into(),
            anchor: "fn verify".into(),
            supersedes: None,
        })
    );
    assert_eq!(
        ok(&["note", "-psrc/auth.rs", "text"]),
        Parsed::Note(NoteArgs {
            text: "text".into(),
            path: "src/auth.rs".into(),
            anchor: "@file".into(),
            supersedes: None,
        })
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
            "-p",
            "README.md",
            "--",
            "--dirty is not the default"
        ]),
        Parsed::Note(NoteArgs {
            text: "--dirty is not the default".into(),
            path: "README.md".into(),
            anchor: "@file".into(),
            supersedes: None,
        })
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
fn why_takes_an_optional_path_and_anchor() {
    assert_eq!(
        ok(&["why"]),
        Parsed::Why(WhyArgs {
            path: None,
            anchor: None
        })
    );
    assert_eq!(
        ok(&["why", "src/auth.rs", "-a", "fn verify"]),
        Parsed::Why(WhyArgs {
            path: Some("src/auth.rs".into()),
            anchor: Some("fn verify".into()),
        })
    );
    assert!(err(&["why", "one", "two"]).contains("unexpected argument 'two' found"));
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

    let Parsed::Done(done) = ok(&["done", "--max-chars", "80"]) else {
        panic!("expected done");
    };
    assert_eq!(done.budget.max_notes, 5);
    assert_eq!(done.budget.max_chars, 80);
}

#[test]
fn done_and_rm_read_the_rest_of_their_flags() {
    assert_eq!(
        ok(&["done", "--trunk", "release", "--keep", "--squash"]),
        Parsed::Done(DoneArgs {
            trunk: Some("release".into()),
            keep: true,
            cd: false,
            squash: true,
            no_merge: false,
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
        ok(&["holds", "01M0G2"]),
        Parsed::Holds {
            id: "01M0G2".into()
        }
    );
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
    assert_eq!(listed.len(), 15, "{listed:?}");
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
        Help::Note,
        Help::Install,
        Help::Uninstall,
        Help::Why,
        Help::Holds,
        Help::Check,
        Help::Audit,
        Help::Done,
        Help::Sweep,
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
fn done_prepares_for_a_pull_request_or_lands_it_but_not_both() {
    let Parsed::Done(done) = ok(&["done", "--no-merge"]) else {
        panic!("expected done");
    };
    assert!(done.no_merge);
    assert!(!done.squash);

    // `--squash` writes the merge commit `--no-merge` leaves to the pull request.
    let message = err(&["done", "--squash", "--no-merge"]);
    assert!(
        message.contains("the argument '--squash' cannot be used with '--no-merge'"),
        "{message}"
    );
    assert!(message.contains("try 'lane done --help'"), "{message}");
}

#[test]
fn sweep_takes_only_its_dry_run() {
    assert_eq!(ok(&["sweep"]), Parsed::Sweep { dry_run: false });
    assert_eq!(ok(&["sweep", "--dry-run"]), Parsed::Sweep { dry_run: true });
    assert_eq!(ok(&["sweep", "--help"]), Parsed::Help(Help::Sweep));
    assert!(err(&["sweep", "extra"]).contains("unexpected argument 'extra' found"));
    assert!(err(&["sweep", "--dry"]).contains("unexpected argument '--dry' found"));
}
