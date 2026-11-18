// src/command/args.rs

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;

use crate::index::IndexBuildOptions;
use crate::index::SemanticMode;

#[derive(Debug, Clone, Args, Builder)]
pub struct RepoTargetArgs {
    /// Path to the target repository root
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,
}

impl Default for RepoTargetArgs {
    fn default() -> Self {
        Self {
            repo: Utf8PathBuf::from("."),
        }
    }
}

#[derive(Debug, Clone, Default, Args, Builder)]
pub struct IndexFilterArgs {
    /// Include `vendor/` directories when indexing repository files
    #[arg(long)]
    #[builder(default = false)]
    pub include_vendor: bool,

    /// Include untracked files when indexing a Git repository
    #[arg(long)]
    #[builder(default = false)]
    pub untracked: bool,
}

#[derive(Debug, Clone, Default, Args, Builder)]
pub struct SemanticArgs {
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

    /// Disable reading and writing the on-disk embedding vector cache
    #[arg(long)]
    #[builder(default = false)]
    pub no_cache: bool,
}

impl SemanticArgs {
    pub fn semantic_mode(&self) -> SemanticMode {
        if self.semantic {
            SemanticMode::Require
        } else if self.lexical {
            SemanticMode::Off
        } else {
            SemanticMode::Auto
        }
    }

    pub fn to_build_options(&self, filter: &IndexFilterArgs, show_progress: bool) -> IndexBuildOptions {
        IndexBuildOptions::builder()
            .semantic_mode(self.semantic_mode())
            .maybe_model_dir(self.model_dir.clone())
            .allow_hf_cache(self.use_hf_cache)
            .include_vendor(filter.include_vendor)
            .include_untracked(filter.untracked)
            .no_cache(self.no_cache)
            .show_progress(show_progress)
            .build()
    }
}

#[derive(Debug, Clone, Default, Args, Builder)]
pub struct ReplHistoryArgs {
    /// Explicit path to the REPL history file
    #[arg(long, conflicts_with_all = ["repo_history", "no_history"])]
    pub history_file: Option<Utf8PathBuf>,

    /// Scope REPL history per repository instead of sharing globally
    #[arg(long, conflicts_with_all = ["history_file", "no_history"])]
    #[builder(default = false)]
    pub repo_history: bool,

    /// Disable reading and writing REPL history
    #[arg(long, conflicts_with_all = ["history_file", "repo_history"])]
    #[builder(default = false)]
    pub no_history: bool,
}
