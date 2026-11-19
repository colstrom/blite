// src/embed/hnsw.rs

use instant_distance::Builder as HnswBuilder;
use instant_distance::HnswMap;
use instant_distance::Point;
use instant_distance::Search;
use tracing::instrument;

use super::EMBED_BATCH_SIZE;
use super::EmbedError;
use super::Embedder;
use super::ResolvedModel;
use super::cache::compute_doc_embed_hash;
use super::cache::embedding_cache_path;
use super::cache::read_embedding_cache;
use super::cache::write_embedding_cache;

#[derive(Debug, Clone)]
struct EmbeddingPoint(Vec<f32>);

impl Point for EmbeddingPoint {
    fn distance(&self, other: &Self) -> f32 {
        let dot: f32 = self.0.iter().zip(other.0.iter()).map(|(a, b)| a * b).sum();
        (1.0 - dot).max(0.0)
    }
}

pub struct SemanticIndex {
    embedder: Embedder,
    hnsw: HnswMap<EmbeddingPoint, usize>,
    pub model_label: String,
    pub device_label: &'static str,
}

impl SemanticIndex {
    #[instrument(skip_all, fields(doc_count = docs.len(), model = %resolved.label, use_cache))]
    pub fn build_with_progress<F>(
        resolved: &ResolvedModel,
        docs: &[(String, String)],
        use_cache: bool,
        mut on_progress: F,
    ) -> Result<Self, EmbedError>
    where
        F: FnMut(usize, usize, &str),
    {
        let embedder = Embedder::load(resolved)?;
        let device_label = embedder.device_label();

        let cache_path = if use_cache { embedding_cache_path(&resolved.label) } else { None };
        let mut cache = cache_path.as_deref().map(read_embedding_cache).unwrap_or_default();

        let mut vectors = vec![Vec::new(); docs.len()];
        let mut uncached_indices = Vec::new();
        let mut uncached_docs = Vec::new();
        let mut doc_hashes = Vec::with_capacity(docs.len());

        for (idx, (label, text)) in docs.iter().enumerate() {
            let hash = compute_doc_embed_hash(&embedder, text);
            doc_hashes.push(hash);

            if let Some(cached_vec) = cache.get(&hash) {
                vectors[idx] = cached_vec.clone();
            } else {
                uncached_indices.push(idx);
                uncached_docs.push((label.as_str(), text.as_str()));
            }
        }

        if !uncached_docs.is_empty() {
            let fresh_vecs = embedder.embed_documents_with_progress(&uncached_docs, EMBED_BATCH_SIZE, &mut on_progress)?;
            for (orig_idx, vec) in uncached_indices.into_iter().zip(fresh_vecs) {
                cache.insert(doc_hashes[orig_idx], vec.clone());
                vectors[orig_idx] = vec;
            }
            if let Some(path) = &cache_path {
                write_embedding_cache(path, &cache);
            }
        }

        let points: Vec<EmbeddingPoint> = vectors.into_iter().map(EmbeddingPoint).collect();
        let values: Vec<usize> = (0..points.len()).collect();
        let hnsw = HnswBuilder::default().build(points, values);

        Ok(Self {
            embedder,
            hnsw,
            model_label: resolved.label.clone(),
            device_label,
        })
    }

    /// Returns `(doc_idx, cosine_similarity)` pairs ordered highest similarity first.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(usize, f32)>, EmbedError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let q_vec = self.embedder.embed_query(query)?;
        let q_point = EmbeddingPoint(q_vec);
        let mut search = Search::default();
        let results = self.hnsw.search(&q_point, &mut search);

        Ok(results
            .take(limit)
            .map(|item| (*item.value, (1.0 - item.distance).clamp(-1.0, 1.0)))
            .collect())
    }
}
