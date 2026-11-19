// src/code/mod.rs

mod filter;
mod treesitter;

use camino::Utf8Path;
use filter::chunk_filter_script;
use treesitter::chunk_with_tree_sitter;

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
    Python,
    Go,
    JavaScript,
    Just,
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
        ScriptKind::Python => chunk_with_tree_sitter(
            content,
            &lines,
            tree_sitter_python::LANGUAGE.into(),
            &["function_definition", "class_definition", "decorated_definition"],
            &["comment"],
        ),
        ScriptKind::Go => chunk_with_tree_sitter(
            content,
            &lines,
            tree_sitter_go::LANGUAGE.into(),
            &["function_declaration", "method_declaration", "type_declaration"],
            &["comment"],
        ),
        ScriptKind::JavaScript => chunk_with_tree_sitter(
            content,
            &lines,
            tree_sitter_javascript::LANGUAGE.into(),
            &["function_declaration", "class_declaration", "export_statement"],
            &["comment"],
        ),
        ScriptKind::Just => chunk_with_tree_sitter(
            content,
            &lines,
            tree_sitter_just::LANGUAGE.into(),
            &["recipe"],
            &["comment", "attribute"],
        ),
        ScriptKind::FilterScript => chunk_filter_script(&lines),
    };

    // If a large file had no extractable top-level definitions, fall back to whole-file indexing
    if chunks.is_empty() { Vec::new() } else { chunks }
}

fn detect_script_kind(path: &Utf8Path, content: &str) -> Option<ScriptKind> {
    if let Some(file_name) = path.file_name()
        && matches!(file_name.to_ascii_lowercase().as_str(), "justfile" | ".justfile")
    {
        return Some(ScriptKind::Just);
    }

    if let Some(ext) = path.extension() {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => return Some(ScriptKind::Rust),
            "rb" | "rake" => return Some(ScriptKind::Ruby),
            "sh" | "bash" | "zsh" => return Some(ScriptKind::Bash),
            "py" | "pyi" => return Some(ScriptKind::Python),
            "go" => return Some(ScriptKind::Go),
            "js" | "mjs" | "cjs" => return Some(ScriptKind::JavaScript),
            "just" => return Some(ScriptKind::Just),
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
        if shebang.contains("python") {
            return Some(ScriptKind::Python);
        }
        if shebang.contains("node") || shebang.contains("deno") || shebang.contains("bun") {
            return Some(ScriptKind::JavaScript);
        }
        if shebang.contains("just") {
            return Some(ScriptKind::Just);
        }
        if shebang.contains("awk") || shebang.contains("jq") || shebang.contains("yq") {
            return Some(ScriptKind::FilterScript);
        }
    }

    None
}
