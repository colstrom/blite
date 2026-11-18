use std::collections::HashSet;
use std::time::Instant;

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use clap::Subcommand;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::config::AppConfig;
use crate::config::ConfigError;
use crate::embed::EMBED_BATCH_SIZE;
use crate::embed::EmbedError;
use crate::embed::Embedder;
use crate::embed::ModelResolveOptions;
use crate::embed::ResolvedModel;
use crate::embed::compute_doc_embed_hash;
use crate::embed::embedding_cache_dir;
use crate::embed::embedding_cache_path;
use crate::embed::find_model_spec;
use crate::embed::probe_hf_cache_snapshot;
use crate::embed::read_embedding_cache;
use crate::embed::resolve_active_model;
use crate::embed::validate_model_dir;
use crate::embed::write_embedding_cache;
use crate::index::IndexError;
use crate::index::collect_documents_for_diagnostics;

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
    action: EmbeddingsAction,
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
    /// Path to the target repository root
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

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

    /// Include `vendor/` directories when indexing repository files
    #[arg(long)]
    #[builder(default = false)]
    pub include_vendor: bool,

    /// Include untracked files when indexing a Git repository
    #[arg(long)]
    #[builder(default = false)]
    pub untracked: bool,
}

#[derive(Debug, Clone, Args, Builder)]
pub struct EmbeddingsPruneArgs {
    /// Path to the target repository root whose live chunks should be kept
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

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

    /// Include `vendor/` directories when scanning live repository files
    #[arg(long)]
    #[builder(default = false)]
    pub include_vendor: bool,

    /// Include untracked files when scanning a Git repository
    #[arg(long)]
    #[builder(default = false)]
    pub untracked: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsPayload {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub total_chunks: usize,
    pub cached_hits: usize,
    pub newly_computed: usize,
    pub removed_entries: usize,
    pub removed_files: usize,
}

impl EmbeddingsCommand {
    pub fn execute(&self) -> Result<EmbeddingsPayload, EmbeddingsError> {
        match &self.action {
            EmbeddingsAction::Build(args) => execute_build(args),
            EmbeddingsAction::Prune(args) => execute_prune(args),
            EmbeddingsAction::Clear(args) => execute_clear(args),
        }
    }

    pub fn run(self) -> Result<serde_json::Value, EmbeddingsError> {
        let payload = self.execute()?;
        Ok(serde_json::to_value(payload)?)
    }
}

fn resolve_target_model(model_arg: Option<&str>, allow_hf_cache: bool) -> Result<ResolvedModel, EmbedError> {
    let config = AppConfig::load()?;

    if let Some(target) = model_arg {
        let path = camino::Utf8Path::new(target);
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
        let app_dir = AppConfig::models_dir()?.join(spec.id);
        if app_dir.is_dir() && validate_model_dir(&app_dir).is_ok() {
            return Ok(ResolvedModel {
                label: spec.id.to_string(),
                dir: app_dir,
                query_prefix: spec.query_prefix.to_string(),
                doc_prefix: spec.doc_prefix.to_string(),
            });
        }

        if (allow_hf_cache || config.allow_hf_cache)
            && let Some(hf_dir) = probe_hf_cache_snapshot(spec.hf_repo)
        {
            return Ok(ResolvedModel {
                label: format!("{} (hf-cache)", spec.id),
                dir: hf_dir,
                query_prefix: spec.query_prefix.to_string(),
                doc_prefix: spec.doc_prefix.to_string(),
            });
        }

        return Err(EmbedError::ModelUnavailable);
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

fn execute_build(args: &EmbeddingsBuildArgs) -> Result<EmbeddingsPayload, EmbeddingsError> {
    let start = Instant::now();
    let resolved = resolve_target_model(args.model.as_deref(), args.use_hf_cache)?;
    let docs = collect_documents_for_diagnostics(&args.repo, args.include_vendor, args.untracked)?;

    let embedder = Embedder::load(&resolved)?;
    let cache_path = embedding_cache_path(&resolved.label);
    let mut cache = if args.force {
        std::collections::HashMap::new()
    } else {
        cache_path.as_deref().map(read_embedding_cache).unwrap_or_default()
    };

    let mut uncached_docs = Vec::new();
    let mut uncached_hashes = Vec::new();
    let mut cached_hits = 0usize;

    for doc in &docs {
        let hash = compute_doc_embed_hash(&embedder, &doc.text);
        if cache.contains_key(&hash) {
            cached_hits += 1;
        } else {
            uncached_hashes.push(hash);
            uncached_docs.push((doc.label.clone(), doc.text.clone()));
        }
    }

    let newly_computed = uncached_docs.len();
    if !uncached_docs.is_empty() {
        let fresh = embedder.embed_documents_with_progress(&uncached_docs, EMBED_BATCH_SIZE, |done, total, path| {
            if done < total {
                eprint!("\r\x1b[2KEmbedding [{done}/{total}] ({:.1?}) {path}", start.elapsed());
            } else {
                eprint!("\r\x1b[2K");
            }
        })?;
        for (hash, vec) in uncached_hashes.into_iter().zip(fresh) {
            cache.insert(hash, vec);
        }
        if let Some(path) = &cache_path {
            write_embedding_cache(path, &cache);
        }
    }

    eprintln!(
        "\x1b[1;32mEmbeddings ready\x1b[0m for '{}' on {} in {:.1?} ({} total, {} cached, {} computed)",
        resolved.label,
        embedder.device_label(),
        start.elapsed(),
        docs.len(),
        cached_hits,
        newly_computed,
    );

    Ok(EmbeddingsPayload {
        action: "build".to_string(),
        model: Some(resolved.label),
        total_chunks: docs.len(),
        cached_hits,
        newly_computed,
        removed_entries: 0,
        removed_files: 0,
    })
}

fn execute_prune(args: &EmbeddingsPruneArgs) -> Result<EmbeddingsPayload, EmbeddingsError> {
    let docs = collect_documents_for_diagnostics(&args.repo, args.include_vendor, args.untracked)?;
    let mut removed_entries = 0usize;
    let mut removed_files = 0usize;
    let mut model_label = None;

    if args.all {
        let Some(cache_dir) = embedding_cache_dir() else {
            return Ok(EmbeddingsPayload {
                action: "prune".to_string(),
                model: None,
                total_chunks: docs.len(),
                cached_hits: 0,
                newly_computed: 0,
                removed_entries: 0,
                removed_files: 0,
            });
        };

        // Match live hashes against both `doc_prefix: ""` and `doc_prefix: "search_document: "` (Nomic)
        let mut live_hashes = HashSet::with_capacity(docs.len() * 2);
        for doc in &docs {
            live_hashes.insert(crate::embed::hash_doc_with_prefix("", &doc.text, 512));
            live_hashes.insert(crate::embed::hash_doc_with_prefix("search_document: ", &doc.text, 512));
        }

        if let Ok(entries) = fs_err::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                let Ok(path) = Utf8PathBuf::try_from(entry.path()) else { continue };
                if path.extension() != Some("bin") {
                    continue;
                }
                let mut map = read_embedding_cache(&path);
                let before = map.len();
                map.retain(|hash, _| live_hashes.contains(hash));
                let pruned = before.saturating_sub(map.len());
                if pruned > 0 {
                    removed_entries += pruned;
                    if map.is_empty() {
                        let _ = fs_err::remove_file(&path);
                        removed_files += 1;
                    } else {
                        write_embedding_cache(&path, &map);
                    }
                }
            }
        }
    } else {
        let resolved = resolve_target_model(args.model.as_deref(), args.use_hf_cache)?;
        let embedder = Embedder::load(&resolved)?;
        model_label = Some(resolved.label.clone());

        let live_hashes: HashSet<u64> = docs.iter().map(|d| compute_doc_embed_hash(&embedder, &d.text)).collect();

        if let Some(cache_path) = embedding_cache_path(&resolved.label)
            && cache_path.exists()
        {
            let mut map = read_embedding_cache(&cache_path);
            let before = map.len();
            map.retain(|hash, _| live_hashes.contains(hash));
            removed_entries = before.saturating_sub(map.len());
            if removed_entries > 0 {
                if map.is_empty() {
                    let _ = fs_err::remove_file(&cache_path);
                    removed_files = 1;
                } else {
                    write_embedding_cache(&cache_path, &map);
                }
            }
        }
    }

    eprintln!(
        "Pruned {} stale cached embedding(s) ({} empty cache file(s) removed).",
        removed_entries, removed_files
    );

    Ok(EmbeddingsPayload {
        action: "prune".to_string(),
        model: model_label,
        total_chunks: docs.len(),
        cached_hits: 0,
        newly_computed: 0,
        removed_entries,
        removed_files,
    })
}

fn execute_clear(args: &EmbeddingsClearArgs) -> Result<EmbeddingsPayload, EmbeddingsError> {
    let mut removed_files = 0usize;
    let mut model_label = None;

    if args.all {
        if let Some(cache_dir) = embedding_cache_dir()
            && cache_dir.is_dir()
            && let Ok(entries) = fs_err::read_dir(&cache_dir)
        {
            for entry in entries.flatten() {
                let Ok(path) = Utf8PathBuf::try_from(entry.path()) else { continue };
                if path.extension() == Some("bin") && fs_err::remove_file(&path).is_ok() {
                    removed_files += 1;
                }
            }
        }
    } else if let Some(target) = &args.model {
        let label = find_model_spec(target).map(|s| s.id.to_string()).unwrap_or_else(|| target.clone());
        model_label = Some(label.clone());

        if let Some(cache_path) = embedding_cache_path(&label)
            && cache_path.exists()
        {
            fs_err::remove_file(&cache_path)?;
            removed_files = 1;
        }
    }

    eprintln!("Cleared {} cached embedding file(s).", removed_files);

    Ok(EmbeddingsPayload {
        action: "clear".to_string(),
        model: model_label,
        total_chunks: 0,
        cached_hits: 0,
        newly_computed: 0,
        removed_entries: 0,
        removed_files,
    })
}
