// src/command/search.rs

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::drift::KitchenComparator;
use crate::git::FileCommitHistory;
use crate::index::IndexBuildOptions;
use crate::index::IndexError;
use crate::index::RepoIndex;
use crate::index::SearchHit;
use crate::index::SemanticMode;
use crate::types::SearchQuery;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum SearchError {
    #[diagnostic(transparent)]
    Index(#[from] IndexError),

    #[diagnostic(code(blite::search::json))]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct SearchCommand {
    /// Query string to search across repository manifests and code
    pub query: SearchQuery,

    /// Path to the target repository root
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

    /// Optional path to the kitchen repository for drift comparison
    #[arg(skip)]
    pub kitchen: Option<Utf8PathBuf>,

    /// Maximum number of matching documents to return
    #[arg(short = 'n', long, default_value_t = 10)]
    #[builder(default = 10)]
    pub limit: usize,

    /// Require hybrid semantic search (fails with setup instructions if no model is installed)
    #[arg(long, conflicts_with = "lexical")]
    #[builder(default = false)]
    pub semantic: bool,

    /// Force lexical BM25 search only, even when an embedding model is configured
    #[arg(long, conflicts_with = "semantic")]
    #[builder(default = false)]
    pub lexical: bool,

    /// Path to a local embedding model directory (containing config.json, tokenizer.json, model.safetensors)
    #[arg(
        long,
        env = "BLITE_MODEL_DIR",
        long_help = "Path to a local embedding model directory.\n\n\
                     Must follow the same layout produced by `blite models download`:\n\
                     - config.json (HuggingFace BertModel or NomicBertModel config)\n\
                     - tokenizer.json (HuggingFace fast tokenizer file)\n\
                     - model.safetensors (Safetensors weights)"
    )]
    pub model_dir: Option<Utf8PathBuf>,

    /// Allow reading models from the local HuggingFace Hub cache (`~/.cache/huggingface/hub`)
    #[arg(long, env = "BLITE_USE_HF_CACHE")]
    #[builder(default = false)]
    pub use_hf_cache: bool,

    /// Include `vendor/` directories when indexing repository files
    #[arg(long)]
    #[builder(default = false)]
    pub include_vendor: bool,

    /// Include untracked files when indexing a Git repository
    #[arg(long)]
    #[builder(default = false)]
    pub untracked: bool,

    /// Disable reading and writing the on-disk embedding vector cache
    #[arg(long)]
    #[builder(default = false)]
    pub no_cache: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchPayload {
    pub query: SearchQuery,
    pub total_hits: usize,
    pub hits: Vec<SearchHit>,
}

impl SearchCommand {
    pub fn execute(&self) -> Result<SearchPayload, SearchError> {
        let semantic_mode = if self.semantic {
            SemanticMode::Require
        } else if self.lexical {
            SemanticMode::Off
        } else {
            SemanticMode::Auto
        };

        let opts = IndexBuildOptions::builder()
            .semantic_mode(semantic_mode)
            .maybe_model_dir(self.model_dir.clone())
            .allow_hf_cache(self.use_hf_cache)
            .include_vendor(self.include_vendor)
            .include_untracked(self.untracked)
            .no_cache(self.no_cache)
            .build();

        let index = RepoIndex::build_with_options(&self.repo, &opts)?;
        let mut hits = index.search(&self.query, self.limit)?;

        if let Some(kitchen_root) = &self.kitchen {
            let comparator = KitchenComparator::new(&self.repo, kitchen_root);
            for hit in &mut hits {
                hit.drift = Some(comparator.compare(&hit.path));
            }
        }

        let file_history = FileCommitHistory::build_from_dir(&self.repo);
        for hit in &mut hits {
            let commits = file_history.commits_for(&hit.path);
            if !commits.is_empty() {
                hit.recent_commits = commits.to_vec();
            }
        }

        Ok(SearchPayload {
            query: self.query.clone(),
            total_hits: hits.len(),
            hits,
        })
    }

    pub fn run(self) -> Result<serde_json::Value, SearchError> {
        let payload = self.execute()?;
        Ok(serde_json::to_value(payload)?)
    }
}
