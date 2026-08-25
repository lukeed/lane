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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Anchor {
    pub(crate) value: String,
    pub(crate) span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Qualification {
    Canonical(Anchor),
    Ambiguous(Vec<Anchor>),
    NotFound,
    Unparsed,
}

struct Grammar {
    language: fn() -> Language,
    decls: &'static str,
    kind: fn(&str, &str) -> Option<&'static str>,
    query: &'static OnceLock<Option<Query>>,
}

fn rust_kind(node: &str, _: &str) -> Option<&'static str> {
    Some(match node {
        "function_item" => "fn",
        "struct_item" => "struct",
        "enum_item" => "enum",
        "trait_item" => "trait",
        "type_item" => "type",
        "mod_item" => "mod",
        "const_item" => "const",
        "static_item" => "static",
        "macro_definition" => "macro",
        "impl_item" => "impl",
        _ => return None,
    })
}

fn go_kind(node: &str, _: &str) -> Option<&'static str> {
    Some(match node {
        "function_declaration" | "method_declaration" => "func",
        "type_declaration" => "type",
        "const_declaration" => "const",
        "var_declaration" => "var",
        _ => return None,
    })
}

fn python_kind(node: &str, _: &str) -> Option<&'static str> {
    Some(match node {
        "function_definition" => "def",
        "class_definition" => "class",
        _ => return None,
    })
}

fn javascript_kind(node: &str, line: &str) -> Option<&'static str> {
    Some(match node {
        "function_declaration" | "generator_function_declaration" => "function",
        "class_declaration" => "class",
        "method_definition" => "method",
        "lexical_declaration" if has_word(line, "const") => "const",
        "lexical_declaration" if has_word(line, "let") => "let",
        "variable_declaration" => "var",
        "interface_declaration" => "interface",
        "type_alias_declaration" => "type",
        "enum_declaration" => "enum",
        _ => return None,
    })
}

fn c_kind(node: &str, _: &str) -> Option<&'static str> {
    Some(match node {
        "function_definition" | "declaration" => "fn",
        "struct_specifier" => "struct",
        "union_specifier" => "union",
        "enum_specifier" => "enum",
        "type_definition" => "type",
        _ => return None,
    })
}

fn java_kind(node: &str, _: &str) -> Option<&'static str> {
    Some(match node {
        "method_declaration" => "method",
        "constructor_declaration" => "constructor",
        "class_declaration" => "class",
        "interface_declaration" => "interface",
        "enum_declaration" => "enum",
        "record_declaration" => "record",
        _ => return None,
    })
}

fn bash_kind(node: &str, _: &str) -> Option<&'static str> {
    (node == "function_definition").then_some("function")
}

fn no_kind(_: &str, _: &str) -> Option<&'static str> {
    None
}

static RUST_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static RUST: Grammar = Grammar {
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
    kind: rust_kind,
    query: &RUST_QUERY,
};

static GO_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static GO: Grammar = Grammar {
    language: || tree_sitter_go::LANGUAGE.into(),
    decls: r#"
        (function_declaration name: (identifier) @name) @decl
        (method_declaration name: (field_identifier) @name) @decl
        (type_declaration (type_spec name: (type_identifier) @name)) @decl
        (const_declaration (const_spec name: (identifier) @name)) @decl
        (var_declaration (var_spec name: (identifier) @name)) @decl
    "#,
    kind: go_kind,
    query: &GO_QUERY,
};

static PYTHON_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static PYTHON: Grammar = Grammar {
    language: || tree_sitter_python::LANGUAGE.into(),
    decls: r#"
        (function_definition name: (identifier) @name) @decl
        (class_definition name: (identifier) @name) @decl
    "#,
    kind: python_kind,
    query: &PYTHON_QUERY,
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

static JAVA_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static JAVA: Grammar = Grammar {
    language: || tree_sitter_java::LANGUAGE.into(),
    decls: r#"
        (method_declaration name: (identifier) @name) @decl
        (constructor_declaration name: (identifier) @name) @decl
        (class_declaration name: (identifier) @name) @decl
        (interface_declaration name: (identifier) @name) @decl
        (enum_declaration name: (identifier) @name) @decl
        (record_declaration name: (identifier) @name) @decl
    "#,
    kind: java_kind,
    query: &JAVA_QUERY,
};

static BASH_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static BASH: Grammar = Grammar {
    language: || tree_sitter_bash::LANGUAGE.into(),
    decls: r#"(function_definition name: (word) @name) @decl"#,
    kind: bash_kind,
    query: &BASH_QUERY,
};

/// css/html/markdown carry no name anchors; they are here for block, heading and comment work.
static CSS_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static CSS: Grammar = Grammar {
    language: || tree_sitter_css::LANGUAGE.into(),
    decls: "",
    kind: no_kind,
    query: &CSS_QUERY,
};

static HTML_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static HTML: Grammar = Grammar {
    language: || tree_sitter_html::LANGUAGE.into(),
    decls: "",
    kind: no_kind,
    query: &HTML_QUERY,
};

static MARKDOWN_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static MARKDOWN: Grammar = Grammar {
    language: || tree_sitter_md::LANGUAGE.into(),
    decls: "",
    kind: no_kind,
    query: &MARKDOWN_QUERY,
};

static JS_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static JS: Grammar = Grammar {
    language: || tree_sitter_javascript::LANGUAGE.into(),
    decls: JS_DECLS,
    kind: javascript_kind,
    query: &JS_QUERY,
};

static TYPESCRIPT_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static TYPESCRIPT: Grammar = Grammar {
    language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    decls: TS_DECLS,
    kind: javascript_kind,
    query: &TYPESCRIPT_QUERY,
};

static TSX_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static TSX: Grammar = Grammar {
    language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
    decls: TS_DECLS,
    kind: javascript_kind,
    query: &TSX_QUERY,
};

static C_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static C: Grammar = Grammar {
    language: || tree_sitter_c::LANGUAGE.into(),
    decls: C_DECLS,
    kind: c_kind,
    query: &C_QUERY,
};

static CPP_QUERY: OnceLock<Option<Query>> = OnceLock::new();
static CPP: Grammar = Grammar {
    language: || tree_sitter_cpp::LANGUAGE.into(),
    decls: C_DECLS,
    kind: c_kind,
    query: &CPP_QUERY,
};

fn grammar_for(ext: &str) -> Option<&'static Grammar> {
    Some(match ext {
        "rs" => &RUST,
        "go" => &GO,
        "py" | "pyi" => &PYTHON,
        "js" | "mjs" | "cjs" | "jsx" => &JS,
        "ts" | "mts" | "cts" => &TYPESCRIPT,
        "tsx" => &TSX,
        "c" | "h" => &C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => &CPP,
        "java" => &JAVA,
        "sh" | "bash" | "zsh" => &BASH,
        "css" => &CSS,
        // An SFC is three languages in one; html locates the blocks, the anchor picks the rest.
        "html" | "htm" | "svelte" | "vue" => &HTML,
        "md" | "markdown" => &MARKDOWN,
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

/// Byte offsets for the starts of the lines that `str::lines()` reports.
fn line_starts(text: &str) -> Vec<usize> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut starts = vec![0];
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'\n' && offset + 1 < text.len() {
            starts.push(offset + 1);
        }
    }
    starts
}

/// One parsed file, reused across every note anchored to it.
pub struct Source {
    text: String,
    line_starts: Vec<usize>,
    ext: String,
    grammar: Option<&'static Grammar>,
    tree: Option<Tree>,
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
            line_starts: line_starts(text),
            ext,
            grammar,
            tree,
        }
    }

    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// A line with the same terminator handling as `str::lines()`.
    fn line_at(&self, row: usize) -> Option<&str> {
        let start = *self.line_starts.get(row)?;
        let end = self
            .line_starts
            .get(row + 1)
            .copied()
            .unwrap_or(self.text.len());
        let line = &self.text[start..end];
        if let Some(line) = line.strip_suffix('\n') {
            return Some(line.strip_suffix('\r').unwrap_or(line));
        }
        Some(line)
    }

    /// Byte range covering a 1-indexed inclusive line span.
    fn byte_range(&self, span: Span) -> Range<usize> {
        let start = span
            .start
            .checked_sub(1)
            .and_then(|line| self.line_starts.get(line))
            .copied()
            .unwrap_or(self.text.len());
        let end = span
            .end
            .checked_sub(1)
            .and_then(|line| {
                self.line_starts
                    .get(line + 1)
                    .copied()
                    .or_else(|| self.line_starts.get(line).map(|_| self.text.len()))
            })
            .unwrap_or(self.text.len());
        start.min(end)..end
    }

    pub fn span_text(&self, span: Span) -> String {
        self.text[self.byte_range(span)].to_string()
    }

    fn file_anchor(&self) -> Anchor {
        Anchor {
            value: "@file".to_string(),
            span: Span {
                start: 1,
                end: self.line_count().max(1),
            },
        }
    }

    pub(crate) fn anchors(&self) -> Vec<Anchor> {
        let mut rest = self.block_candidates();
        rest.extend(self.heading_candidates());
        rest.extend(self.declaration_candidates());
        rest.sort_by(|a, b| {
            a.span
                .start
                .cmp(&b.span.start)
                .then_with(|| a.span.end.cmp(&b.span.end))
                .then_with(|| a.value.cmp(&b.value))
        });
        let mut seen = std::collections::HashSet::new();
        rest.retain(|anchor| seen.insert(anchor.value.clone()));

        let mut anchors = Vec::with_capacity(rest.len() + 1);
        anchors.push(self.file_anchor());
        anchors.extend(rest);
        anchors
    }

    pub(crate) fn qualify(&self, anchor: &str) -> Qualification {
        let value = anchor.trim();
        let anchors = self.anchors();
        if let Some(candidate) = anchors.iter().find(|candidate| candidate.value == value) {
            return Qualification::Canonical(candidate.clone());
        }
        if self.grammar.is_none() {
            return Qualification::Unparsed;
        }
        if value.starts_with('#') || value.split_whitespace().count() != 1 {
            return Qualification::NotFound;
        }

        let candidates: Vec<_> = anchors
            .into_iter()
            .filter(|candidate| {
                candidate.value != "@file"
                    && !candidate.value.starts_with('#')
                    && candidate.value.split_whitespace().last() == Some(value)
            })
            .collect();
        match candidates.len() {
            0 => Qualification::NotFound,
            1 => Qualification::Canonical(candidates.into_iter().next().unwrap()),
            _ => Qualification::Ambiguous(candidates),
        }
    }

    pub fn resolve_detail(&self, anchor: &str) -> Resolution {
        let a = anchor.trim();
        if a.is_empty() || a == "@file" || a == "*" {
            return Resolution::Found(self.file_anchor().span);
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
        self.block_candidates()
            .into_iter()
            .find(|anchor| {
                anchor
                    .value
                    .strip_prefix('#')
                    .is_some_and(|value| value.eq_ignore_ascii_case(block))
            })
            .map(|anchor| anchor.span)
    }

    fn block_candidates(&self) -> Vec<Anchor> {
        if !is_sfc(&self.ext) {
            return Vec::new();
        }
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
        let mut found = Vec::new();
        walk(tree.root_node(), &mut |node| {
            // html gives script and style their own node kinds; template is a plain element.
            let value = match node.kind() {
                "script_element" => Some("#script"),
                "style_element" => Some("#style"),
                "element"
                    if element_tag(node, &self.text)
                        .is_some_and(|tag| tag.eq_ignore_ascii_case("template")) =>
                {
                    Some("#template")
                }
                _ => None,
            };
            if let Some(value) = value {
                found.push(Anchor {
                    value: value.to_string(),
                    span: node_span(node),
                });
            }
        });
        found
    }

    /// A markdown section: the heading plus everything until the next heading of its level or above.
    fn resolve_heading(&self, anchor: &str) -> Option<Span> {
        self.heading_candidates()
            .into_iter()
            .find(|candidate| candidate.value == anchor.trim())
            .map(|candidate| candidate.span)
    }

    fn heading_candidates(&self) -> Vec<Anchor> {
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
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

        headings
            .iter()
            .enumerate()
            .map(|(idx, (start, level, value))| {
                let end = headings[idx + 1..]
                    .iter()
                    .find(|(_, next_level, _)| *next_level <= *level)
                    .map(|(line, _, _)| line - 1)
                    .unwrap_or_else(|| self.line_count());
                Anchor {
                    value: value.clone(),
                    span: Span {
                        start: *start,
                        end: end.max(*start),
                    },
                }
            })
            .collect()
    }

    /// `fn verify` or a bare `verify`: the earliest declaration of that name in the file.
    fn resolve_decl(&self, anchor: &str) -> Option<Span> {
        let anchor = anchor.trim();
        let candidates = self.declaration_candidates();
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.value == anchor)
        {
            return Some(candidate.span);
        }
        if anchor.split_whitespace().count() != 1 {
            return None;
        }
        candidates
            .into_iter()
            .find(|candidate| candidate.value.split_whitespace().last() == Some(anchor))
            .map(|candidate| candidate.span)
    }

    fn declaration_candidates(&self) -> Vec<Anchor> {
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
        let Some(query) = self.decl_query() else {
            return Vec::new();
        };
        let Some(grammar) = self.grammar else {
            return Vec::new();
        };

        let Some(name_idx) = query.capture_index_for_name("name") else {
            return Vec::new();
        };
        let Some(decl_idx) = query.capture_index_for_name("decl") else {
            return Vec::new();
        };

        let mut found = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), self.text.as_bytes());
        while let Some(m) = matches.next() {
            let Some(name) = m
                .captures
                .iter()
                .find(|c| c.index == name_idx)
                .map(|capture| &self.text[capture.node.byte_range()])
            else {
                continue;
            };
            let Some(decl) = m.captures.iter().find(|c| c.index == decl_idx) else {
                continue;
            };
            let line = self
                .line_at(decl.node.start_position().row)
                .unwrap_or_default();
            let Some(kind) = (grammar.kind)(decl.node.kind(), line) else {
                continue;
            };
            found.push((
                decl.node.start_byte(),
                Anchor {
                    value: format!("{kind} {name}"),
                    span: node_span(decl.node),
                },
            ));
        }
        found.sort_by_key(|(start, _)| *start);
        found.into_iter().map(|(_, anchor)| anchor).collect()
    }

    fn decl_query(&self) -> Option<&Query> {
        let grammar = self.grammar?;
        grammar
            .query
            .get_or_init(|| {
                if grammar.decls.trim().is_empty() {
                    return None;
                }
                Query::new(&(grammar.language)(), grammar.decls).ok()
            })
            .as_ref()
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
    fn rust_anchors_are_canonical_and_in_source_order() {
        let text = "fn work() {}\n\
struct Item;\n\
enum Mode { One }\n\
trait Ready {}\n\
type Count = u8;\n\
mod nested {}\n\
const LIMIT: u8 = 1;\n\
static ENABLED: bool = true;\n\
macro_rules! build { () => {} }\n\
impl Item {}\n";
        let source = Source::new(text, "a.rs");
        let values: Vec<_> = source
            .anchors()
            .into_iter()
            .map(|anchor| anchor.value)
            .collect();
        assert_eq!(
            values,
            [
                "@file",
                "fn work",
                "struct Item",
                "enum Mode",
                "trait Ready",
                "type Count",
                "mod nested",
                "const LIMIT",
                "static ENABLED",
                "macro build",
                "impl Item",
            ]
        );
    }

    #[test]
    fn javascript_lexical_anchors_keep_const_and_let_distinct() {
        let source = Source::new(
            "export const first = 1;\nlet second = 2;\nvar third = 3;\n",
            "a.js",
        );
        let values: Vec<_> = source
            .anchors()
            .into_iter()
            .map(|anchor| anchor.value)
            .collect();
        assert_eq!(values, ["@file", "const first", "let second", "var third"]);
    }

    #[test]
    fn markdown_anchors_keep_exact_heading_text_and_section_spans() {
        let anchors = Source::new(MD_SRC, "guide.md").anchors();
        assert_eq!(
            anchors,
            [
                Anchor {
                    value: "@file".into(),
                    span: Span { start: 1, end: 13 },
                },
                Anchor {
                    value: "# Title".into(),
                    span: Span { start: 1, end: 13 },
                },
                Anchor {
                    value: "## Rate limiting".into(),
                    span: Span { start: 3, end: 10 },
                },
                Anchor {
                    value: "## Retries".into(),
                    span: Span { start: 11, end: 13 },
                },
            ]
        );
    }

    #[test]
    fn sfc_anchors_include_only_blocks_that_are_present() {
        let source = Source::new(
            "<template><main /></template>\n<script>let x = 1;</script>\n<style>main {}</style>\n",
            "View.vue",
        );
        let values: Vec<_> = source
            .anchors()
            .into_iter()
            .map(|anchor| anchor.value)
            .collect();
        assert_eq!(values, ["@file", "#template", "#script", "#style"]);
    }

    #[test]
    fn unparsed_files_have_only_the_file_anchor() {
        assert_eq!(
            Source::new("func work() {}\nreturn\n", "a.swift").anchors(),
            [Anchor {
                value: "@file".into(),
                span: Span { start: 1, end: 2 },
            }]
        );
    }

    #[test]
    fn empty_files_have_a_one_line_file_anchor() {
        assert_eq!(
            Source::new("", "a.rs").anchors(),
            [Anchor {
                value: "@file".into(),
                span: Span { start: 1, end: 1 },
            }]
        );
    }

    #[test]
    fn identical_anchor_values_keep_the_earliest_span() {
        let anchors = Source::new("fn run() {}\nfn run() {}\n", "a.rs").anchors();
        assert_eq!(anchors.len(), 2);
        assert_eq!(
            anchors[1],
            Anchor {
                value: "fn run".into(),
                span: Span { start: 1, end: 1 },
            }
        );
    }

    #[test]
    fn an_old_bare_anchor_still_resolves_to_the_earliest_declaration() {
        let source = Source::new("const run: u8 = 1;\nfn run() {}\n", "a.rs");
        assert_eq!(source.resolve("run"), Some(Span { start: 1, end: 1 }));
    }

    #[test]
    fn a_unique_bare_name_qualifies_to_its_canonical_anchor() {
        let source = Source::new("pub fn verify() {}\n", "a.rs");
        assert_eq!(
            source.qualify("verify"),
            Qualification::Canonical(Anchor {
                value: "fn verify".into(),
                span: Span { start: 1, end: 1 },
            })
        );
        assert!(matches!(source.qualify("*"), Qualification::NotFound));
        assert!(matches!(
            Source::new("## Rate limiting\n", "a.md").qualify("limiting"),
            Qualification::NotFound
        ));
    }

    #[test]
    fn a_bare_name_with_distinct_canonical_values_is_ambiguous() {
        let source = Source::new("fn run() {}\nconst run: u8 = 1;\n", "a.rs");
        let Qualification::Ambiguous(choices) = source.qualify("run") else {
            panic!("expected ambiguity");
        };
        assert_eq!(
            choices
                .into_iter()
                .map(|choice| choice.value)
                .collect::<Vec<_>>(),
            ["fn run", "const run"]
        );
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

    fn legacy_span_text(text: &str, span: Span) -> String {
        let mut start = text.len();
        let mut end = text.len();
        let mut offset = 0usize;
        for (i, line) in text.split_inclusive('\n').enumerate() {
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
        text[start.min(end)..end].to_string()
    }

    #[test]
    fn line_index_matches_str_lines_for_edge_cases() {
        for (text, count) in [
            ("", 0),
            ("one", 1),
            ("one\n", 1),
            ("one\r\n", 1),
            ("a\r\nb\r\n", 2),
            ("a\n\nb\n", 3),
            ("é 🙂\n中", 2),
        ] {
            let src = Source::new(text, "a.rs");
            assert_eq!(src.line_count(), count, "{text:?}");
            assert_eq!(src.line_count(), text.lines().count(), "{text:?}");
            for (row, line) in text.lines().enumerate() {
                assert_eq!(src.line_at(row), Some(line), "{text:?}, row {row}");
            }
        }
    }

    #[test]
    fn indexed_byte_ranges_preserve_legacy_span_text_exactly() {
        let text = "é 🙂\r\n\r\n中\n";
        let src = Source::new(text, "a.rs");
        for span in [
            Span { start: 1, end: 1 },
            Span { start: 2, end: 99 },
            Span { start: 99, end: 99 },
            Span { start: 3, end: 1 },
            Span { start: 1, end: 99 },
            Span { start: 0, end: 1 },
        ] {
            assert_eq!(
                src.span_text(span),
                legacy_span_text(text, span),
                "{span:?}"
            );
        }
    }

    #[test]
    fn declaration_queries_are_shared_per_grammar() {
        let rust_a = Source::new("fn shared() {}\n", "a.rs");
        let rust_b = Source::new("fn shared() {}\n", "b.rs");
        let go = Source::new("func shared() {}\n", "a.go");

        let rust_a_query = rust_a.decl_query().unwrap() as *const Query;
        let rust_b_query = rust_b.decl_query().unwrap() as *const Query;
        let go_query = go.decl_query().unwrap() as *const Query;
        assert_eq!(rust_a_query, rust_b_query);
        assert_ne!(rust_a_query, go_query);
        assert!(rust_a.resolve("fn shared").is_some());
        assert!(go.resolve("func shared").is_some());

        let html = Source::new("<p>nothing</p>", "a.html");
        assert!(html.decl_query().is_none());
        let unparsed = Source::new("func shared() {}", "a.swift");
        assert!(matches!(
            unparsed.resolve_detail("func shared"),
            Resolution::Unparsed
        ));
    }

    #[test]
    fn sfc_comment_language_remains_independent_of_the_html_grammar() {
        let source = Source::new("<script>\nlet x = 1;\n</script>\n", "E.svelte");
        assert_eq!(
            source.comment_language("#script"),
            Some(tree_sitter_javascript::LANGUAGE.into())
        );
        assert_eq!(
            source.comment_language("#style"),
            Some(tree_sitter_css::LANGUAGE.into())
        );
        assert_eq!(
            source.comment_language("#template"),
            Some(tree_sitter_html::LANGUAGE.into())
        );
    }
}
