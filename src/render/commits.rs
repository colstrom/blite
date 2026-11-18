// src/render/commits.rs

use crate::types::CommitHit;

pub(super) fn render_commit_hits(query: &str, hits: &[CommitHit]) -> String {
    if hits.is_empty() {
        return format!("No matching commits found for \"{query}\".\n");
    }

    let mut out = String::new();
    for (idx, hit) in hits.iter().enumerate() {
        let rank = idx + 1;
        out.push_str(&format!(
            "\x1b[1;33m[{rank}] commit {}\x1b[0m \x1b[2mby {} (score: {:.2})\x1b[0m\n",
            hit.short_oid, hit.author, hit.score
        ));

        for line in hit.message.lines() {
            out.push_str(&format!("    {line}\n"));
        }

        if !hit.touched_paths.is_empty() {
            out.push_str("  \x1b[2m↳ Touched files:\x1b[0m\n");
            for path in &hit.touched_paths {
                out.push_str(&format!("    \x1b[36m{path}\x1b[0m\n"));
            }
        }

        out.push('\n');
    }

    out
}
