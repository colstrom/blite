// src/command/embeddings/exec.rs

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

use serde::Deserialize;
use serde::Serialize;

use super::EmbeddingsBuildArgs;
use super::EmbeddingsClearArgs;
use super::EmbeddingsError;
use super::EmbeddingsPruneArgs;
use crate::embed::EMBED_BATCH_SIZE;
use crate::embed::Embedder;
use crate::embed::compute_doc_embed_hash;
use crate::embed::embedding_cache_path;
use crate::embed::find_model_spec;
use crate::embed::hash_doc_with_prefix;
use crate::embed::list_embedding_cache_files;
use crate::embed::prune_embedding_cache_file;
use crate::embed::read_embedding_cache;
use crate::embed::resolve_model_target;
use crate::embed::stderr_embed_progress;
use crate::embed::write_embedding_cache;
use crate::index::collect_embeddable_docs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsPayload {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub total_chunks: usize,
    pub cached_hits: usize,
    pub newly_computed: usize,
    pub removed_entries: usize,
    pub removed_files: usize,
}

pub(super) fn execute_build(args: &EmbeddingsBuildArgs) -> Result<EmbeddingsPayload, EmbeddingsError> {
    let start = Instant::now();
    let resolved = resolve_model_target(args.model.as_deref(), args.use_hf_cache)?;
    let docs = collect_embeddable_docs(&args.target.repo, args.filter.include_vendor, args.filter.untracked)?;

    let embedder = Embedder::load(&resolved)?;
    let cache_path = embedding_cache_path(&resolved.label);
    let mut cache = if args.force {
        HashMap::new()
    } else {
        cache_path.as_deref().map(read_embedding_cache).unwrap_or_default()
    };

    let mut uncached_docs = Vec::new();
    let mut uncached_hashes = Vec::new();
    let mut cached_hits = 0usize;

    for doc in &docs {
        let hash = compute_doc_embed_hash(&embedder, &doc.text);
        if cache.contains_key(&hash) {
            cached_hits += 1;
        } else {
            uncached_hashes.push(hash);
            uncached_docs.push((doc.label.as_str(), doc.text.as_str()));
        }
    }

    let newly_computed = uncached_docs.len();
    if !uncached_docs.is_empty() {
        let fresh = embedder.embed_documents_with_progress(&uncached_docs, EMBED_BATCH_SIZE, stderr_embed_progress(start, true))?;
        for (hash, vec) in uncached_hashes.into_iter().zip(fresh) {
            cache.insert(hash, vec);
        }
        if let Some(path) = &cache_path {
            write_embedding_cache(path, &cache);
        }
    }

    eprintln!(
        "\x1b[1;32mEmbeddings ready\x1b[0m for '{}' on {} in {:.1?} ({} total, {} cached, {} computed)",
        resolved.label,
        embedder.device_label(),
        start.elapsed(),
        docs.len(),
        cached_hits,
        newly_computed,
    );

    Ok(EmbeddingsPayload {
        action: "build".to_string(),
        model: Some(resolved.label),
        total_chunks: docs.len(),
        cached_hits,
        newly_computed,
        removed_entries: 0,
        removed_files: 0,
    })
}

pub(super) fn execute_prune(args: &EmbeddingsPruneArgs) -> Result<EmbeddingsPayload, EmbeddingsError> {
    let docs = collect_embeddable_docs(&args.target.repo, args.filter.include_vendor, args.filter.untracked)?;
    let mut removed_entries = 0usize;
    let mut removed_files = 0usize;
    let mut model_label = None;

    if args.all {
        // Match live hashes against both `doc_prefix: ""` and `doc_prefix: "search_document: "` (Nomic)
        let mut live_hashes = HashSet::with_capacity(docs.len() * 2);
        for doc in &docs {
            live_hashes.insert(hash_doc_with_prefix("", &doc.text, 512));
            live_hashes.insert(hash_doc_with_prefix("search_document: ", &doc.text, 512));
        }

        for path in list_embedding_cache_files() {
            let stats = prune_embedding_cache_file(&path, &live_hashes);
            removed_entries += stats.removed_entries;
            if stats.removed_file {
                removed_files += 1;
            }
        }
    } else {
        let resolved = resolve_model_target(args.model.as_deref(), args.use_hf_cache)?;
        let embedder = Embedder::load(&resolved)?;
        model_label = Some(resolved.label.clone());

        let live_hashes: HashSet<u64> = docs.iter().map(|d| compute_doc_embed_hash(&embedder, &d.text)).collect();

        if let Some(cache_path) = embedding_cache_path(&resolved.label) {
            let stats = prune_embedding_cache_file(&cache_path, &live_hashes);
            removed_entries = stats.removed_entries;
            removed_files = usize::from(stats.removed_file);
        }
    }

    eprintln!(
        "Pruned {} stale cached embedding(s) ({} empty cache file(s) removed).",
        removed_entries, removed_files
    );

    Ok(EmbeddingsPayload {
        action: "prune".to_string(),
        model: model_label,
        total_chunks: docs.len(),
        cached_hits: 0,
        newly_computed: 0,
        removed_entries,
        removed_files,
    })
}

pub(super) fn execute_clear(args: &EmbeddingsClearArgs) -> Result<EmbeddingsPayload, EmbeddingsError> {
    let mut removed_files = 0usize;
    let mut model_label = None;

    if args.all {
        for path in list_embedding_cache_files() {
            if fs_err::remove_file(&path).is_ok() {
                removed_files += 1;
            }
        }
    } else if let Some(target) = &args.model {
        let label = find_model_spec(target).map(|s| s.id.to_string()).unwrap_or_else(|| target.clone());
        model_label = Some(label.clone());

        if let Some(cache_path) = embedding_cache_path(&label)
            && cache_path.exists()
        {
            fs_err::remove_file(&cache_path)?;
            removed_files = 1;
        }
    }

    eprintln!("Cleared {} cached embedding file(s).", removed_files);

    Ok(EmbeddingsPayload {
        action: "clear".to_string(),
        model: model_label,
        total_chunks: 0,
        cached_hits: 0,
        newly_computed: 0,
        removed_entries: 0,
        removed_files,
    })
}
