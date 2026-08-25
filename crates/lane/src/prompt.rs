use crate::syntax::Anchor;
use anyhow::{Result, bail};
use std::io::{BufRead, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditAction {
    Confirm,
    Replace,
    Retire,
    SetPinned(bool),
}

pub(crate) fn select_anchor<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    path: &str,
    anchors: &[Anchor],
) -> Result<Anchor> {
    let Some(only) = anchors.first() else {
        bail!("no anchors available for {path}");
    };
    if anchors.len() == 1 {
        return Ok(only.clone());
    }

    writeln!(output, "Anchor for {path}:")?;
    for (index, anchor) in anchors.iter().enumerate() {
        writeln!(
            output,
            "  {}. {} ({}-{})",
            index + 1,
            anchor.value,
            anchor.span.start,
            anchor.span.end
        )?;
    }

    loop {
        write!(output, "Choose [1-{}]: ", anchors.len())?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            bail!("no anchor selected");
        }
        match line.trim().parse::<usize>() {
            Ok(choice) if (1..=anchors.len()).contains(&choice) => {
                return Ok(anchors[choice - 1].clone());
            }
            _ => writeln!(output, "Enter a number from 1 to {}.", anchors.len())?,
        }
    }
}

pub(crate) fn read_note<R: BufRead, W: Write>(input: &mut R, output: &mut W) -> Result<String> {
    read_text(input, output, "Note")
}

pub(crate) fn read_replacement<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
) -> Result<String> {
    read_text(input, output, "Replacement")
}

fn read_text<R: BufRead, W: Write>(input: &mut R, output: &mut W, label: &str) -> Result<String> {
    write!(output, "{label}: ")?;
    output.flush()?;
    let mut line = String::new();
    if input.read_line(&mut line)? == 0 {
        bail!("no note text entered");
    }
    let text = line.trim();
    if text.is_empty() {
        bail!("note text cannot be empty");
    }
    Ok(text.to_string())
}

pub(crate) fn select_edit_action<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    pinned: bool,
) -> Result<EditAction> {
    let (pin_verb, pin_description) = if pinned {
        ("unpin", "remove eviction protection")
    } else {
        ("pin", "protect from eviction")
    };
    writeln!(output, "Action:")?;
    writeln!(output, "  1. confirm — still true")?;
    writeln!(output, "  2. replace — change the text")?;
    writeln!(output, "  3. retire — no longer applies")?;
    writeln!(output, "  4. {pin_verb} — {pin_description}")?;

    loop {
        write!(output, "Choose [1-4]: ")?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            bail!("no edit action selected");
        }
        match line.trim() {
            "1" => return Ok(EditAction::Confirm),
            "2" => return Ok(EditAction::Replace),
            "3" => return Ok(EditAction::Retire),
            "4" => return Ok(EditAction::SetPinned(!pinned)),
            _ => writeln!(output, "Enter a number from 1 to 4.")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::Span;
    use std::io::Cursor;

    fn anchor(value: &str, start: usize, end: usize) -> Anchor {
        Anchor {
            value: value.into(),
            span: Span { start, end },
        }
    }

    #[test]
    fn a_sole_anchor_is_selected_without_a_menu() {
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let only = anchor("@file", 1, 8);
        assert_eq!(
            select_anchor(
                &mut input,
                &mut output,
                "src/auth.rs",
                std::slice::from_ref(&only),
            )
            .unwrap(),
            only
        );
        assert!(output.is_empty());
    }

    #[test]
    fn a_numbered_choice_selects_the_matching_anchor() {
        let anchors = [anchor("@file", 1, 8), anchor("fn verify", 1, 4)];
        let mut input = Cursor::new(b"2\n");
        let mut output = Vec::new();
        assert_eq!(
            select_anchor(&mut input, &mut output, "src/auth.rs", &anchors).unwrap(),
            anchors[1]
        );
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Anchor for src/auth.rs:\n  1. @file (1-8)\n  2. fn verify (1-4)\nChoose [1-2]: "
        );
    }

    #[test]
    fn an_invalid_choice_is_explained_and_retried() {
        let anchors = [anchor("@file", 1, 8), anchor("fn verify", 1, 4)];
        let mut input = Cursor::new(b"nope\n3\n1\n");
        let mut output = Vec::new();
        assert_eq!(
            select_anchor(&mut input, &mut output, "src/auth.rs", &anchors).unwrap(),
            anchors[0]
        );
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "Anchor for src/auth.rs:\n  1. @file (1-8)\n  2. fn verify (1-4)\n\
             Choose [1-2]: Enter a number from 1 to 2.\n\
             Choose [1-2]: Enter a number from 1 to 2.\n\
             Choose [1-2]: "
        );
    }

    #[test]
    fn empty_and_eof_note_text_are_refused() {
        let mut empty = Cursor::new(b"  \n");
        let mut empty_output = Vec::new();
        assert_eq!(
            read_note(&mut empty, &mut empty_output)
                .unwrap_err()
                .to_string(),
            "note text cannot be empty"
        );
        assert_eq!(String::from_utf8(empty_output).unwrap(), "Note: ");

        let mut eof = Cursor::new(Vec::<u8>::new());
        let mut eof_output = Vec::new();
        assert_eq!(
            read_note(&mut eof, &mut eof_output)
                .unwrap_err()
                .to_string(),
            "no note text entered"
        );
        assert_eq!(String::from_utf8(eof_output).unwrap(), "Note: ");
    }

    #[test]
    fn edit_selects_an_action_and_labels_the_pin_toggle() {
        let mut input = Cursor::new(b"2\n");
        let mut output = Vec::new();
        assert_eq!(
            select_edit_action(&mut input, &mut output, false).unwrap(),
            EditAction::Replace
        );
        assert_eq!(
            String::from_utf8(output).unwrap(),
            concat!(
                "Action:\n",
                "  1. confirm — still true\n",
                "  2. replace — change the text\n",
                "  3. retire — no longer applies\n",
                "  4. pin — protect from eviction\n",
                "Choose [1-4]: "
            )
        );

        let mut input = Cursor::new(b"4\n");
        let mut output = Vec::new();
        assert_eq!(
            select_edit_action(&mut input, &mut output, true).unwrap(),
            EditAction::SetPinned(false)
        );
        assert!(String::from_utf8(output).unwrap().contains("4. unpin —"));
    }

    #[test]
    fn edit_retries_an_invalid_action_and_reads_replacement_text() {
        let mut input = Cursor::new(b"nope\n2\nreplacement text\n");
        let mut output = Vec::new();
        assert_eq!(
            select_edit_action(&mut input, &mut output, false).unwrap(),
            EditAction::Replace
        );
        assert_eq!(
            read_replacement(&mut input, &mut output).unwrap(),
            "replacement text"
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Choose [1-4]: Enter a number from 1 to 4.\nChoose [1-4]: "));
        assert!(output.ends_with("Replacement: "));
    }
}
