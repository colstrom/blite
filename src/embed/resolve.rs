// src/embed/resolve.rs

use camino::Utf8Path;
use camino::Utf8PathBuf;

use super::EmbedError;
use super::ModelSpec;
use super::catalog::find_model_spec;
use super::catalog::infer_prefixes_from_dir;
use crate::config::AppConfig;

pub const REQUIRED_MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

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
pub enum ModelSource {
    AppDir(Utf8PathBuf),
    HfCache(Utf8PathBuf),
}

impl ModelSource {
    pub fn into_path(self) -> Utf8PathBuf {
        match self {
            Self::AppDir(path) | Self::HfCache(path) => path,
        }
    }

    pub fn into_resolved(self, spec: &ModelSpec) -> ResolvedModel {
        let (label, dir) = match self {
            Self::AppDir(path) => (spec.id.to_string(), path),
            Self::HfCache(path) => (format!("{} (hf-cache)", spec.id), path),
        };
        ResolvedModel {
            label,
            dir,
            query_prefix: spec.query_prefix.to_string(),
            doc_prefix: spec.doc_prefix.to_string(),
        }
    }
}

pub fn locate_curated_model(spec: &ModelSpec, allow_hf_cache: bool) -> Option<ModelSource> {
    if let Ok(models_dir) = AppConfig::models_dir() {
        let app_dir = models_dir.join(spec.id);
        if app_dir.is_dir() && validate_model_dir(&app_dir).is_ok() {
            return Some(ModelSource::AppDir(app_dir));
        }
    }

    if allow_hf_cache && let Some(hf_dir) = probe_hf_cache_snapshot(spec.hf_repo) {
        return Some(ModelSource::HfCache(hf_dir));
    }

    None
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

    let hf_consented = opts.allow_hf_cache_override || config.allow_hf_cache;
    if let Some(spec) = find_model_spec(model_id) {
        return Ok(locate_curated_model(spec, hf_consented).map(|src| src.into_resolved(spec)));
    }

    if let Ok(models_root) = AppConfig::models_dir() {
        let local_dir = models_root.join(model_id);
        if local_dir.is_dir() && validate_model_dir(&local_dir).is_ok() {
            return Ok(Some(ResolvedModel {
                label: model_id.clone(),
                dir: local_dir,
                query_prefix: String::new(),
                doc_prefix: String::new(),
            }));
        }
    }

    Ok(None)
}

/// Resolves an explicit CLI model target (directory path or curated model ID) or falls back
/// to the active configured model, failing with `EmbedError::ModelUnavailable` if not installed.
pub fn resolve_model_target(target: Option<&str>, allow_hf_cache: bool) -> Result<ResolvedModel, EmbedError> {
    let config = AppConfig::load()?;

    if let Some(target) = target {
        let path = Utf8Path::new(target);
        if path.is_dir() {
            return resolve_active_model(
                &config,
                &ModelResolveOptions {
                    override_model_dir: Some(path.to_path_buf()),
                    allow_hf_cache_override: allow_hf_cache,
                },
            )?
            .ok_or(EmbedError::ModelUnavailable);
        }

        let spec = find_model_spec(target).ok_or_else(|| EmbedError::UnknownModel(target.to_string()))?;
        let hf_consented = allow_hf_cache || config.allow_hf_cache;
        return locate_curated_model(spec, hf_consented)
            .map(|src| src.into_resolved(spec))
            .ok_or(EmbedError::ModelUnavailable);
    }

    resolve_active_model(
        &config,
        &ModelResolveOptions {
            override_model_dir: None,
            allow_hf_cache_override: allow_hf_cache,
        },
    )?
    .ok_or(EmbedError::ModelUnavailable)
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
