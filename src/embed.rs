use std::collections::HashMap;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use candle_core::DType;
use candle_core::Device;
use candle_core::Tensor;
use candle_nn::VarBuilder;
use candle_transformers::models::bert::BertModel;
use candle_transformers::models::bert::Config as BertConfig;
use candle_transformers::models::nomic_bert::Config as NomicBertConfig;
use candle_transformers::models::nomic_bert::NomicBertModel;
use instant_distance::Builder as HnswBuilder;
use instant_distance::HnswMap;
use instant_distance::Point;
use instant_distance::Search;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use tokenizers::PaddingParams;
use tokenizers::Tokenizer;
use tokenizers::TruncationParams;
use tracing::debug;
use tracing::instrument;

use crate::config::AppConfig;
use crate::config::ConfigError;

pub const REQUIRED_MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

#[derive(Debug, Error, Diagnostic)]
pub enum EmbedError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] ConfigError),

    #[error("failed to read model file")]
    #[diagnostic(code(blite::embed::io))]
    Io(#[from] std::io::Error),

    #[error("unknown model '{0}'; run `blite models list` to inspect available models")]
    #[diagnostic(code(blite::embed::unknown_model))]
    UnknownModel(String),

    #[error("model directory '{path}' is missing required file '{missing}'")]
    #[diagnostic(
        code(blite::embed::incomplete_model_dir),
        help("A valid model directory must contain config.json, tokenizer.json, and model.safetensors.")
    )]
    IncompleteModelDir { path: Utf8PathBuf, missing: String },

    #[error("semantic search was explicitly requested, but no embedding model is configured or installed")]
    #[diagnostic(
        code(blite::embed::model_unavailable),
        help(
            "Run `blite models list` and `blite models download <model-id>` (or pass `--model-dir <path>`), or omit `--semantic` to fall back to lexical search."
        )
    )]
    ModelUnavailable,

    #[error("failed to parse model configuration JSON: {0}")]
    #[diagnostic(code(blite::embed::model_config))]
    ModelConfig(#[from] serde_json::Error),

    #[error("tokenizer error: {0}")]
    #[diagnostic(code(blite::embed::tokenizer))]
    Tokenizer(String),

    #[error("candle tensor/model execution error: {0}")]
    #[diagnostic(code(blite::embed::candle))]
    Candle(#[from] candle_core::Error),

    #[error("huggingface hub download error: {0}")]
    #[diagnostic(code(blite::embed::hf_hub))]
    HfHub(String),
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub hf_repo: &'static str,
    pub origin: &'static str,
    pub license: &'static str,
    pub params: &'static str,
    pub layers: usize,
    pub dimensions: usize,
    pub max_context: usize,
    pub approx_size: &'static str,
    pub speed_tier: &'static str,
    pub compliance_notes: &'static str,
    pub best_for: &'static str,
    pub query_prefix: &'static str,
    pub doc_prefix: &'static str,
}

pub const MODEL_CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "mxbai-xsmall",
        hf_repo: "mixedbread-ai/mxbai-embed-xsmall-v1",
        origin: "DE / EU (Mixedbread)",
        license: "Apache-2.0",
        params: "24M",
        layers: 6,
        dimensions: 384,
        max_context: 512,
        approx_size: "~95 MB",
        speed_tier: "Very Fast (CPU & Metal)",
        compliance_notes: "EU-developed (GDPR jurisdiction), Apache-2.0, open weights.",
        best_for: "Ultra-fast indexing & queries on Linux CPU with EU origin.",
        query_prefix: "Represent this sentence for searching relevant passages: ",
        doc_prefix: "",
    },
    ModelSpec {
        id: "mxbai-large",
        hf_repo: "mixedbread-ai/mxbai-embed-large-v1",
        origin: "DE / EU (Mixedbread)",
        license: "Apache-2.0",
        params: "335M",
        layers: 24,
        dimensions: 1024,
        max_context: 512,
        approx_size: "~1.3 GB",
        speed_tier: "Moderate (Best on Metal)",
        compliance_notes: "EU-developed (GDPR jurisdiction), Apache-2.0, open weights.",
        best_for: "Heavyweight semantic accuracy on Apple Silicon Metal.",
        query_prefix: "Represent this sentence for searching relevant passages: ",
        doc_prefix: "",
    },
    ModelSpec {
        id: "granite-30m",
        hf_repo: "ibm-granite/granite-embedding-30m-english",
        origin: "US (IBM Research)",
        license: "Apache-2.0",
        params: "30M",
        layers: 6,
        dimensions: 384,
        max_context: 512,
        approx_size: "~120 MB",
        speed_tier: "Very Fast (CPU & Metal)",
        compliance_notes: "Enterprise-audited training corpus for IP/licensing provenance; Apache-2.0.",
        best_for: "Strict enterprise compliance environments & fast 6-layer CPU runs.",
        query_prefix: "",
        doc_prefix: "",
    },
    ModelSpec {
        id: "arctic-xs",
        hf_repo: "Snowflake/snowflake-arctic-embed-xs",
        origin: "US (Snowflake)",
        license: "Apache-2.0",
        params: "22M",
        layers: 6,
        dimensions: 384,
        max_context: 512,
        approx_size: "~90 MB",
        speed_tier: "Very Fast (CPU & Metal)",
        compliance_notes: "US-developed, Apache-2.0, open weights.",
        best_for: "Independent US-origin option with minimal disk and CPU footprint.",
        query_prefix: "Represent this sentence for searching relevant passages: ",
        doc_prefix: "",
    },
    ModelSpec {
        id: "nomic-v1.5",
        hf_repo: "nomic-ai/nomic-embed-text-v1.5",
        origin: "US (Nomic AI)",
        license: "Apache-2.0",
        params: "137M",
        layers: 12,
        dimensions: 768,
        max_context: 8192,
        approx_size: "~550 MB",
        speed_tier: "Fast (Metal) / Moderate (CPU)",
        compliance_notes: "Fully open weights, training code, and audited open training data; Apache-2.0.",
        best_for: "Long scripts/READMEs (8K context) & maximum open-source transparency.",
        query_prefix: "search_query: ",
        doc_prefix: "search_document: ",
    },
    ModelSpec {
        id: "bge-small",
        hf_repo: "BAAI/bge-small-en-v1.5",
        origin: "CN (BAAI)",
        license: "MIT",
        params: "33M",
        layers: 12,
        dimensions: 384,
        max_context: 512,
        approx_size: "~130 MB",
        speed_tier: "Fast (CPU & Metal)",
        compliance_notes: "Open weights (MIT); training data not itemized; CN-origin.",
        best_for: "Proven, compact baseline for short manifests and code snippets.",
        query_prefix: "Represent this sentence for searching relevant passages: ",
        doc_prefix: "",
    },
];

pub fn find_model_spec(id_or_repo: &str) -> Option<&'static ModelSpec> {
    let needle = id_or_repo.trim().to_ascii_lowercase();
    MODEL_CATALOG
        .iter()
        .find(|spec| spec.id.eq_ignore_ascii_case(&needle) || spec.hf_repo.eq_ignore_ascii_case(&needle))
}

pub fn validate_model_dir(dir: &Utf8Path) -> Result<(), EmbedError> {
    for file in REQUIRED_MODEL_FILES {
        if !dir.join(file).is_file() {
            return Err(EmbedError::IncompleteModelDir {
                path: dir.to_path_buf(),
                missing: (*file).to_string(),
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub label: String,
    pub dir: Utf8PathBuf,
    pub query_prefix: String,
    pub doc_prefix: String,
}

#[derive(Debug, Clone, Default)]
pub struct ModelResolveOptions {
    pub override_model_dir: Option<Utf8PathBuf>,
    pub allow_hf_cache_override: bool,
}

pub fn resolve_active_model(config: &AppConfig, opts: &ModelResolveOptions) -> Result<Option<ResolvedModel>, EmbedError> {
    if let Some(dir) = &opts.override_model_dir {
        validate_model_dir(dir)?;
        let (query_prefix, doc_prefix) = infer_prefixes_from_dir(dir);
        return Ok(Some(ResolvedModel {
            label: dir.to_string(),
            dir: dir.clone(),
            query_prefix,
            doc_prefix,
        }));
    }

    if let Some(dir) = &config.active_model_path {
        if dir.is_dir() {
            validate_model_dir(dir)?;
            let (query_prefix, doc_prefix) = infer_prefixes_from_dir(dir);
            return Ok(Some(ResolvedModel {
                label: dir.to_string(),
                dir: dir.clone(),
                query_prefix,
                doc_prefix,
            }));
        }
        return Ok(None);
    }

    let Some(model_id) = &config.active_model else {
        return Ok(None);
    };

    let spec = find_model_spec(model_id);
    let (query_prefix, doc_prefix) = spec
        .map(|s| (s.query_prefix.to_string(), s.doc_prefix.to_string()))
        .unwrap_or_default();

    if let Ok(models_root) = AppConfig::models_dir() {
        let local_dir = models_root.join(model_id);
        if local_dir.is_dir() && validate_model_dir(&local_dir).is_ok() {
            return Ok(Some(ResolvedModel {
                label: model_id.clone(),
                dir: local_dir,
                query_prefix,
                doc_prefix,
            }));
        }
    }

    let hf_consented = opts.allow_hf_cache_override || config.allow_hf_cache;
    if hf_consented
        && let Some(spec) = spec
        && let Some(hf_dir) = probe_hf_cache_snapshot(spec.hf_repo)
    {
        return Ok(Some(ResolvedModel {
            label: format!("{} (hf-cache)", spec.id),
            dir: hf_dir,
            query_prefix,
            doc_prefix,
        }));
    }

    Ok(None)
}

fn infer_prefixes_from_dir(dir: &Utf8Path) -> (String, String) {
    let dir_lower = dir.as_str().to_ascii_lowercase();
    if dir_lower.contains("nomic") {
        return ("search_query: ".to_string(), "search_document: ".to_string());
    }
    if dir_lower.contains("bge") || dir_lower.contains("mxbai") || dir_lower.contains("arctic") {
        return (
            "Represent this sentence for searching relevant passages: ".to_string(),
            String::new(),
        );
    }
    (String::new(), String::new())
}

pub fn probe_hf_cache_snapshot(hf_repo: &str) -> Option<Utf8PathBuf> {
    let hub_dir = std::env::var("HF_HOME")
        .ok()
        .map(|h| Utf8PathBuf::from(h).join("hub"))
        .or_else(|| {
            dirs::cache_dir()
                .and_then(|c| Utf8PathBuf::try_from(c).ok())
                .map(|c| c.join("huggingface").join("hub"))
        })?;

    let repo_folder = format!("models--{}", hf_repo.replace('/', "--"));
    let snapshots_dir = hub_dir.join(repo_folder).join("snapshots");
    let entries = fs_err::read_dir(&snapshots_dir).ok()?;

    for entry in entries.flatten() {
        let Ok(path) = Utf8PathBuf::try_from(entry.path()) else {
            continue;
        };
        if path.is_dir() && validate_model_dir(&path).is_ok() {
            return Some(path);
        }
    }

    None
}

fn truncate_for_tokenizer(text: &str, max_tokens: usize) -> &str {
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
        validate_model_dir(&resolved.dir)?;

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

#[derive(Debug, Clone)]
struct EmbeddingPoint(Vec<f32>);

impl Point for EmbeddingPoint {
    fn distance(&self, other: &Self) -> f32 {
        let dot: f32 = self.0.iter().zip(other.0.iter()).map(|(a, b)| a * b).sum();
        (1.0 - dot).max(0.0)
    }
}

#[cfg(target_os = "macos")]
pub const EMBED_BATCH_SIZE: usize = 64;
#[cfg(not(target_os = "macos"))]
pub const EMBED_BATCH_SIZE: usize = 32;

const CACHE_MAGIC: &[u8; 8] = b"BLITEEMB";
const CACHE_VERSION: u32 = 1;

fn hash_embed_text(prefix: &str, text: &str) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for byte in prefix.as_bytes().iter().chain(b"\0").chain(text.as_bytes()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn load_embedding_cache(path: &Utf8Path) -> HashMap<u64, Vec<f32>> {
    let Ok(bytes) = fs_err::read(path) else {
        return HashMap::new();
    };
    if bytes.len() < 16 || &bytes[0..8] != CACHE_MAGIC {
        return HashMap::new();
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap_or_default());
    if version != CACHE_VERSION {
        return HashMap::new();
    }
    let dims = u32::from_le_bytes(bytes[12..16].try_into().unwrap_or_default()) as usize;
    if dims == 0 {
        return HashMap::new();
    }

    let record_bytes = 8 + dims * 4;
    let payload = &bytes[16..];
    let complete_records = payload.len() / record_bytes;
    let mut map = HashMap::with_capacity(complete_records);

    for chunk in payload.chunks_exact(record_bytes) {
        let hash = u64::from_le_bytes(chunk[0..8].try_into().unwrap_or_default());
        let mut vec = Vec::with_capacity(dims);
        for f_bytes in chunk[8..].chunks_exact(4) {
            vec.push(f32::from_le_bytes(f_bytes.try_into().unwrap_or_default()));
        }
        map.insert(hash, vec);
    }

    map
}

fn save_embedding_cache(path: &Utf8Path, entries: &HashMap<u64, Vec<f32>>) {
    let Some(first_vec) = entries.values().next() else {
        return;
    };
    let dims = first_vec.len() as u32;
    if dims == 0 {
        return;
    }

    if let Some(parent) = path.parent()
        && let Err(err) = fs_err::create_dir_all(parent)
    {
        debug!(path = %parent, error = %err, "failed to create embedding cache directory");
        return;
    }

    let mut buf = Vec::with_capacity(16 + entries.len() * (8 + first_vec.len() * 4));
    buf.extend_from_slice(CACHE_MAGIC);
    buf.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    buf.extend_from_slice(&dims.to_le_bytes());

    for (&hash, vec) in entries {
        if vec.len() != first_vec.len() {
            continue;
        }
        buf.extend_from_slice(&hash.to_le_bytes());
        for &val in vec {
            buf.extend_from_slice(&val.to_le_bytes());
        }
    }

    if let Err(err) = fs_err::write(path, &buf) {
        debug!(path = %path, error = %err, "failed to write embedding cache");
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
        let mut cache = cache_path.as_deref().map(load_embedding_cache).unwrap_or_default();

        let mut vectors = vec![Vec::new(); docs.len()];
        let mut uncached_indices = Vec::new();
        let mut uncached_docs = Vec::new();
        let mut doc_hashes = Vec::with_capacity(docs.len());

        for (idx, (label, text)) in docs.iter().enumerate() {
            let truncated = truncate_for_tokenizer(text, embedder.max_tokens());
            let hash = hash_embed_text(&embedder.doc_prefix, truncated);
            doc_hashes.push(hash);

            if let Some(cached_vec) = cache.get(&hash) {
                vectors[idx] = cached_vec.clone();
            } else {
                uncached_indices.push(idx);
                uncached_docs.push((label.clone(), text.clone()));
            }
        }

        if !uncached_docs.is_empty() {
            let fresh_vecs = embedder.embed_documents_with_progress(&uncached_docs, EMBED_BATCH_SIZE, &mut on_progress)?;
            for (orig_idx, vec) in uncached_indices.into_iter().zip(fresh_vecs) {
                cache.insert(doc_hashes[orig_idx], vec.clone());
                vectors[orig_idx] = vec;
            }
            if let Some(path) = &cache_path {
                save_embedding_cache(path, &cache);
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

pub fn embedding_cache_dir() -> Option<Utf8PathBuf> {
    let base = dirs::cache_dir()?;
    Utf8PathBuf::try_from(base).ok().map(|dir| dir.join("blite").join("embeddings"))
}

pub fn embedding_cache_path(model_label: &str) -> Option<Utf8PathBuf> {
    let cache_dir = embedding_cache_dir()?;
    let safe_name: String = model_label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(cache_dir.join(format!("{safe_name}.bin")))
}

pub fn hash_doc_with_prefix(prefix: &str, text: &str, max_tokens: usize) -> u64 {
    hash_embed_text(prefix, truncate_for_tokenizer(text, max_tokens))
}

pub fn compute_doc_embed_hash(embedder: &Embedder, text: &str) -> u64 {
    hash_doc_with_prefix(&embedder.doc_prefix, text, embedder.max_tokens())
}

pub fn read_embedding_cache(path: &Utf8Path) -> HashMap<u64, Vec<f32>> {
    load_embedding_cache(path)
}

pub fn write_embedding_cache(path: &Utf8Path, entries: &HashMap<u64, Vec<f32>>) {
    save_embedding_cache(path, entries);
}
