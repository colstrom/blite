// src/command/models/exec.rs

use camino::Utf8Path;
use camino::Utf8PathBuf;
use serde::Deserialize;
use serde::Serialize;

use super::HfCacheArgs;
use super::HfCacheConsent;
use super::ModelDownloadArgs;
use super::ModelInfoArgs;
use super::ModelRemoveArgs;
use super::ModelUseArgs;
use super::ModelsAction;
use super::ModelsCommand;
use super::ModelsError;
use crate::config::AppConfig;
use crate::embed::EmbedError;
use crate::embed::MODEL_CATALOG;
use crate::embed::ValidatedModelDir;
use crate::embed::download_curated_model;
use crate::embed::embedding_cache_path;
use crate::embed::find_model_spec;
use crate::embed::locate_curated_model;
use crate::types::ModelId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntryPayload {
    pub id: ModelId,
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
    pub active_model: Option<ModelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_model_path: Option<Utf8PathBuf>,
    pub allow_hf_cache: bool,
    pub models: Vec<ModelEntryPayload>,
}

pub(super) fn execute(cmd: &ModelsCommand) -> Result<ModelsPayload, ModelsError> {
    let action = cmd.action.clone().unwrap_or(ModelsAction::List);

    match action {
        ModelsAction::List => build_models_payload(None),
        ModelsAction::Info(args) => execute_info(&args),
        ModelsAction::Download(args) => execute_download(&args),
        ModelsAction::Use(args) => execute_use(&args),
        ModelsAction::Disable => execute_disable(),
        ModelsAction::Remove(args) => execute_remove(&args),
        ModelsAction::HfCache(args) => execute_hf_cache(&args),
    }
}

fn execute_info(args: &ModelInfoArgs) -> Result<ModelsPayload, ModelsError> {
    let spec = find_model_spec(&args.model).ok_or_else(|| EmbedError::UnknownModel(args.model.clone()))?;
    build_models_payload(Some(spec.id))
}

fn execute_download(args: &ModelDownloadArgs) -> Result<ModelsPayload, ModelsError> {
    let spec = find_model_spec(&args.model).ok_or_else(|| EmbedError::UnknownModel(args.model.clone()))?;
    download_curated_model(spec)?;

    let mut config = AppConfig::load()?;
    if !args.no_activate {
        config.active_model = ModelId::try_from(spec.id).ok();
        config.active_model_path = None;
        let cfg_path = config.save()?;
        eprintln!("Activated '{}' in {}", spec.id, cfg_path);
    }

    build_models_payload(Some(spec.id))
}

fn execute_use(args: &ModelUseArgs) -> Result<ModelsPayload, ModelsError> {
    let mut config = AppConfig::load()?;
    let candidate_path = Utf8Path::new(&args.target);

    if candidate_path.is_dir() {
        let validated = ValidatedModelDir::try_from(candidate_path)?;
        let canonical = validated.canonicalize_utf8().unwrap_or_else(|_| validated.into_inner());
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

    if locate_curated_model(spec, config.allow_hf_cache).is_none() {
        eprintln!(
            "\x1b[1;33mNote:\x1b[0m '{}' is not downloaded locally yet. Run `blite models download {}` to fetch it.",
            spec.id, spec.id
        );
    }

    config.active_model = ModelId::try_from(spec.id).ok();
    config.active_model_path = None;
    let cfg_path = config.save()?;
    eprintln!("Activated '{}' in {}", spec.id, cfg_path);

    build_models_payload(Some(spec.id))
}

fn execute_disable() -> Result<ModelsPayload, ModelsError> {
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

fn execute_remove(args: &ModelRemoveArgs) -> Result<ModelsPayload, ModelsError> {
    let spec = find_model_spec(&args.model).ok_or_else(|| EmbedError::UnknownModel(args.model.clone()))?;

    let model_dir = AppConfig::models_dir()?.join(spec.id);
    if model_dir.exists() {
        fs_err::remove_dir_all(&model_dir)?;
        eprintln!("Removed model directory {}", model_dir);
    } else {
        eprintln!("Model '{}' is not installed in {}", spec.id, model_dir);
    }

    if let Some(cache_file) = embedding_cache_path(spec.id)
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

fn execute_hf_cache(args: &HfCacheArgs) -> Result<ModelsPayload, ModelsError> {
    let mut config = AppConfig::load()?;
    config.allow_hf_cache = matches!(args.consent, HfCacheConsent::Enable);
    let cfg_path = config.save()?;
    eprintln!(
        "HuggingFace Hub local cache access {} in {}",
        if config.allow_hf_cache { "enabled" } else { "disabled" },
        cfg_path
    );
    build_models_payload(None)
}

fn build_models_payload(filter_id: Option<&str>) -> Result<ModelsPayload, ModelsError> {
    let config = AppConfig::load()?;
    let models_dir = AppConfig::models_dir()?;

    let models = MODEL_CATALOG
        .iter()
        .filter(|spec| filter_id.is_none_or(|id| spec.id == id))
        .filter_map(|spec| {
            let id = ModelId::try_from(spec.id).ok()?;
            let local_path = locate_curated_model(spec, config.allow_hf_cache).map(|src| src.into_path());
            let installed = local_path.is_some();

            let active = config.active_model_path.is_none()
                && config
                    .active_model
                    .as_deref()
                    .is_some_and(|active_id| active_id.eq_ignore_ascii_case(spec.id));

            Some(ModelEntryPayload {
                id,
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
            })
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
