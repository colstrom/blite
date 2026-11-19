// src/command/models.rs

use bon::Builder;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use clap::Args;
use clap::Subcommand;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::config::AppConfig;
use crate::config::ConfigError;
use crate::embed::EmbedError;
use crate::embed::MODEL_CATALOG;
use crate::embed::download_curated_model;
use crate::embed::find_model_spec;
use crate::embed::probe_hf_cache_snapshot;
use crate::embed::validate_model_dir;

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
    action: Option<ModelsAction>,
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
    pub model: String,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ModelInfoArgs {
    /// Curated model ID (e.g. `granite-30m`, `mxbai-xsmall`, `nomic-v1.5`)
    pub model: String,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ModelDownloadArgs {
    /// Curated model ID to download (see `blite models list`)
    pub model: String,

    /// Download the model without setting it as the active model in config
    #[arg(long)]
    #[builder(default = false)]
    pub no_activate: bool,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ModelUseArgs {
    /// Curated model ID (e.g. `granite-30m`) or path to a local model directory
    #[arg(long_help = "Curated model ID (see `blite models list`) OR a path to a local model directory.\n\n\
                     A local model directory must use the same layout produced by `blite models download`:\n\
                     - config.json (HuggingFace BertModel or NomicBertModel config)\n\
                     - tokenizer.json (HuggingFace fast tokenizer file)\n\
                     - model.safetensors (Safetensors weights)")]
    pub target: String,

    /// Allow resolving the model from the local HuggingFace Hub cache if not in blite's models directory
    #[arg(long)]
    #[builder(default = false)]
    pub allow_hf_cache: bool,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct HfCacheArgs {
    /// Set to `enable` to allow probing `~/.cache/huggingface/hub`, or `disable` to forbid it
    #[arg(value_parser = ["enable", "disable"])]
    pub consent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntryPayload {
    pub id: String,
    pub hf_repo: String,
    pub origin: String,
    pub license: String,
    pub params: String,
    pub layers: usize,
    pub dimensions: usize,
    pub max_context: usize,
    pub approx_size: String,
    pub speed_tier: String,
    pub compliance_notes: String,
    pub best_for: String,
    pub installed: bool,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<Utf8PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsPayload {
    pub models_dir: Utf8PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model_path: Option<Utf8PathBuf>,
    pub allow_hf_cache: bool,
    pub models: Vec<ModelEntryPayload>,
}

impl ModelsCommand {
    pub fn execute(&self) -> Result<ModelsPayload, ModelsError> {
        let action = self.action.clone().unwrap_or(ModelsAction::List);

        match action {
            ModelsAction::List => build_models_payload(None),
            ModelsAction::Info(args) => {
                let spec = find_model_spec(&args.model).ok_or_else(|| EmbedError::UnknownModel(args.model.clone()))?;
                build_models_payload(Some(spec.id))
            }
            ModelsAction::Download(args) => {
                let spec = find_model_spec(&args.model).ok_or_else(|| EmbedError::UnknownModel(args.model.clone()))?;
                download_curated_model(spec)?;

                let mut config = AppConfig::load()?;
                if !args.no_activate {
                    config.active_model = Some(spec.id.to_string());
                    config.active_model_path = None;
                    let cfg_path = config.save()?;
                    eprintln!("Activated '{}' in {}", spec.id, cfg_path);
                }

                build_models_payload(Some(spec.id))
            }
            ModelsAction::Use(args) => {
                let mut config = AppConfig::load()?;
                let candidate_path = Utf8Path::new(&args.target);

                if candidate_path.is_dir() {
                    validate_model_dir(candidate_path)?;
                    let canonical = candidate_path.canonicalize_utf8().unwrap_or_else(|_| candidate_path.to_path_buf());
                    config.active_model_path = Some(canonical.clone());
                    config.active_model = None;
                    let cfg_path = config.save()?;
                    eprintln!("Activated custom model directory '{}' in {}", canonical, cfg_path);
                    return build_models_payload(None);
                }

                let spec = find_model_spec(&args.target).ok_or_else(|| EmbedError::UnknownModel(args.target.clone()))?;

                if args.allow_hf_cache {
                    config.allow_hf_cache = true;
                }

                let models_dir = AppConfig::models_dir()?;
                let app_model_dir = models_dir.join(spec.id);
                let in_app_dir = app_model_dir.is_dir() && validate_model_dir(&app_model_dir).is_ok();
                let in_hf_cache = config.allow_hf_cache && probe_hf_cache_snapshot(spec.hf_repo).is_some();

                if !in_app_dir && !in_hf_cache {
                    eprintln!(
                        "\x1b[1;33mNote:\x1b[0m '{}' is not downloaded locally yet. Run `blite models download {}` to fetch it.",
                        spec.id, spec.id
                    );
                }

                config.active_model = Some(spec.id.to_string());
                config.active_model_path = None;
                let cfg_path = config.save()?;
                eprintln!("Activated '{}' in {}", spec.id, cfg_path);

                build_models_payload(Some(spec.id))
            }
            ModelsAction::Disable => {
                let mut config = AppConfig::load()?;
                let had_active = config.active_model.is_some() || config.active_model_path.is_some();
                config.active_model = None;
                config.active_model_path = None;
                let cfg_path = config.save()?;

                if had_active {
                    eprintln!(
                        "Deactivated active embedding model in {} (default search is now lexical-only).",
                        cfg_path
                    );
                } else {
                    eprintln!(
                        "No active embedding model was configured in {} (default search is already lexical-only).",
                        cfg_path
                    );
                }

                build_models_payload(None)
            }
            ModelsAction::Remove(args) => {
                let spec = find_model_spec(&args.model).ok_or_else(|| EmbedError::UnknownModel(args.model.clone()))?;

                let model_dir = AppConfig::models_dir()?.join(spec.id);
                if model_dir.exists() {
                    fs_err::remove_dir_all(&model_dir)?;
                    eprintln!("Removed model directory {}", model_dir);
                } else {
                    eprintln!("Model '{}' is not installed in {}", spec.id, model_dir);
                }

                if let Some(cache_file) = crate::embed::embedding_cache_path(spec.id)
                    && cache_file.exists()
                {
                    fs_err::remove_file(&cache_file)?;
                    eprintln!("Removed embedding cache {}", cache_file);
                }

                let mut config = AppConfig::load()?;
                if config
                    .active_model
                    .as_deref()
                    .is_some_and(|active| active.eq_ignore_ascii_case(spec.id))
                {
                    config.active_model = None;
                    let cfg_path = config.save()?;
                    eprintln!("Deactivated '{}' in {}", spec.id, cfg_path);
                }

                build_models_payload(None)
            }
            ModelsAction::HfCache(args) => {
                let mut config = AppConfig::load()?;
                config.allow_hf_cache = args.consent == "enable";
                let cfg_path = config.save()?;
                eprintln!(
                    "HuggingFace Hub local cache access {} in {}",
                    if config.allow_hf_cache { "enabled" } else { "disabled" },
                    cfg_path
                );
                build_models_payload(None)
            }
        }
    }

    pub fn run(self) -> Result<serde_json::Value, ModelsError> {
        let payload = self.execute()?;
        Ok(serde_json::to_value(payload)?)
    }
}

fn build_models_payload(filter_id: Option<&str>) -> Result<ModelsPayload, ModelsError> {
    let config = AppConfig::load()?;
    let models_dir = AppConfig::models_dir()?;

    let models = MODEL_CATALOG
        .iter()
        .filter(|spec| filter_id.is_none_or(|id| spec.id == id))
        .map(|spec| {
            let app_dir = models_dir.join(spec.id);
            let app_installed = app_dir.is_dir() && validate_model_dir(&app_dir).is_ok();
            let hf_snapshot = if !app_installed && config.allow_hf_cache {
                probe_hf_cache_snapshot(spec.hf_repo)
            } else {
                None
            };

            let (installed, local_path) = if app_installed {
                (true, Some(app_dir))
            } else if let Some(snap) = hf_snapshot {
                (true, Some(snap))
            } else {
                (false, None)
            };

            let active = config.active_model_path.is_none()
                && config
                    .active_model
                    .as_deref()
                    .is_some_and(|active_id| active_id.eq_ignore_ascii_case(spec.id));

            ModelEntryPayload {
                id: spec.id.to_string(),
                hf_repo: spec.hf_repo.to_string(),
                origin: spec.origin.to_string(),
                license: spec.license.to_string(),
                params: spec.params.to_string(),
                layers: spec.layers,
                dimensions: spec.dimensions,
                max_context: spec.max_context,
                approx_size: spec.approx_size.to_string(),
                speed_tier: spec.speed_tier.to_string(),
                compliance_notes: spec.compliance_notes.to_string(),
                best_for: spec.best_for.to_string(),
                installed,
                active,
                local_path,
            }
        })
        .collect();

    Ok(ModelsPayload {
        models_dir,
        active_model: config.active_model,
        active_model_path: config.active_model_path,
        allow_hf_cache: config.allow_hf_cache,
        models,
    })
}
