// src/command/models/mod.rs

mod exec;

use bon::Builder;
use clap::Args;
use clap::Subcommand;
use clap::ValueEnum;
pub use exec::ModelEntryPayload;
pub use exec::ModelsPayload;
use miette::Diagnostic;
use thiserror::Error;

use crate::command::ExecuteCommand;
use crate::command::json_command;
use crate::config::ConfigError;
use crate::embed::EmbedError;
use crate::render::render_models_table;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum ModelsError {
    #[diagnostic(transparent)]
    Config(#[from] ConfigError),

    #[diagnostic(transparent)]
    Embed(#[from] EmbedError),

    #[error("filesystem error while managing local models")]
    #[diagnostic(code(blite::models::io))]
    Io(#[from] std::io::Error),

    #[error("failed to serialize models payload: {0}")]
    #[diagnostic(code(blite::models::json))]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ModelsCommand {
    #[command(subcommand)]
    pub(crate) action: Option<ModelsAction>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ModelsAction {
    /// List available curated embedding models with origin, license, speed, and compliance details
    List,
    /// Inspect detailed metadata and local installation status for a specific model
    Info(ModelInfoArgs),
    /// Download a curated embedding model into blite's local models directory and activate it
    Download(ModelDownloadArgs),
    /// Activate a curated model ID or a custom local model directory path
    Use(ModelUseArgs),
    /// Deactivate the currently selected embedding model and revert default search to lexical-only
    #[command(visible_aliases = ["off", "clear", "none"])]
    Disable,
    /// Remove a downloaded model from blite's local models directory
    #[command(visible_aliases = ["rm", "delete", "uninstall"])]
    Remove(ModelRemoveArgs),
    /// Explicitly allow or disallow reading from the local HuggingFace Hub cache
    HfCache(HfCacheArgs),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ModelRemoveArgs {
    /// Curated model ID to remove from blite's local models directory
    pub(crate) model: String,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ModelInfoArgs {
    /// Curated model ID (e.g. `granite-30m`, `mxbai-xsmall`, `nomic-v1.5`)
    pub(crate) model: String,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ModelDownloadArgs {
    /// Curated model ID to download (see `blite models list`)
    pub(crate) model: String,

    /// Download the model without setting it as the active model in config
    #[arg(long)]
    #[builder(default = false)]
    pub(crate) no_activate: bool,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ModelUseArgs {
    /// Curated model ID (e.g. `granite-30m`) or path to a local model directory
    #[arg(long_help = "Curated model ID (see `blite models list`) OR a path to a local model directory.\n\n\
                     A local model directory must use the same layout produced by `blite models download`:\n\
                     - config.json (HuggingFace BertModel or NomicBertModel config)\n\
                     - tokenizer.json (HuggingFace fast tokenizer file)\n\
                     - model.safetensors (Safetensors weights)")]
    pub(crate) target: String,

    /// Allow resolving the model from the local HuggingFace Hub cache if not in blite's models directory
    #[arg(long)]
    #[builder(default = false)]
    pub(crate) allow_hf_cache: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HfCacheConsent {
    Enable,
    Disable,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct HfCacheArgs {
    /// Set to `enable` to allow probing `~/.cache/huggingface/hub`, or `disable` to forbid it
    #[arg(value_enum)]
    pub(crate) consent: HfCacheConsent,
}

impl ExecuteCommand for ModelsCommand {
    type Error = ModelsError;
    type Output = ModelsPayload;

    fn execute(self) -> Result<Self::Output, Self::Error> {
        exec::execute(&self)
    }

    fn render(output: Self::Output) -> Result<Option<String>, Self::Error> {
        Ok(Some(render_models_table(&output)))
    }
}

json_command!(ModelsCommand);
