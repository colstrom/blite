// src/render/diagnostics.rs

use crate::command::diagnostics::DiagnosticsPayload;

pub fn render_diagnostics(payload: &DiagnosticsPayload) -> String {
    let mut out = String::new();
    let model_label = payload.model.as_deref().unwrap_or("none (using ~4 chars/token estimate)");
    out.push_str(&format!("\x1b[1;36mRepository Diagnostics:\x1b[0m {}\n", payload.repo));
    out.push_str(&format!(
        "  Chunks: {} | Total Size: {} bytes | Model: {}\n",
        payload.total_chunks, payload.total_bytes, model_label
    ));
    out.push_str(&format!(
        "  Context Window: {} tokens | Batch Size: {} | Multi-Doc Chunks: {} | Truncated Chunks: {}\n\n",
        payload.max_context_tokens, payload.batch_size, payload.multi_doc_chunks, payload.truncated_count
    ));

    for (idx, entry) in payload.entries.iter().enumerate() {
        let rank = idx + 1;
        let tok_suffix = if entry.exact_tokens { "tok" } else { "~tok" };
        let multi_tag = if entry.multi_doc { " \x1b[35m[multi-doc]\x1b[0m" } else { "" };
        let status = if entry.larger_than_batch_budget {
            "\x1b[1;31m[exceeds full batch budget + truncated]\x1b[0m"
        } else if entry.truncated {
            "\x1b[1;33m[truncated]\x1b[0m"
        } else if entry.pad_tokens > 0 {
            "\x1b[2m[padded]\x1b[0m"
        } else {
            "\x1b[1;32m[exact]\x1b[0m"
        };

        out.push_str(&format!(
            "  {:>2}. \x1b[36m{}\x1b[0m — {} {}, {} bytes (pad: +{}){multi_tag} {status}\n",
            rank, entry.path, entry.tokens, tok_suffix, entry.bytes, entry.pad_tokens
        ));
    }

    out
}
