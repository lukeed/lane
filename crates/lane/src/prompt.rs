use crate::syntax::Anchor;
use anyhow::{Result, bail};
use std::io::{BufRead, Write};

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
    write!(output, "Note: ")?;
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
}
