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

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use super::WHOLE_FILE_LINE_THRESHOLD;
    use super::chunk_code_file;
    use crate::types::LineRange;

    fn pad_lines(prefix: &str, total_lines: usize) -> String {
        let mut out = prefix.to_string();
        let current = out.lines().count();
        for i in current..total_lines {
            out.push_str(&format!("\n// padding line {i}"));
        }
        out
    }

    #[test]
    fn skips_chunking_for_small_files() {
        let short_rs = "/// Doc\npub fn alpha() {}\n\npub fn beta() {}\n";
        assert!(short_rs.lines().count() <= WHOLE_FILE_LINE_THRESHOLD);
        assert!(chunk_code_file(Utf8Path::new("src/lib.rs"), short_rs).is_empty());
    }

    #[test]
    fn chunks_rust_items_with_doc_comments_and_attributes() {
        let header = "\
use std::fmt;

/// Greets a user by name.
#[inline]
pub fn greet(name: &str) -> String {
    format!(\"hi {name}\")
}

pub struct Widget {
    id: u64,
}

impl fmt::Display for Widget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, \"{}\", self.id)
    }
}
";
        let source = pad_lines(header, WHOLE_FILE_LINE_THRESHOLD + 5);
        let chunks = chunk_code_file(Utf8Path::new("src/widget.rs"), &source);
        assert_eq!(chunks.len(), 3);

        let greet = chunks.first().unwrap();
        assert_eq!(greet.symbol.as_deref(), Some("greet"));
        assert_eq!(greet.lines, LineRange { start: 3, end: 7 });
        assert!(greet.content.starts_with("/// Greets a user by name."));

        let widget = chunks.get(1).unwrap();
        assert_eq!(widget.symbol.as_deref(), Some("Widget"));
        assert_eq!(widget.lines, LineRange { start: 9, end: 11 });

        let display_impl = chunks.get(2).unwrap();
        assert_eq!(display_impl.symbol.as_deref(), Some("Widget"));
        assert_eq!(display_impl.lines, LineRange { start: 13, end: 17 });
    }

    #[test]
    fn unwraps_python_decorators_and_detects_shebangs() {
        let script = "\
#!/usr/bin/env python3

# Runs the nightly sync
@staticmethod
def sync_all():
    return True

class Runner:
    pass
";
        let padded = pad_lines(script, WHOLE_FILE_LINE_THRESHOLD + 5);
        let chunks = chunk_code_file(Utf8Path::new("bin/nightly-sync"), &padded);
        assert_eq!(chunks.len(), 2);

        let func = chunks.first().unwrap();
        assert_eq!(func.symbol.as_deref(), Some("sync_all"));
        assert_eq!(func.lines, LineRange { start: 3, end: 6 });

        let cls = chunks.get(1).unwrap();
        assert_eq!(cls.symbol.as_deref(), Some("Runner"));
        assert_eq!(cls.lines, LineRange { start: 8, end: 9 });
    }

    #[test]
    fn chunks_justfile_recipes_and_jq_filters() {
        let justfile = "\
# Build release binary
build:
    cargo build --release

test:
    cargo test
";
        let padded_just = pad_lines(justfile, WHOLE_FILE_LINE_THRESHOLD + 5);
        let just_chunks = chunk_code_file(Utf8Path::new("Justfile"), &padded_just);
        assert_eq!(just_chunks.len(), 2);
        assert_eq!(just_chunks.first().unwrap().symbol.as_deref(), Some("build"));
        assert_eq!(just_chunks.get(1).unwrap().symbol.as_deref(), Some("test"));

        let jq_script = "\
# Normalize labels
def normalize_labels:
  .metadata.labels // {};

def extract_name:
  .metadata.name;
";
        let padded_jq = pad_lines(jq_script, WHOLE_FILE_LINE_THRESHOLD + 5);
        let jq_chunks = chunk_code_file(Utf8Path::new("filters/k8s.jq"), &padded_jq);
        assert_eq!(jq_chunks.len(), 2);
        assert_eq!(jq_chunks.first().unwrap().symbol.as_deref(), Some("normalize_labels"));
        assert_eq!(jq_chunks.first().unwrap().lines, LineRange { start: 1, end: 4 });
        assert_eq!(jq_chunks.get(1).unwrap().symbol.as_deref(), Some("extract_name"));
    }
}
