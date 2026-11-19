// src/embed/resolve.rs

use std::fmt;
use std::ops::Deref;

use camino::Utf8Path;
use camino::Utf8PathBuf;

use super::EmbedError;
use super::ModelSpec;
use super::catalog::find_model_spec;
use super::catalog::infer_prefixes_from_dir;
use crate::config::AppConfig;

pub const REQUIRED_MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

/// A model directory verified to contain `config.json`, `tokenizer.json`, and `model.safetensors`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedModelDir(Utf8PathBuf);

impl ValidatedModelDir {
    pub fn into_inner(self) -> Utf8PathBuf {
        self.0
    }
}

impl AsRef<Utf8Path> for ValidatedModelDir {
    fn as_ref(&self) -> &Utf8Path {
        &self.0
    }
}

impl Deref for ValidatedModelDir {
    type Target = Utf8Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for ValidatedModelDir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<ValidatedModelDir> for Utf8PathBuf {
    fn from(dir: ValidatedModelDir) -> Self {
        dir.0
    }
}

impl TryFrom<Utf8PathBuf> for ValidatedModelDir {
    type Error = EmbedError;

    fn try_from(dir: Utf8PathBuf) -> Result<Self, Self::Error> {
        for file in REQUIRED_MODEL_FILES {
            if !dir.join(file).is_file() {
                return Err(EmbedError::IncompleteModelDir {
                    path: dir,
                    missing: (*file).to_string(),
                });
            }
        }
        Ok(Self(dir))
    }
}

impl TryFrom<&Utf8Path> for ValidatedModelDir {
    type Error = EmbedError;

    fn try_from(dir: &Utf8Path) -> Result<Self, Self::Error> {
        Self::try_from(dir.to_path_buf())
    }
}

#[derive(Debug, Clone)]
pub enum ModelSource {
    AppDir(ValidatedModelDir),
    HfCache(ValidatedModelDir),
}

impl ModelSource {
    pub fn into_path(self) -> Utf8PathBuf {
        match self {
            Self::AppDir(dir) | Self::HfCache(dir) => dir.into_inner(),
        }
    }

    pub fn into_resolved(self, spec: &ModelSpec) -> ResolvedModel {
        let (label, dir) = match self {
            Self::AppDir(dir) => (spec.id.to_string(), dir),
            Self::HfCache(dir) => (format!("{} (hf-cache)", spec.id), dir),
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
    if let Ok(models_dir) = AppConfig::models_dir()
        && let Ok(app_dir) = ValidatedModelDir::try_from(models_dir.join(spec.id))
    {
        return Some(ModelSource::AppDir(app_dir));
    }

    if allow_hf_cache && let Some(hf_dir) = probe_hf_cache_snapshot(spec.hf_repo) {
        return Some(ModelSource::HfCache(hf_dir));
    }

    None
}

#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub label: String,
    pub dir: ValidatedModelDir,
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
        let validated = ValidatedModelDir::try_from(dir.as_path())?;
        let (query_prefix, doc_prefix) = infer_prefixes_from_dir(&validated);
        return Ok(Some(ResolvedModel {
            label: validated.to_string(),
            dir: validated,
            query_prefix,
            doc_prefix,
        }));
    }

    if let Some(dir) = &config.active_model_path {
        if dir.is_dir() {
            let validated = ValidatedModelDir::try_from(dir.as_path())?;
            let (query_prefix, doc_prefix) = infer_prefixes_from_dir(&validated);
            return Ok(Some(ResolvedModel {
                label: validated.to_string(),
                dir: validated,
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

    if let Ok(models_root) = AppConfig::models_dir()
        && let Ok(local_dir) = ValidatedModelDir::try_from(models_root.join(model_id.as_ref()))
    {
        return Ok(Some(ResolvedModel {
            label: model_id.to_string(),
            dir: local_dir,
            query_prefix: String::new(),
            doc_prefix: String::new(),
        }));
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

pub fn probe_hf_cache_snapshot(hf_repo: &str) -> Option<ValidatedModelDir> {
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
        if let Ok(validated) = ValidatedModelDir::try_from(path) {
            return Some(validated);
        }
    }

    None
}
