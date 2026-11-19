// src/code/filter.rs

use super::CodeChunk;
use crate::types::LineRange;
use crate::types::SymbolName;

/// Lightweight function/def chunker for `.awk`, `.jq`, and `.yq` scripts.
pub(super) fn chunk_filter_script(lines: &[&str]) -> Vec<CodeChunk> {
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
