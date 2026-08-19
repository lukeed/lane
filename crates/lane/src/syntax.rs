//! Anchor resolution and span hashing, both driven by tree-sitter.
//!
//! Everything downstream consumes a line range and never learns how it was produced.

use std::ops::Range;
use std::sync::OnceLock;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, Tree};

/// 1-indexed, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

pub enum Resolution {
    Found(Span),
    NotFound,
    /// No grammar for this file type, so absence of a match means nothing.
    Unparsed,
}

struct Grammar {
    language: fn() -> Language,
    decls: &'static str,
}

const RUST: Grammar = Grammar {
    language: || tree_sitter_rust::LANGUAGE.into(),
    decls: r#"
        (function_item name: (identifier) @name) @decl
        (struct_item name: (type_identifier) @name) @decl
        (enum_item name: (type_identifier) @name) @decl
        (trait_item name: (type_identifier) @name) @decl
        (type_item name: (type_identifier) @name) @decl
        (mod_item name: (identifier) @name) @decl
        (const_item name: (identifier) @name) @decl
        (static_item name: (identifier) @name) @decl
        (macro_definition name: (identifier) @name) @decl
        (impl_item type: (type_identifier) @name) @decl
    "#,
};

const GO: Grammar = Grammar {
    language: || tree_sitter_go::LANGUAGE.into(),
    decls: r#"
        (function_declaration name: (identifier) @name) @decl
        (method_declaration name: (field_identifier) @name) @decl
        (type_declaration (type_spec name: (type_identifier) @name)) @decl
        (const_declaration (const_spec name: (identifier) @name)) @decl
        (var_declaration (var_spec name: (identifier) @name)) @decl
    "#,
};

const PYTHON: Grammar = Grammar {
    language: || tree_sitter_python::LANGUAGE.into(),
    decls: r#"
        (function_definition name: (identifier) @name) @decl
        (class_definition name: (identifier) @name) @decl
    "#,
};

/// Shared by javascript, typescript and tsx; the TS-only forms are appended below.
const JS_DECLS: &str = r#"
    (function_declaration name: (identifier) @name) @decl
    (generator_function_declaration name: (identifier) @name) @decl
    (class_declaration name: (identifier) @name) @decl
    (method_definition name: (property_identifier) @name) @decl
    (lexical_declaration (variable_declarator name: (identifier) @name)) @decl
    (variable_declaration (variable_declarator name: (identifier) @name)) @decl
"#;

const TS_DECLS: &str = r#"
    (function_declaration name: (identifier) @name) @decl
    (generator_function_declaration name: (identifier) @name) @decl
    (class_declaration name: (type_identifier) @name) @decl
    (method_definition name: (property_identifier) @name) @decl
    (lexical_declaration (variable_declarator name: (identifier) @name)) @decl
    (variable_declaration (variable_declarator name: (identifier) @name)) @decl
    (interface_declaration name: (type_identifier) @name) @decl
    (type_alias_declaration name: (type_identifier) @name) @decl
    (enum_declaration name: (identifier) @name) @decl
"#;

const C_DECLS: &str = r#"
    (function_definition declarator: (function_declarator declarator: (identifier) @name)) @decl
    (declaration declarator: (function_declarator declarator: (identifier) @name)) @decl
    (struct_specifier name: (type_identifier) @name) @decl
    (union_specifier name: (type_identifier) @name) @decl
    (enum_specifier name: (type_identifier) @name) @decl
    (type_definition declarator: (type_identifier) @name) @decl
"#;

const JAVA: Grammar = Grammar {
    language: || tree_sitter_java::LANGUAGE.into(),
    decls: r#"
        (method_declaration name: (identifier) @name) @decl
        (constructor_declaration name: (identifier) @name) @decl
        (class_declaration name: (identifier) @name) @decl
        (interface_declaration name: (identifier) @name) @decl
        (enum_declaration name: (identifier) @name) @decl
        (record_declaration name: (identifier) @name) @decl
    "#,
};

const BASH: Grammar = Grammar {
    language: || tree_sitter_bash::LANGUAGE.into(),
    decls: r#"(function_definition name: (word) @name) @decl"#,
};

/// css/html/markdown carry no name anchors; they are here for block, heading and comment work.
const CSS: Grammar = Grammar {
    language: || tree_sitter_css::LANGUAGE.into(),
    decls: "",
};

const HTML: Grammar = Grammar {
    language: || tree_sitter_html::LANGUAGE.into(),
    decls: "",
};

const MARKDOWN: Grammar = Grammar {
    language: || tree_sitter_md::LANGUAGE.into(),
    decls: "",
};

fn grammar_for(ext: &str) -> Option<Grammar> {
    Some(match ext {
        "rs" => RUST,
        "go" => GO,
        "py" | "pyi" => PYTHON,
        "js" | "mjs" | "cjs" | "jsx" => Grammar {
            language: || tree_sitter_javascript::LANGUAGE.into(),
            decls: JS_DECLS,
        },
        "ts" | "mts" | "cts" => Grammar {
            language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            decls: TS_DECLS,
        },
        "tsx" => Grammar {
            language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
            decls: TS_DECLS,
        },
        "c" | "h" => Grammar {
            language: || tree_sitter_c::LANGUAGE.into(),
            decls: C_DECLS,
        },
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Grammar {
            language: || tree_sitter_cpp::LANGUAGE.into(),
            decls: C_DECLS,
        },
        "java" => JAVA,
        "sh" | "bash" | "zsh" => BASH,
        "css" => CSS,
        // An SFC is three languages in one; html locates the blocks, the anchor picks the rest.
        "html" | "htm" | "svelte" | "vue" => HTML,
        "md" | "markdown" => MARKDOWN,
        _ => return None,
    })
}

fn is_sfc(ext: &str) -> bool {
    matches!(ext, "svelte" | "vue" | "html" | "htm")
}

fn parse_with(text: &str, language: &Language) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    parser.parse(text, None)
}

/// One parsed file, reused across every note anchored to it.
pub struct Source {
    text: String,
    ext: String,
    grammar: Option<Grammar>,
    tree: Option<Tree>,
    query: OnceLock<Option<Query>>,
}

impl Source {
    pub fn new(text: &str, path: &str) -> Self {
        let ext = std::path::Path::new(path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let grammar = grammar_for(&ext);
        let tree = grammar
            .as_ref()
            .and_then(|g| parse_with(text, &(g.language)()));
        Source {
            text: text.to_string(),
            ext,
            grammar,
            tree,
            query: OnceLock::new(),
        }
    }

    fn line_count(&self) -> usize {
        self.text.lines().count()
    }

    /// Byte range covering a 1-indexed inclusive line span.
    fn byte_range(&self, span: Span) -> Range<usize> {
        let mut start = self.text.len();
        let mut end = self.text.len();
        let mut offset = 0usize;
        for (i, line) in self.text.split_inclusive('\n').enumerate() {
            let no = i + 1;
            if no == span.start {
                start = offset;
            }
            offset += line.len();
            if no == span.end {
                end = offset;
                break;
            }
        }
        start.min(end)..end
    }

    pub fn span_text(&self, span: Span) -> String {
        self.text[self.byte_range(span)].to_string()
    }

    pub fn resolve_detail(&self, anchor: &str) -> Resolution {
        let a = anchor.trim();
        if a.is_empty() || a == "@file" || a == "*" {
            return Resolution::Found(Span {
                start: 1,
                end: self.line_count().max(1),
            });
        }
        if self.grammar.is_none() {
            return Resolution::Unparsed;
        }
        let span = if let Some(block) = a.strip_prefix('#')
            && !block.starts_with('#')
            && matches!(
                block.trim().to_lowercase().as_str(),
                "script" | "style" | "template"
            ) {
            self.resolve_block(block.trim())
        } else if a.starts_with("##") || a.starts_with("# ") {
            self.resolve_heading(a)
        } else {
            self.resolve_decl(a)
        };
        span.map_or(Resolution::NotFound, Resolution::Found)
    }

    pub fn resolve(&self, anchor: &str) -> Option<Span> {
        let Resolution::Found(span) = self.resolve_detail(anchor) else {
            return None;
        };
        Some(span)
    }

    /// `<script>` / `<style>` / `<template>` in an SFC or html file.
    fn resolve_block(&self, block: &str) -> Option<Span> {
        let tree = self.tree.as_ref()?;
        let mut found = None;
        walk(tree.root_node(), &mut |node| {
            if found.is_some() {
                return;
            }
            // html gives script and style their own node kinds; template is a plain element.
            let hit = match node.kind() {
                "script_element" => block.eq_ignore_ascii_case("script"),
                "style_element" => block.eq_ignore_ascii_case("style"),
                "element" => {
                    element_tag(node, &self.text).is_some_and(|t| t.eq_ignore_ascii_case(block))
                }
                _ => false,
            };
            if hit {
                found = Some(node_span(node));
            }
        });
        found
    }

    /// A markdown section: the heading plus everything until the next heading of its level or above.
    fn resolve_heading(&self, anchor: &str) -> Option<Span> {
        let tree = self.tree.as_ref()?;
        let mut headings: Vec<(usize, usize, String)> = Vec::new();
        walk(tree.root_node(), &mut |node| {
            if node.kind() != "atx_heading" {
                return;
            }
            let text = self.text[node.byte_range()].trim().to_string();
            let level = text.chars().take_while(|c| *c == '#').count();
            headings.push((node.start_position().row + 1, level, text));
        });
        headings.sort_by_key(|h| h.0);

        let want = anchor.trim();
        let idx = headings.iter().position(|(_, _, text)| text == want)?;
        let (start, level, _) = headings[idx];
        let end = headings[idx + 1..]
            .iter()
            .find(|(_, l, _)| *l <= level)
            .map(|(line, _, _)| line - 1)
            .unwrap_or_else(|| self.line_count());
        Some(Span {
            start,
            end: end.max(start),
        })
    }

    /// `fn verify` or a bare `verify`: the earliest declaration of that name in the file.
    fn resolve_decl(&self, anchor: &str) -> Option<Span> {
        let tree = self.tree.as_ref()?;
        let query = self.decl_query().as_ref()?;
        let parts: Vec<&str> = anchor.split_whitespace().collect();
        let name = *parts.last()?;
        let keyword = if parts.len() > 1 {
            Some(parts[0])
        } else {
            None
        };

        let name_idx = query.capture_index_for_name("name")?;
        let decl_idx = query.capture_index_for_name("decl")?;

        let mut best: Option<(usize, Span)> = None;
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), self.text.as_bytes());
        while let Some(m) = matches.next() {
            let named = m
                .captures
                .iter()
                .find(|c| c.index == name_idx)
                .map(|c| &self.text[c.node.byte_range()]);
            if named != Some(name) {
                continue;
            }
            let Some(decl) = m.captures.iter().find(|c| c.index == decl_idx) else {
                continue;
            };
            // The keyword is checked against the declaration's own line: `fn verify`
            // must find a line that really says `fn`, whichever pattern matched it.
            if let Some(kw) = keyword {
                let row = decl.node.start_position().row;
                let line = self.text.lines().nth(row).unwrap_or_default();
                if !has_word(line, kw) {
                    continue;
                }
            }
            let start = decl.node.start_byte();
            if best.as_ref().is_none_or(|(b, _)| start < *b) {
                best = Some((start, node_span(decl.node)));
            }
        }
        best.map(|(_, span)| span)
    }

    fn decl_query(&self) -> &Option<Query> {
        self.query.get_or_init(|| {
            let g = self.grammar.as_ref()?;
            if g.decls.trim().is_empty() {
                return None;
            }
            Query::new(&(g.language)(), g.decls).ok()
        })
    }

    /// Which grammar owns the comments inside this span.
    fn comment_language(&self, anchor: &str) -> Option<Language> {
        if is_sfc(&self.ext) {
            let block = anchor.trim().trim_start_matches('#').trim().to_lowercase();
            return Some(match block.as_str() {
                "script" => tree_sitter_javascript::LANGUAGE.into(),
                "style" => tree_sitter_css::LANGUAGE.into(),
                _ => tree_sitter_html::LANGUAGE.into(),
            });
        }
        self.grammar.as_ref().map(|g| (g.language)())
    }

    /// (signature, body, raw) hashes for a span. The first two normalize comments and
    /// whitespace away; the raw one does not, so a grammar upgrade can be resolved.
    pub fn hashes(&self, span: Span, anchor: &str) -> (String, String, String) {
        let chunk = self.span_text(span);
        if chunk.is_empty() {
            return (String::new(), String::new(), String::new());
        }
        let stripped = strip_comments(&chunk, self.comment_language(anchor).as_ref());
        let mut lines = stripped.lines();
        let sig = normalize(lines.next().unwrap_or_default());
        let body = normalize(&lines.collect::<Vec<_>>().join("\n"));
        (sha(&sig), sha(&body), sha(&chunk))
    }
}

/// Comment bytes become spaces, so line structure and offsets survive.
fn strip_comments(text: &str, language: Option<&Language>) -> String {
    let Some(language) = language else {
        return text.to_string();
    };
    let Some(tree) = parse_with(text, language) else {
        return text.to_string();
    };
    let mut out = text.as_bytes().to_vec();
    walk(tree.root_node(), &mut |node| {
        if !node.kind().contains("comment") {
            return;
        }
        for byte in &mut out[node.byte_range()] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    });
    String::from_utf8_lossy(&out).into_owned()
}

/// Collapse runs of whitespace and drop blank lines; formatting churn is not drift.
fn normalize(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Bump whenever a grammar upgrade or a change to strip_comments/normalize can move a hash.
pub const NORM_VERSION: &str = "1";

pub fn sha(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn walk(node: Node, visit: &mut dyn FnMut(Node)) {
    visit(node);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, visit);
    }
}

fn node_span(node: Node) -> Span {
    Span {
        start: node.start_position().row + 1,
        end: node.end_position().row + 1,
    }
}

fn element_tag<'a>(node: Node, text: &'a str) -> Option<&'a str> {
    let mut cursor = node.walk();
    let start_tag = node
        .children(&mut cursor)
        .find(|c| c.kind() == "start_tag")?;
    let mut inner = start_tag.walk();
    let name = start_tag
        .children(&mut inner)
        .find(|c| c.kind() == "tag_name")?;
    Some(&text[name.byte_range()])
}

/// Word-boundary containment without pulling in a regex engine.
fn has_word(haystack: &str, word: &str) -> bool {
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
    let mut from = 0;
    while let Some(hit) = haystack[from..].find(word) {
        let at = from + hit;
        let before = haystack[..at].chars().next_back();
        let after = haystack[at + word.len()..].chars().next();
        if boundary(before) && boundary(after) {
            return true;
        }
        from = at + word.len().max(1);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span_of(text: &str, path: &str, anchor: &str) -> Option<Span> {
        Source::new(text, path).resolve(anchor)
    }

    const RS_SRC: &str = r#"pub fn verify(token: &str) -> bool {
    let msg = "closing } inside a string";
    parse(token).is_valid()
}

pub fn refresh(token: &str) -> String {
    rotate(token)
}
"#;

    #[test]
    fn resolves_a_rust_fn_to_its_real_extent() {
        assert_eq!(
            span_of(RS_SRC, "src/auth.rs", "fn verify"),
            Some(Span { start: 1, end: 4 })
        );
        assert_eq!(
            span_of(RS_SRC, "src/auth.rs", "verify"),
            Some(Span { start: 1, end: 4 })
        );
        assert_eq!(
            span_of(RS_SRC, "src/auth.rs", "fn refresh"),
            Some(Span { start: 6, end: 8 })
        );
    }

    #[test]
    fn a_brace_in_a_string_does_not_end_the_span() {
        // The regex predecessor counted braces per line and stopped at line 2.
        let span = span_of(RS_SRC, "src/auth.rs", "fn verify").unwrap();
        assert_eq!(span.end, 4);
    }

    #[test]
    fn the_keyword_must_match_the_declaration() {
        assert!(span_of(RS_SRC, "src/auth.rs", "struct verify").is_none());
    }

    #[test]
    fn earliest_declaration_wins_not_pattern_order() {
        let src = "pub fn handler() {}\n\npub const handler: u8 = 1;\n";
        assert_eq!(span_of(src, "a.rs", "handler").unwrap().start, 1);
    }

    #[test]
    fn file_anchor_covers_everything() {
        assert_eq!(
            span_of(RS_SRC, "a.rs", "@file"),
            Some(Span { start: 1, end: 8 })
        );
    }

    const MD_SRC: &str = "# Title\n\n## Rate limiting\n\none bucket per key.\n\n```\n## Not a heading\n```\n\n## Retries\n\nbackoff.\n";

    #[test]
    fn markdown_section_stops_at_the_next_heading() {
        assert_eq!(
            span_of(MD_SRC, "docs/g.md", "## Rate limiting"),
            Some(Span { start: 3, end: 10 })
        );
    }

    #[test]
    fn a_heading_inside_a_code_fence_is_not_a_heading() {
        // The regex predecessor matched this line and mis-sectioned the document.
        assert!(span_of(MD_SRC, "docs/g.md", "## Not a heading").is_none());
    }

    const SFC_SRC: &str = "<script>\n  let undoStack = [];\n</script>\n<style>\n  .viewport { overflow: auto; }\n</style>\n";

    #[test]
    fn sfc_blocks_resolve_separately() {
        assert_eq!(
            span_of(SFC_SRC, "src/E.svelte", "#script"),
            Some(Span { start: 1, end: 3 })
        );
        assert_eq!(
            span_of(SFC_SRC, "src/E.svelte", "#style"),
            Some(Span { start: 4, end: 6 })
        );
    }

    #[test]
    fn comment_churn_is_not_drift() {
        let a = Source::new("pub fn f() {\n    go(); // fire and forget\n}\n", "a.rs");
        let b = Source::new(
            "pub fn f() {\n    go(); // fire, forget, move on\n}\n",
            "a.rs",
        );
        let span = Span { start: 1, end: 3 };
        let (sig_a, body_a, raw_a) = a.hashes(span, "fn f");
        let (sig_b, body_b, raw_b) = b.hashes(span, "fn f");
        assert_eq!((sig_a, body_a), (sig_b, body_b));
        // The raw hash is deliberately not comment-insensitive; that is its whole job.
        assert_ne!(raw_a, raw_b);
    }

    #[test]
    fn a_url_change_inside_a_string_is_drift() {
        // The regex predecessor truncated at `//` and called these identical.
        let a = Source::new(
            "pub fn f() {\n    get(\"https://x.com/charge\");\n}\n",
            "a.rs",
        );
        let b = Source::new(
            "pub fn f() {\n    get(\"https://x.com/refund\");\n}\n",
            "a.rs",
        );
        let span = Span { start: 1, end: 3 };
        assert_ne!(a.hashes(span, "fn f"), b.hashes(span, "fn f"));
    }

    #[test]
    fn a_markdown_signature_is_a_real_hash() {
        let src = Source::new(MD_SRC, "docs/g.md");
        let span = src.resolve("## Rate limiting").unwrap();
        let (sig, _, _) = src.hashes(span, "## Rate limiting");
        assert_ne!(sig, sha(""));
    }

    #[test]
    fn editing_script_leaves_style_alone() {
        let before = Source::new(SFC_SRC, "E.svelte");
        let after = Source::new(
            "<script>\n  let undoStack = [];\n  let cursor = 0;\n</script>\n<style>\n  .viewport { overflow: auto; }\n</style>\n",
            "E.svelte",
        );
        let style_before = before.resolve("#style").unwrap();
        let style_after = after.resolve("#style").unwrap();
        assert_eq!(
            before.hashes(style_before, "#style"),
            after.hashes(style_after, "#style")
        );
        let script_before = before.resolve("#script").unwrap();
        let script_after = after.resolve("#script").unwrap();
        assert_ne!(
            before.hashes(script_before, "#script"),
            after.hashes(script_after, "#script")
        );
    }

    #[test]
    fn python_and_typescript_resolve_too() {
        let py = "def verify(token):\n    return ok(token)\n";
        assert_eq!(
            span_of(py, "a.py", "def verify"),
            Some(Span { start: 1, end: 2 })
        );
        let ts = "export const verify = (t: string): boolean => {\n  return ok(t);\n};\n";
        assert_eq!(span_of(ts, "a.ts", "const verify").unwrap().start, 1);
        let iface = "export interface Session {\n  id: string;\n}\n";
        assert_eq!(
            span_of(iface, "a.ts", "interface Session"),
            Some(Span { start: 1, end: 3 })
        );
    }

    #[test]
    fn a_changed_declaration_moves_the_signature_not_just_the_body() {
        let base = Source::new("pub fn verify(t: &str) -> bool {\n    ok(t)\n}\n", "a.rs");
        let sig_changed = Source::new(
            "pub fn verify(t: &str, now: u64) -> bool {\n    ok(t)\n}\n",
            "a.rs",
        );
        let body_changed = Source::new("pub fn verify(t: &str) -> bool {\n    ok2(t)\n}\n", "a.rs");
        let span = Span { start: 1, end: 3 };

        let (sig0, body0, _) = base.hashes(span, "fn verify");
        let (sig1, body1, _) = sig_changed.hashes(span, "fn verify");
        let (sig2, body2, _) = body_changed.hashes(span, "fn verify");

        assert_ne!(
            sig0, sig1,
            "declaration line must change the signature hash"
        );
        assert_eq!(body0, body1, "an untouched body must keep its hash");
        assert_eq!(sig0, sig2, "an untouched declaration must keep its hash");
        assert_ne!(body0, body2, "a changed body must change the body hash");
    }

    #[test]
    fn a_named_anchor_without_a_grammar_is_unparsed() {
        let src = Source::new("func verify() {}\n", "Auth.swift");
        assert!(matches!(
            src.resolve_detail("func verify"),
            Resolution::Unparsed
        ));
    }

    #[test]
    fn a_file_anchor_without_a_grammar_still_resolves() {
        let src = Source::new("func verify() {}\n", "Auth.swift");
        assert!(matches!(
            src.resolve_detail("@file"),
            Resolution::Found(Span { start: 1, end: 1 })
        ));
    }

    #[test]
    fn a_missing_anchor_with_a_grammar_is_not_found() {
        let src = Source::new(RS_SRC, "src/auth.rs");
        assert!(matches!(
            src.resolve_detail("fn verfy"),
            Resolution::NotFound
        ));
    }
}
