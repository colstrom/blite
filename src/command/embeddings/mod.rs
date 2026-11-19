// src/command/embeddings/mod.rs

mod exec;

use bon::Builder;
use clap::Args;
use clap::Subcommand;
pub use exec::EmbeddingsPayload;
use miette::Diagnostic;
use thiserror::Error;

use crate::command::args::IndexFilterArgs;
use crate::command::args::RepoTargetArgs;
use crate::config::ConfigError;
use crate::embed::EmbedError;
use crate::index::IndexError;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum EmbeddingsError {
    #[diagnostic(transparent)]
    Config(#[from] ConfigError),

    #[diagnostic(transparent)]
    Embed(#[from] EmbedError),

    #[diagnostic(transparent)]
    Index(#[from] IndexError),

    #[diagnostic(code(blite::embeddings::io))]
    Io(#[from] std::io::Error),

    #[diagnostic(code(blite::embeddings::json))]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct EmbeddingsCommand {
    #[command(subcommand)]
    pub(super) action: EmbeddingsAction,
}

#[derive(Debug, Clone, Subcommand)]
pub enum EmbeddingsAction {
    /// Generate or rebuild cached embedding vectors for a repository
    #[command(visible_aliases = ["generate", "warm", "rebuild"])]
    Build(EmbeddingsBuildArgs),
    /// Remove cached embeddings for files/chunks no longer present in a target repository
    Prune(EmbeddingsPruneArgs),
    /// Delete cached embedding files for a specific model or across all models
    #[command(visible_aliases = ["clean", "purge"])]
    Clear(EmbeddingsClearArgs),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct EmbeddingsBuildArgs {
    #[command(flatten)]
    #[builder(default)]
    pub target: RepoTargetArgs,

    /// Model ID or local model directory path (defaults to the active model in config)
    #[arg(short, long)]
    pub model: Option<String>,

    /// Recompute all embeddings from scratch even if already cached
    #[arg(long)]
    #[builder(default = false)]
    pub force: bool,

    /// Allow reading models from the local HuggingFace Hub cache
    #[arg(long, env = "BLITE_USE_HF_CACHE")]
    #[builder(default = false)]
    pub use_hf_cache: bool,

    #[command(flatten)]
    #[builder(default)]
    pub filter: IndexFilterArgs,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct EmbeddingsPruneArgs {
    #[command(flatten)]
    #[builder(default)]
    pub target: RepoTargetArgs,

    /// Model ID or local model directory path (defaults to the active model in config)
    #[arg(short, long, conflicts_with = "all")]
    pub model: Option<String>,

    /// Prune stale entries across all cached model files
    #[arg(long, conflicts_with = "model")]
    #[builder(default = false)]
    pub all: bool,

    /// Allow reading models from the local HuggingFace Hub cache
    #[arg(long, env = "BLITE_USE_HF_CACHE")]
    #[builder(default = false)]
    pub use_hf_cache: bool,

    #[command(flatten)]
    #[builder(default)]
    pub filter: IndexFilterArgs,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct EmbeddingsClearArgs {
    /// Model ID or local model directory path to clear cache for
    #[arg(short, long, required_unless_present = "all", conflicts_with = "all")]
    pub model: Option<String>,

    /// Clear cached embeddings for all models
    #[arg(long, required_unless_present = "model", conflicts_with = "model")]
    #[builder(default = false)]
    pub all: bool,
}

impl EmbeddingsCommand {
    pub fn execute(&self) -> Result<EmbeddingsPayload, EmbeddingsError> {
        match &self.action {
            EmbeddingsAction::Build(args) => exec::execute_build(args),
            EmbeddingsAction::Prune(args) => exec::execute_prune(args),
            EmbeddingsAction::Clear(args) => exec::execute_clear(args),
        }
    }

    pub fn run(self) -> Result<serde_json::Value, EmbeddingsError> {
        let payload = self.execute()?;
        Ok(serde_json::to_value(payload)?)
    }
}
