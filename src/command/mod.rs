// src/command/mod.rs

pub mod diagnostics;
pub mod embeddings;
pub mod history;
pub mod models;
pub mod repl;
pub mod search;

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Parser;
use diagnostics::DiagnosticsCommand;
use diagnostics::DiagnosticsError;
use diagnostics::render_diagnostics;
use embeddings::EmbeddingsCommand;
use embeddings::EmbeddingsError;
use history::HistoryCommand;
use history::HistoryError;
use history::HistoryPayload;
use miette::Diagnostic;
use models::ModelsCommand;
use models::ModelsError;
use models::render_models_table;
use repl::ReplCommand;
use repl::ReplError;
use search::SearchCommand;
use search::SearchError;
use search::SearchPayload;
use thiserror::Error;

use crate::render::RenderError;
use crate::render::TerminalRenderer;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum CommandError {
    #[diagnostic(transparent)]
    Diagnostics(#[from] DiagnosticsError),

    #[diagnostic(transparent)]
    Search(#[from] SearchError),

    #[diagnostic(transparent)]
    History(#[from] HistoryError),

    #[diagnostic(transparent)]
    Models(#[from] ModelsError),

    #[diagnostic(transparent)]
    Embeddings(#[from] EmbeddingsError),

    #[diagnostic(transparent)]
    Repl(#[from] ReplError),

    #[diagnostic(transparent)]
    Render(#[from] RenderError),

    #[diagnostic(code(blite::command::json))]
    Json(#[from] serde_json::Error),
}

/// Boring Local Information Telling Engine.
///
/// Tells you things you might want to know using information you already have.
#[derive(Debug, Clone, Parser, Builder)]
#[command(about)]
#[command(version)]
pub struct Command {
    /// Emit machine-readable JSON instead of syntax-highlighted terminal output
    #[arg(long, global = true)]
    #[builder(default = false)]
    pub json: bool,

    /// Default repository root when launching interactive mode directly
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

    /// Optional path to the kitchen repository for drift comparison
    #[arg(short, long, global = true)]
    pub kitchen: Option<Utf8PathBuf>,

    /// Default result limit when launching interactive mode directly
    #[arg(short = 'n', long, default_value_t = 5)]
    #[builder(default = 5)]
    pub limit: usize,

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

    #[command(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Subcommand {
    /// Search manifests, scripts, and docs using lexical BM25 and optional semantic ranking
    Search(SearchCommand),
    /// Search Git commit history, authors, and touched file paths
    History(HistoryCommand),
    /// Manage local embedding models and HuggingFace cache consent
    Models(ModelsCommand),
    /// Build, prune, or clear cached embedding vectors
    Embeddings(EmbeddingsCommand),
    /// Inspect repository chunk sizes, token counts, truncation, and batch padding
    Diagnostics(DiagnosticsCommand),
    /// Launch an interactive search session with a hot in-memory index
    Repl(ReplCommand),
}

impl Command {
    pub fn run(self) -> Result<(), crate::Error> {
        match self.subcommand {
            Some(Subcommand::Search(mut cmd)) => {
                cmd.kitchen = cmd.kitchen.or(self.kitchen);
                cmd.semantic = cmd.semantic || self.semantic;
                cmd.lexical = cmd.lexical || self.lexical;
                cmd.model_dir = cmd.model_dir.or(self.model_dir);
                cmd.use_hf_cache = cmd.use_hf_cache || self.use_hf_cache;
                cmd.include_vendor = cmd.include_vendor || self.include_vendor;
                cmd.untracked = cmd.untracked || self.untracked;
                cmd.no_cache = cmd.no_cache || self.no_cache;

                if self.json {
                    let value = cmd.run().map_err(CommandError::from)?;
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    let payload: SearchPayload = cmd.execute().map_err(CommandError::from)?;
                    let rendered = TerminalRenderer::new()
                        .render_hits(payload.query.as_ref(), &payload.hits)
                        .map_err(CommandError::from)?;
                    print!("{rendered}");
                }
            }
            Some(Subcommand::History(cmd)) => {
                if self.json {
                    let value = cmd.run().map_err(CommandError::from)?;
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    let payload: HistoryPayload = cmd.execute().map_err(CommandError::from)?;
                    let rendered = TerminalRenderer::new().render_commit_hits(payload.query.as_ref(), &payload.hits);
                    print!("{rendered}");
                }
            }
            Some(Subcommand::Models(cmd)) => {
                if self.json {
                    let value = cmd.run().map_err(CommandError::from)?;
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    let payload = cmd.execute().map_err(CommandError::from)?;
                    print!("{}", render_models_table(&payload));
                }
            }
            Some(Subcommand::Embeddings(cmd)) => {
                let value = cmd.run().map_err(CommandError::from)?;
                if self.json {
                    println!("{}", serde_json::to_string_pretty(&value)?);
                }
            }
            Some(Subcommand::Diagnostics(mut cmd)) => {
                cmd.model_dir = cmd.model_dir.or(self.model_dir);
                cmd.include_vendor = cmd.include_vendor || self.include_vendor;
                cmd.untracked = cmd.untracked || self.untracked;
                cmd.use_hf_cache = cmd.use_hf_cache || self.use_hf_cache;
                if self.json {
                    let value = cmd.run().map_err(CommandError::from)?;
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    let payload = cmd.execute().map_err(CommandError::from)?;
                    print!("{}", render_diagnostics(&payload));
                }
            }
            Some(Subcommand::Repl(mut cmd)) => {
                cmd.kitchen = cmd.kitchen.or(self.kitchen);
                cmd.semantic = cmd.semantic || self.semantic;
                cmd.lexical = cmd.lexical || self.lexical;
                cmd.model_dir = cmd.model_dir.or(self.model_dir);
                cmd.use_hf_cache = cmd.use_hf_cache || self.use_hf_cache;
                cmd.include_vendor = cmd.include_vendor || self.include_vendor;
                cmd.untracked = cmd.untracked || self.untracked;
                cmd.no_cache = cmd.no_cache || self.no_cache;
                cmd.run().map_err(CommandError::from)?;
            }
            None => {
                ReplCommand::builder()
                    .repo(self.repo)
                    .maybe_kitchen(self.kitchen)
                    .limit(self.limit)
                    .maybe_history_file(self.history_file)
                    .repo_history(self.repo_history)
                    .no_history(self.no_history)
                    .semantic(self.semantic)
                    .lexical(self.lexical)
                    .maybe_model_dir(self.model_dir)
                    .use_hf_cache(self.use_hf_cache)
                    .include_vendor(self.include_vendor)
                    .untracked(self.untracked)
                    .no_cache(self.no_cache)
                    .build()
                    .run()
                    .map_err(CommandError::from)?;
            }
        }

        Ok(())
    }
}
