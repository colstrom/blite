// src/index/fusion.rs

use std::collections::HashMap;

const RRF_K: f32 = 60.0;

pub(super) fn has_field_filter(query: &str) -> bool {
    ["kind:", "ns:", "name:", "labels:", "annotations:", "symbol:", "path:", "doc_index:"]
        .iter()
        .any(|prefix| query.contains(prefix))
}

/// Fuses ranked lexical `(doc_id)` and semantic `(doc_id, cosine_similarity)` candidate lists
/// using Reciprocal Rank Fusion (RRF), returning up to `limit` `(doc_id, fused_score)` pairs.
pub(super) fn rrf_fuse(
    lexical_ids: &[usize],
    semantic_hits: &[(usize, f32)],
    restrict_to_lexical: bool,
    limit: usize,
) -> Vec<(usize, f32)> {
    let mut rrf_scores: HashMap<usize, f32> = HashMap::with_capacity(lexical_ids.len() + semantic_hits.len());

    for (rank, &doc_id) in lexical_ids.iter().enumerate() {
        *rrf_scores.entry(doc_id).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f32);
    }

    for (rank, &(doc_id, _cosine)) in semantic_hits.iter().enumerate() {
        if restrict_to_lexical && !rrf_scores.contains_key(&doc_id) {
            continue;
        }
        *rrf_scores.entry(doc_id).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f32);
    }

    let mut ranked: Vec<(usize, f32)> = rrf_scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked.truncate(limit);
    ranked
}
