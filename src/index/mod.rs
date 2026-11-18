// src/index/mod.rs

mod fusion;
mod ingest;
mod schema;

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::time::Instant;

use bon::Builder;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use fusion::has_field_filter;
use fusion::rrf_fuse;
pub use ingest::DiagnosticDocInput;
pub use ingest::collect_documents_for_diagnostics;
use ingest::collect_repo_documents;
use ingest::format_doc_for_embedding;
use miette::Diagnostic;
use schema::IndexSchema;
use serde::Deserialize;
use serde::Serialize;
use tantivy::Index;
use tantivy::ReloadPolicy;
use tantivy::Searcher;
use tantivy::TantivyDocument;
use tantivy::Term;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::query::TermQuery;
use tantivy::schema::IndexRecordOption;
use tantivy::schema::Value;
use thiserror::Error;
use tracing::debug;
use tracing::instrument;

use crate::config::AppConfig;
use crate::config::ConfigError;
use crate::embed::EmbedError;
use crate::embed::ModelResolveOptions;
use crate::embed::SemanticIndex;
use crate::embed::resolve_active_model;
use crate::k8s::SubstitutionTable;
use crate::types::DriftReport;
use crate::types::FileCommit;
use crate::types::K8sMetadata;
use crate::types::LineRange;
use crate::types::RepoRelativePath;
use crate::types::ResolvedVar;
use crate::types::SearchQuery;
use crate::types::SymbolName;

#[derive(Debug, Error, Diagnostic)]
pub enum IndexError {
    #[error("failed to read file during indexing")]
    #[diagnostic(code(blite::index::io))]
    Io(#[from] std::io::Error),

    #[error("tantivy search engine error: {0}")]
    #[diagnostic(code(blite::index::tantivy))]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("invalid search query: {0}")]
    #[diagnostic(code(blite::index::query_parse))]
    QueryParse(#[from] tantivy::query::QueryParserError),

    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    #[diagnostic(transparent)]
    Embed(#[from] EmbedError),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SemanticMode {
    /// Use semantic search if a model is configured and available locally; otherwise use lexical BM25
    #[default]
    Auto,
    /// Require semantic search and return a diagnostic error if no model is available
    Require,
    /// Disable semantic search even when a model is available
    Off,
}

#[derive(Debug, Clone, Default, Builder)]
pub struct IndexBuildOptions {
    #[builder(default)]
    pub semantic_mode: SemanticMode,
    pub model_dir: Option<Utf8PathBuf>,
    #[builder(default = false)]
    pub allow_hf_cache: bool,
    #[builder(default = false)]
    pub include_vendor: bool,
    #[builder(default = false)]
    pub include_untracked: bool,
    #[builder(default = false)]
    pub show_progress: bool,
    #[builder(default = false)]
    pub no_cache: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: RepoRelativePath,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k8s: Option<K8sMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<SymbolName>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<LineRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_vars: Vec<ResolvedVar>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_commits: Vec<FileCommit>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift: Option<DriftReport>,
}

pub struct RepoIndex {
    index: Index,
    fields: IndexSchema,
    substitutions: SubstitutionTable,
    semantic: Option<SemanticIndex>,
    kinds: Vec<String>,
    namespaces: Vec<String>,
    labels: Vec<String>,
    annotations: Vec<String>,
}

impl RepoIndex {
    pub fn build_from_dir(root: impl AsRef<Utf8Path>) -> Result<Self, IndexError> {
        Self::build_with_options(root, &IndexBuildOptions::default())
    }

    #[instrument(skip_all, fields(root = %root.as_ref()))]
    pub fn build_with_options(root: impl AsRef<Utf8Path>, opts: &IndexBuildOptions) -> Result<Self, IndexError> {
        let root = root.as_ref();
        let docs = collect_repo_documents(root, opts.include_vendor, opts.include_untracked)?;

        // Resolve optional semantic model up front so `SemanticMode::Require` fails fast before indexing
        let resolved_model = match opts.semantic_mode {
            SemanticMode::Off => None,
            SemanticMode::Auto => {
                let cfg = AppConfig::load()?;
                resolve_active_model(
                    &cfg,
                    &ModelResolveOptions {
                        override_model_dir: opts.model_dir.clone(),
                        allow_hf_cache_override: opts.allow_hf_cache,
                    },
                )?
            }
            SemanticMode::Require => {
                let cfg = AppConfig::load()?;
                let found = resolve_active_model(
                    &cfg,
                    &ModelResolveOptions {
                        override_model_dir: opts.model_dir.clone(),
                        allow_hf_cache_override: opts.allow_hf_cache,
                    },
                )?;
                Some(found.ok_or(EmbedError::ModelUnavailable)?)
            }
        };

        let mut substitutions = SubstitutionTable::default();
        let mut kinds = BTreeSet::new();
        let mut namespaces = BTreeSet::new();
        let mut labels = BTreeSet::new();
        let mut annotations = BTreeSet::new();

        for doc in &docs {
            if let Some(parsed) = &doc.parsed_yaml {
                kinds.insert(parsed.metadata.kind.as_str().to_string());
                if let Some(ns) = &parsed.metadata.namespace {
                    namespaces.insert(ns.as_str().to_string());
                }
                for label in &parsed.metadata.labels {
                    if let Some((key, _)) = label.split_once('=') {
                        labels.insert(key.to_string());
                    }
                    labels.insert(label.clone());
                }
                for annotation in &parsed.metadata.annotations {
                    if let Some((key, _)) = annotation.split_once('=') {
                        annotations.insert(key.to_string());
                    }
                    annotations.insert(annotation.clone());
                }
                if let Some(data) = &parsed.configmap_data {
                    substitutions.insert_configmap(&doc.rel_path, &parsed.metadata, data.clone());
                }
            }
        }

        let semantic = if let Some(model) = &resolved_model {
            let embed_docs: Vec<(String, String)> = docs.iter().map(|d| (d.rel_path.to_string(), format_doc_for_embedding(d))).collect();
            let start_embed = Instant::now();
            let show_progress = opts.show_progress;
            let built = SemanticIndex::build_with_progress(model, &embed_docs, !opts.no_cache, |done, total, path| {
                if !show_progress {
                    return;
                }
                if done < total {
                    eprint!("\r\x1b[2KEmbedding [{done}/{total}] ({:.1?}) {path}", start_embed.elapsed());
                } else {
                    eprint!("\r\x1b[2K");
                }
            })?;
            Some(built)
        } else {
            None
        };

        debug!(
            document_count = docs.len(),
            flux_substitutions = substitutions.flux_var_count(),
            semantic_enabled = semantic.is_some(),
            "collected and parsed repository documents"
        );

        let (schema, fields) = IndexSchema::build();
        let index = Index::create_in_ram(schema);
        let mut writer = index.writer(15_000_000)?;

        for (doc_id, item) in docs.into_iter().enumerate() {
            writer.add_document(fields.to_tantivy_doc(doc_id as u64, &item))?;
        }
        writer.commit()?;

        Ok(Self {
            index,
            fields,
            substitutions,
            semantic,
            kinds: kinds.into_iter().collect(),
            namespaces: namespaces.into_iter().collect(),
            labels: labels.into_iter().collect(),
            annotations: annotations.into_iter().collect(),
        })
    }

    pub fn active_semantic_model(&self) -> Option<&str> {
        self.semantic.as_ref().map(|s| s.model_label.as_str())
    }

    #[instrument(skip(self), fields(query = %query))]
    pub fn search(&self, query: &SearchQuery, limit: usize) -> Result<Vec<SearchHit>, IndexError> {
        let reader = self.index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;
        let searcher = reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, self.fields.searchable_fields());
        let parsed_query = query_parser.parse_query(query.as_ref())?;

        // Pure lexical path when semantic search is disabled or no model is installed
        let Some(semantic) = &self.semantic else {
            let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit).order_by_score())?;
            let mut hits = Vec::with_capacity(top_docs.len());
            for (score, doc_address) in top_docs {
                let doc: TantivyDocument = searcher.doc(doc_address)?;
                hits.push(self.fields.hydrate_hit(&doc, score, &self.substitutions));
            }
            return Ok(hits);
        };

        // Hybrid path: gather top lexical + semantic candidates and fuse via Reciprocal Rank Fusion (RRF)
        let candidate_limit = (limit * 4).max(25);
        let lexical_top = searcher.search(&parsed_query, &TopDocs::with_limit(candidate_limit).order_by_score())?;

        let mut docs_by_id: HashMap<usize, TantivyDocument> = HashMap::with_capacity(lexical_top.len());
        let mut lexical_ids = Vec::with_capacity(lexical_top.len());

        for (_bm25_score, doc_address) in lexical_top {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            if let Some(id) = doc.get_first(self.fields.doc_id).and_then(|v| v.as_u64()) {
                let doc_id = id as usize;
                lexical_ids.push(doc_id);
                docs_by_id.insert(doc_id, doc);
            }
        }

        let restrict_to_lexical = has_field_filter(query.as_ref());
        let semantic_top = semantic.search(query.as_ref(), candidate_limit)?;
        let ranked = rrf_fuse(&lexical_ids, &semantic_top, restrict_to_lexical, limit);

        let mut hits = Vec::with_capacity(ranked.len());
        for (doc_id, fused_score) in ranked {
            if let Some(doc) = docs_by_id.remove(&doc_id) {
                hits.push(self.fields.hydrate_hit(&doc, fused_score, &self.substitutions));
            } else if let Some(doc) = self.fetch_doc_by_id(&searcher, doc_id as u64)? {
                hits.push(self.fields.hydrate_hit(&doc, fused_score, &self.substitutions));
            }
        }

        Ok(hits)
    }

    fn fetch_doc_by_id(&self, searcher: &Searcher, target_id: u64) -> Result<Option<TantivyDocument>, IndexError> {
        let query = TermQuery::new(Term::from_field_u64(self.fields.doc_id, target_id), IndexRecordOption::Basic);
        let top = searcher.search(&query, &TopDocs::with_limit(1).order_by_score())?;
        if let Some((_score, addr)) = top.into_iter().next() {
            Ok(Some(searcher.doc(addr)?))
        } else {
            Ok(None)
        }
    }

    pub fn kinds(&self) -> &[String] {
        &self.kinds
    }

    pub fn namespaces(&self) -> &[String] {
        &self.namespaces
    }

    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    pub fn annotations(&self) -> &[String] {
        &self.annotations
    }

    pub fn active_semantic_info(&self) -> Option<(&str, &'static str)> {
        self.semantic.as_ref().map(|s| (s.model_label.as_str(), s.device_label))
    }
}
