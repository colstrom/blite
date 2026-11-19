// src/embed/mod.rs

mod cache;
mod catalog;
mod download;
mod error;
mod hnsw;
mod resolve;

use std::time::Instant;

pub use cache::PruneFileStats;
pub use cache::compute_doc_embed_hash;
pub use cache::embedding_cache_dir;
pub use cache::embedding_cache_path;
pub use cache::hash_doc_with_prefix;
pub use cache::list_embedding_cache_files;
pub use cache::prune_embedding_cache_file;
pub use cache::read_embedding_cache;
pub use cache::write_embedding_cache;
use candle_core::DType;
use candle_core::Device;
use candle_core::Tensor;
use candle_nn::VarBuilder;
use candle_transformers::models::bert::BertModel;
use candle_transformers::models::bert::Config as BertConfig;
use candle_transformers::models::nomic_bert::Config as NomicBertConfig;
use candle_transformers::models::nomic_bert::NomicBertModel;
pub use catalog::MODEL_CATALOG;
pub use catalog::ModelSpec;
pub use catalog::find_model_spec;
pub use download::download_curated_model;
pub use error::EmbedError;
pub use hnsw::SemanticIndex;
pub use resolve::ModelResolveOptions;
pub use resolve::ModelSource;
pub use resolve::REQUIRED_MODEL_FILES;
pub use resolve::ResolvedModel;
pub use resolve::ValidatedModelDir;
pub use resolve::locate_curated_model;
pub use resolve::probe_hf_cache_snapshot;
pub use resolve::resolve_active_model;
pub use resolve::resolve_model_target;
use serde::Deserialize;
use tokenizers::PaddingParams;
use tokenizers::Tokenizer;
use tokenizers::TruncationParams;
use tracing::debug;
use tracing::instrument;

#[cfg(target_os = "macos")]
pub const EMBED_BATCH_SIZE: usize = 64;
#[cfg(not(target_os = "macos"))]
pub const EMBED_BATCH_SIZE: usize = 32;

/// Returns a progress callback that renders an in-place status line to `stderr`
/// when `enabled` is true, and clears the line once `done == total`.
pub fn stderr_embed_progress(start: Instant, enabled: bool) -> impl FnMut(usize, usize, &str) {
    move |done, total, path| {
        if !enabled {
            return;
        }
        if done < total {
            eprint!("\r\x1b[2KEmbedding [{done}/{total}] ({:.1?}) {path}", start.elapsed());
        } else {
            eprint!("\r\x1b[2K");
        }
    }
}

pub(super) fn truncate_for_tokenizer(text: &str, max_tokens: usize) -> &str {
    let byte_budget = max_tokens.saturating_mul(6);
    if text.len() <= byte_budget {
        return text;
    }
    let mut end = byte_budget;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

enum EncoderBackend {
    Bert(BertModel),
    NomicBert(NomicBertModel),
}

#[derive(Debug, Deserialize)]
struct RawArchProbe {
    #[serde(default)]
    model_type: String,
    #[serde(default)]
    architectures: Vec<String>,
    #[serde(default = "default_max_pos")]
    max_position_embeddings: usize,
}

fn default_max_pos() -> usize {
    512
}

pub struct Embedder {
    backend: EncoderBackend,
    tokenizer: Tokenizer,
    device: Device,
    max_tokens: usize,
    query_prefix: String,
    doc_prefix: String,
}

impl Embedder {
    #[instrument(skip_all, fields(model = %resolved.label, dir = %resolved.dir))]
    pub fn load(resolved: &ResolvedModel) -> Result<Self, EmbedError> {
        let device = select_device();
        let config_str = fs_err::read_to_string(resolved.dir.join("config.json"))?;
        let probe: RawArchProbe = serde_json::from_str(&config_str)?;

        let mut tokenizer =
            Tokenizer::from_file(resolved.dir.join("tokenizer.json").as_std_path()).map_err(|e| EmbedError::Tokenizer(e.to_string()))?;

        let max_tokens = probe.max_position_embeddings.min(512);
        tokenizer.with_padding(Some(PaddingParams::default()));
        let _ = tokenizer.with_truncation(Some(TruncationParams {
            max_length: max_tokens,
            ..Default::default()
        }));

        let weights_path = resolved.dir.join("model.safetensors");
        // SAFETY: `model.safetensors` is validated on disk and read-only for the lifetime of VarBuilder.
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[weights_path.as_std_path()], DType::F32, &device)? };

        let is_nomic = probe.model_type.eq_ignore_ascii_case("nomic_bert") || probe.architectures.iter().any(|a| a.contains("NomicBert"));

        let backend = if is_nomic {
            let cfg: NomicBertConfig = serde_json::from_str(&config_str)?;
            let model = NomicBertModel::load(vb, &cfg)?;
            EncoderBackend::NomicBert(model)
        } else {
            let cfg: BertConfig = serde_json::from_str(&config_str)?;
            let model = BertModel::load(vb, &cfg)?;
            EncoderBackend::Bert(model)
        };

        debug!(device = ?device, is_nomic, max_tokens, "loaded local embedding model");

        Ok(Self {
            backend,
            tokenizer,
            device,
            max_tokens,
            query_prefix: resolved.query_prefix.clone(),
            doc_prefix: resolved.doc_prefix.clone(),
        })
    }

    pub fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    pub fn doc_prefix(&self) -> &str {
        &self.doc_prefix
    }

    /// Counts exact raw tokens for `text` (including `doc_prefix`) without truncation.
    pub fn count_raw_tokens(&self, text: &str) -> Result<usize, EmbedError> {
        let input = if self.doc_prefix.is_empty() {
            text.to_string()
        } else {
            format!("{}{text}", self.doc_prefix)
        };
        let mut raw_tok = self.tokenizer.clone();
        let _ = raw_tok.with_truncation(None);
        let encoding = raw_tok.encode(input, true).map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        Ok(encoding.get_ids().len())
    }

    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbedError> {
        let input = if self.query_prefix.is_empty() {
            query.to_string()
        } else {
            format!("{}{query}", self.query_prefix)
        };
        let mut batch = self.encode_batch(&[input])?;
        Ok(batch.pop().unwrap_or_default())
    }

    pub fn embed_documents_with_progress<F>(
        &self,
        docs: &[(String, String)],
        batch_size: usize,
        mut on_progress: F,
    ) -> Result<Vec<Vec<f32>>, EmbedError>
    where
        F: FnMut(usize, usize, &str),
    {
        if docs.is_empty() {
            return Ok(Vec::new());
        }

        // Sort indices by text length ascending so short manifests batch together with minimal padding
        let mut order: Vec<usize> = (0..docs.len()).collect();
        order.sort_by_key(|&idx| docs[idx].1.len());

        let mut results = vec![Vec::new(); docs.len()];
        let step = batch_size.max(1);
        let total = docs.len();
        let mut completed = 0usize;

        for idx_chunk in order.chunks(step) {
            let current_label = idx_chunk.last().map(|&idx| docs[idx].0.as_str()).unwrap_or_default();
            on_progress(completed, total, current_label);

            let prefixed: Vec<String> = idx_chunk
                .iter()
                .map(|&idx| {
                    let body = truncate_for_tokenizer(&docs[idx].1, self.max_tokens);
                    if self.doc_prefix.is_empty() {
                        body.to_string()
                    } else {
                        format!("{}{body}", self.doc_prefix)
                    }
                })
                .collect();

            let batch_vecs = self.encode_batch(&prefixed)?;
            for (&orig_idx, vec) in idx_chunk.iter().zip(batch_vecs) {
                results[orig_idx] = vec;
            }

            completed += idx_chunk.len();
        }

        on_progress(total, total, "");
        Ok(results)
    }

    fn encode_batch(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let sliced: Vec<String> = inputs
            .iter()
            .map(|s| truncate_for_tokenizer(s, self.max_tokens).to_string())
            .collect();

        let encodings = self
            .tokenizer
            .encode_batch(sliced, true)
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;

        let batch_size = encodings.len();
        let seq_len = encodings.first().map_or(0, |e| e.get_ids().len());

        let mut flat_ids = Vec::with_capacity(batch_size * seq_len);
        let mut flat_mask = Vec::with_capacity(batch_size * seq_len);
        let mut flat_type_ids = Vec::with_capacity(batch_size * seq_len);

        for enc in &encodings {
            flat_ids.extend_from_slice(enc.get_ids());
            flat_mask.extend_from_slice(enc.get_attention_mask());
            flat_type_ids.extend_from_slice(enc.get_type_ids());
        }

        let input_ids = Tensor::from_vec(flat_ids, (batch_size, seq_len), &self.device)?;
        let attention_mask = Tensor::from_vec(flat_mask, (batch_size, seq_len), &self.device)?;
        let token_type_ids = Tensor::from_vec(flat_type_ids, (batch_size, seq_len), &self.device)?;

        let token_embeddings = match &self.backend {
            EncoderBackend::Bert(model) => model.forward(&input_ids, &token_type_ids, Some(&attention_mask))?,
            EncoderBackend::NomicBert(model) => model.forward(&input_ids, Some(&token_type_ids), Some(&attention_mask))?,
        };

        let mask_f32 = attention_mask.to_dtype(DType::F32)?.unsqueeze(2)?;
        let masked = token_embeddings.broadcast_mul(&mask_f32)?;
        let summed = masked.sum(1)?;
        let counts = mask_f32.sum(1)?.clamp(1e-9, f64::MAX)?;
        let pooled = summed.broadcast_div(&counts)?;

        let norms = pooled.sqr()?.sum_keepdim(1)?.sqrt()?.clamp(1e-12, f64::MAX)?;
        let normalized = pooled.broadcast_div(&norms)?;

        Ok(normalized.to_vec2::<f32>()?)
    }

    pub fn device_label(&self) -> &'static str {
        match self.device {
            Device::Cpu => "cpu",
            Device::Cuda(_) => "cuda",
            Device::Metal(_) => "metal",
        }
    }
}

fn select_device() -> Device {
    #[cfg(target_os = "macos")]
    if let Ok(metal) = Device::new_metal(0) {
        return metal;
    }

    Device::Cpu
}
