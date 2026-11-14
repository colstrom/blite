use camino::Utf8Path;
use tree_sitter::Node;
use tree_sitter::Parser;

use crate::types::LineRange;
use crate::types::SymbolName;

const WHOLE_FILE_LINE_THRESHOLD: usize = 80;

#[derive(Debug, Clone)]
pub struct CodeChunk {
    pub symbol: Option<SymbolName>,
    pub lines: LineRange,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptKind {
    Rust,
    Ruby,
    Bash,
    FilterScript, // .awk, .jq, .yq
}

/// Splits supported source/script files into AST-aligned chunks (with leading doc comments).
/// Returns an empty `Vec` if the file should be indexed as a single whole document.
pub fn chunk_code_file(path: &Utf8Path, content: &str) -> Vec<CodeChunk> {
    let Some(kind) = detect_script_kind(path, content) else {
        return Vec::new();
    };

    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= WHOLE_FILE_LINE_THRESHOLD {
        return Vec::new();
    }

    let chunks = match kind {
        ScriptKind::Rust => chunk_with_tree_sitter(
            content,
            &lines,
            tree_sitter_rust::LANGUAGE.into(),
            &["function_item", "struct_item", "enum_item", "trait_item", "impl_item", "mod_item"],
            &["line_comment", "block_comment", "attribute_item"],
        ),
        ScriptKind::Ruby => chunk_with_tree_sitter(
            content,
            &lines,
            tree_sitter_ruby::LANGUAGE.into(),
            &["method", "singleton_method", "class", "module"],
            &["comment"],
        ),
        ScriptKind::Bash => chunk_with_tree_sitter(
            content,
            &lines,
            tree_sitter_bash::LANGUAGE.into(),
            &["function_definition"],
            &["comment"],
        ),
        ScriptKind::FilterScript => chunk_filter_script(&lines),
    };

    // If a large file had no extractable top-level definitions, fall back to whole-file indexing
    if chunks.is_empty() { Vec::new() } else { chunks }
}

fn detect_script_kind(path: &Utf8Path, content: &str) -> Option<ScriptKind> {
    if let Some(ext) = path.extension() {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => return Some(ScriptKind::Rust),
            "rb" | "rake" => return Some(ScriptKind::Ruby),
            "sh" | "bash" | "zsh" => return Some(ScriptKind::Bash),
            "awk" | "jq" | "yq" => return Some(ScriptKind::FilterScript),
            _ => {}
        }
    }

    // Handle extensionless scripts in `bin/` or `scripts/` via shebang
    let first_line = content.lines().next()?.trim();
    if let Some(shebang) = first_line.strip_prefix("#!") {
        if shebang.contains("ruby") {
            return Some(ScriptKind::Ruby);
        }
        if shebang.contains("bash") || shebang.ends_with("/sh") || shebang.contains("env sh") {
            return Some(ScriptKind::Bash);
        }
        if shebang.contains("awk") || shebang.contains("jq") || shebang.contains("yq") {
            return Some(ScriptKind::FilterScript);
        }
    }

    None
}

fn chunk_with_tree_sitter(
    source: &str,
    lines: &[&str],
    language: tree_sitter::Language,
    item_kinds: &[&str],
    trivia_kinds: &[&str],
) -> Vec<CodeChunk> {
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }

    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };

    let root = tree.root_node();
    let mut cursor = root.walk();
    let children: Vec<Node<'_>> = root.children(&mut cursor).collect();
    let mut chunks = Vec::new();

    for (idx, node) in children.iter().enumerate() {
        if !item_kinds.contains(&node.kind()) {
            continue;
        }

        // Walk backward across contiguous doc comments / attributes directly attached to this item
        let mut start_row = node.start_position().row;
        let mut prev_idx = idx;
        while prev_idx > 0 {
            let prev = children[prev_idx - 1];
            let prev_end_row = prev.end_position().row;
            if trivia_kinds.contains(&prev.kind()) && start_row.saturating_sub(prev_end_row) <= 1 {
                start_row = prev.start_position().row;
                prev_idx -= 1;
            } else {
                break;
            }
        }

        let end_row = node.end_position().row.min(lines.len().saturating_sub(1));
        let snippet = lines[start_row..=end_row].join("\n");
        let symbol = extract_symbol_name(*node, source);

        chunks.push(CodeChunk {
            symbol,
            lines: LineRange {
                start: start_row + 1,
                end: end_row + 1,
            },
            content: snippet,
        });
    }

    chunks
}

fn extract_symbol_name(node: Node<'_>, source: &str) -> Option<SymbolName> {
    // Most tree-sitter grammars expose a `name` field on functions, structs, classes, and modules
    if let Some(name_node) = node.child_by_field_name("name")
        && let Ok(text) = name_node.utf8_text(source.as_bytes())
        && let Ok(sym) = SymbolName::try_from(text.to_string())
    {
        return Some(sym);
    }

    // For Rust `impl Trait for Type` or `impl Type`, inspect `type` field
    if node.kind() == "impl_item"
        && let Some(type_node) = node.child_by_field_name("type")
        && let Ok(text) = type_node.utf8_text(source.as_bytes())
        && let Ok(sym) = SymbolName::try_from(text.to_string())
    {
        return Some(sym);
    }

    None
}

/// Lightweight function/def chunker for `.awk`, `.jq`, and `.yq` scripts.
fn chunk_filter_script(lines: &[&str]) -> Vec<CodeChunk> {
    let mut def_starts = Vec::new();

    for (row, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("function ") || trimmed.starts_with("def ") {
            // Walk up contiguous `#` comments above the function
            let mut start = row;
            while start > 0 && lines[start - 1].trim_start().starts_with('#') {
                start -= 1;
            }
            def_starts.push((start, row));
        }
    }

    let mut chunks = Vec::new();
    for (i, &(start_row, sig_row)) in def_starts.iter().enumerate() {
        let end_row = if i + 1 < def_starts.len() {
            def_starts[i + 1].0.saturating_sub(1)
        } else {
            lines.len().saturating_sub(1)
        };

        let sig = lines[sig_row].trim_start();
        let raw_name = sig
            .strip_prefix("function ")
            .or_else(|| sig.strip_prefix("def "))
            .and_then(|rest| rest.split(['(', ':', ';', ' ', '\t']).next())
            .unwrap_or_default();

        let symbol = SymbolName::try_from(raw_name.to_string()).ok();

        chunks.push(CodeChunk {
            symbol,
            lines: LineRange {
                start: start_row + 1,
                end: end_row + 1,
            },
            content: lines[start_row..=end_row].join("\n"),
        });
    }

    chunks
}
