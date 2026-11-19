// src/code/treesitter.rs

use tree_sitter::Language;
use tree_sitter::Node;
use tree_sitter::Parser;

use super::CodeChunk;
use crate::types::LineRange;
use crate::types::SymbolName;

pub(super) fn chunk_with_tree_sitter(
    source: &str,
    lines: &[&str],
    language: Language,
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
    // Unwrap Python `@decorator` or JS `export` wrapper nodes to inspect the inner definition
    let target = match node.kind() {
        "decorated_definition" => node.child_by_field_name("definition").unwrap_or(node),
        "export_statement" => node.child_by_field_name("declaration").unwrap_or(node),
        _ => node,
    };

    // Most tree-sitter grammars expose a `name` field on functions, structs, classes, and modules
    if let Some(name_node) = target.child_by_field_name("name")
        && let Ok(text) = name_node.utf8_text(source.as_bytes())
        && let Ok(sym) = SymbolName::try_from(text)
    {
        return Some(sym);
    }

    // For Rust `impl Trait for Type` or `impl Type`, inspect `type` field
    if target.kind() == "impl_item"
        && let Some(type_node) = target.child_by_field_name("type")
        && let Ok(text) = type_node.utf8_text(source.as_bytes())
        && let Ok(sym) = SymbolName::try_from(text)
    {
        return Some(sym);
    }

    // For Go `type Foo struct { ... }` (`type_declaration` -> `type_spec` -> `name`)
    if target.kind() == "type_declaration" {
        let mut cursor = target.walk();
        for child in target.children(&mut cursor) {
            if child.kind() == "type_spec"
                && let Some(name_node) = child.child_by_field_name("name")
                && let Ok(text) = name_node.utf8_text(source.as_bytes())
                && let Ok(sym) = SymbolName::try_from(text)
            {
                return Some(sym);
            }
        }
    }

    // For Just recipes (`recipe` -> `recipe_header` -> `name`)
    if target.kind() == "recipe" {
        let mut cursor = target.walk();
        for child in target.children(&mut cursor) {
            if child.kind() == "recipe_header"
                && let Some(name_node) = child.child_by_field_name("name")
                && let Ok(text) = name_node.utf8_text(source.as_bytes())
                && let Ok(sym) = SymbolName::try_from(text)
            {
                return Some(sym);
            }
        }
    }

    None
}
