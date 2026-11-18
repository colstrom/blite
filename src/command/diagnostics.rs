// src/command/diagnostics.rs

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use super::args::IndexFilterArgs;
use super::args::RepoTargetArgs;
use crate::config::AppConfig;
use crate::embed::EMBED_BATCH_SIZE;
use crate::embed::EmbedError;
use crate::embed::Embedder;
use crate::embed::ModelResolveOptions;
use crate::embed::resolve_active_model;
use crate::index::IndexError;
use crate::index::collect_documents_for_diagnostics;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum DiagnosticsError {
    #[diagnostic(transparent)]
    Index(#[from] IndexError),

    #[diagnostic(transparent)]
    Embed(#[from] EmbedError),

    #[diagnostic(code(blite::diagnostics::json))]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct DiagnosticsCommand {
    #[command(flatten)]
    #[builder(default)]
    pub target: RepoTargetArgs,

    /// Number of largest documents/chunks to display (sorted by token count descending)
    #[arg(short = 'n', long, default_value_t = 25)]
    #[builder(default = 25)]
    pub top: usize,

    /// Path to a local embedding model directory
    #[arg(long, env = "BLITE_MODEL_DIR")]
    pub model_dir: Option<Utf8PathBuf>,

    /// Allow reading models from the local HuggingFace Hub cache
    #[arg(long, env = "BLITE_USE_HF_CACHE")]
    #[builder(default = false)]
    pub use_hf_cache: bool,

    #[command(flatten)]
    #[builder(default)]
    pub filter: IndexFilterArgs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentDiagnostic {
    pub path: String,
    pub bytes: usize,
    pub tokens: usize,
    pub exact_tokens: bool,
    pub multi_doc: bool,
    pub truncated: bool,
    pub larger_than_batch_budget: bool,
    pub pad_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsPayload {
    pub repo: Utf8PathBuf,
    pub total_chunks: usize,
    pub total_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub max_context_tokens: usize,
    pub batch_size: usize,
    pub multi_doc_chunks: usize,
    pub truncated_count: usize,
    pub entries: Vec<DocumentDiagnostic>,
}

impl DiagnosticsCommand {
    pub fn execute(&self) -> Result<DiagnosticsPayload, DiagnosticsError> {
        let docs = collect_documents_for_diagnostics(&self.target.repo, self.filter.include_vendor, self.filter.untracked)?;
        let total_chunks = docs.len();
        let total_bytes: usize = docs.iter().map(|d| d.text.len()).sum();

        let cfg = AppConfig::load().unwrap_or_default();
        let resolved = resolve_active_model(
            &cfg,
            &ModelResolveOptions {
                override_model_dir: self.model_dir.clone(),
                allow_hf_cache_override: self.use_hf_cache,
            },
        )?;

        let embedder = match &resolved {
            Some(model) => Some(Embedder::load(model)?),
            None => None,
        };

        let max_context = embedder.as_ref().map_or(512, Embedder::max_tokens);
        let batch_token_budget = max_context * EMBED_BATCH_SIZE;

        let mut sorted: Vec<_> = docs.iter().collect();
        sorted.sort_by_key(|d| d.text.len());

        let mut measured = Vec::with_capacity(sorted.len());
        for chunk in sorted.chunks(EMBED_BATCH_SIZE) {
            let mut batch_items = Vec::with_capacity(chunk.len());
            let mut batch_max_effective = 0usize;

            for doc in chunk {
                let (tokens, exact_tokens) = match &embedder {
                    Some(emb) => (emb.count_raw_tokens(&doc.text)?, true),
                    None => (doc.text.len().div_ceil(4), false),
                };
                let effective = tokens.min(max_context);
                batch_max_effective = batch_max_effective.max(effective);
                batch_items.push((doc, tokens, exact_tokens, effective));
            }

            for (doc, tokens, exact_tokens, effective) in batch_items {
                measured.push(DocumentDiagnostic {
                    path: doc.label.clone(),
                    bytes: doc.text.len(),
                    tokens,
                    exact_tokens,
                    multi_doc: doc.is_multi_doc,
                    truncated: tokens > max_context,
                    larger_than_batch_budget: tokens > batch_token_budget,
                    pad_tokens: batch_max_effective.saturating_sub(effective),
                });
            }
        }

        let multi_doc_chunks = measured.iter().filter(|d| d.multi_doc).count();
        let truncated_count = measured.iter().filter(|d| d.truncated).count();
        measured.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| b.bytes.cmp(&a.bytes)));
        measured.truncate(self.top);

        Ok(DiagnosticsPayload {
            repo: self.target.repo.clone(),
            total_chunks,
            total_bytes,
            model: resolved.map(|r| r.label),
            max_context_tokens: max_context,
            batch_size: EMBED_BATCH_SIZE,
            multi_doc_chunks,
            truncated_count,
            entries: measured,
        })
    }

    pub fn run(self) -> Result<serde_json::Value, DiagnosticsError> {
        let payload = self.execute()?;
        Ok(serde_json::to_value(payload)?)
    }
}
