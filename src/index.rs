// src/index.rs

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::Mutex;

use bon::Builder;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use ignore::WalkBuilder;
use miette::Diagnostic;
use rayon::prelude::*;
use serde::Deserialize;
use serde::Serialize;
use tantivy::Index;
use tantivy::ReloadPolicy;
use tantivy::TantivyDocument;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::Field;
use tantivy::schema::STORED;
use tantivy::schema::Schema;
use tantivy::schema::TEXT;
use tantivy::schema::Value;
use thiserror::Error;
use tracing::debug;
use tracing::instrument;

use crate::code::chunk_code_file;
use crate::config::AppConfig;
use crate::config::ConfigError;
use crate::embed::EmbedError;
use crate::embed::ModelResolveOptions;
use crate::embed::SemanticIndex;
use crate::embed::resolve_active_model;
use crate::k8s::ParsedYaml;
use crate::k8s::SubstitutionTable;
use crate::k8s::parse_k8s_yaml_documents;
use crate::types::DriftReport;
use crate::types::FileCommit;
use crate::types::K8sKind;
use crate::types::K8sMetadata;
use crate::types::K8sName;
use crate::types::K8sNamespace;
use crate::types::LineRange;
use crate::types::RepoRelativePath;
use crate::types::ResolvedVar;
use crate::types::SearchQuery;
use crate::types::SymbolName;

const RRF_K: f32 = 60.0;

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

#[derive(Debug, Clone)]
struct IndexDocument {
    rel_path: RepoRelativePath,
    content: String,
    parsed_yaml: Option<ParsedYaml>,
    symbol: Option<SymbolName>,
    lines: Option<LineRange>,
    is_multi_doc: bool,
}

pub struct RepoIndex {
    index: Index,
    substitutions: SubstitutionTable,
    semantic: Option<SemanticIndex>,
    kinds: Vec<String>,
    namespaces: Vec<String>,
    labels: Vec<String>,
    annotations: Vec<String>,
    doc_id_field: Field,
    path_field: Field,
    body_field: Field,
    kind_field: Field,
    name_field: Field,
    ns_field: Field,
    labels_field: Field,
    annotations_field: Field,
    symbol_field: Field,
    line_start_field: Field,
    line_end_field: Field,
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
            let start_embed = std::time::Instant::now();
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

        let mut schema_builder = Schema::builder();
        let doc_id_field = schema_builder.add_u64_field("doc_id", tantivy::schema::INDEXED | STORED);
        let path_field = schema_builder.add_text_field("path", TEXT | STORED);
        let body_field = schema_builder.add_text_field("body", TEXT | STORED);
        let kind_field = schema_builder.add_text_field("kind", TEXT | STORED);
        let name_field = schema_builder.add_text_field("name", TEXT | STORED);
        let ns_field = schema_builder.add_text_field("ns", TEXT | STORED);
        let labels_field = schema_builder.add_text_field("labels", TEXT | STORED);
        let annotations_field = schema_builder.add_text_field("annotations", TEXT | STORED);
        let symbol_field = schema_builder.add_text_field("symbol", TEXT | STORED);
        let line_start_field = schema_builder.add_u64_field("line_start", STORED);
        let line_end_field = schema_builder.add_u64_field("line_end", STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema);
        let mut writer = index.writer(15_000_000)?;

        for (doc_id, item) in docs.into_iter().enumerate() {
            let mut document = TantivyDocument::new();
            document.add_u64(doc_id_field, doc_id as u64);
            document.add_text(path_field, item.rel_path.as_str());
            document.add_text(body_field, &item.content);

            if let Some(parsed) = &item.parsed_yaml {
                let k8s = &parsed.metadata;
                document.add_text(kind_field, k8s.kind.as_str());
                if let Some(name) = &k8s.name {
                    document.add_text(name_field, name.as_str());
                }
                if let Some(ns) = &k8s.namespace {
                    document.add_text(ns_field, ns.as_str());
                }
                for label in &k8s.labels {
                    document.add_text(labels_field, label);
                    if let Some((key, _)) = label.split_once('=') {
                        document.add_text(labels_field, key);
                    }
                }
                for annotation in &k8s.annotations {
                    document.add_text(annotations_field, annotation);
                    if let Some((key, _)) = annotation.split_once('=') {
                        document.add_text(annotations_field, key);
                    }
                }
            }

            if let Some(symbol) = &item.symbol {
                document.add_text(symbol_field, symbol.as_str());
            }

            if let Some(lines) = item.lines {
                document.add_u64(line_start_field, lines.start as u64);
                document.add_u64(line_end_field, lines.end as u64);
            }

            writer.add_document(document)?;
        }
        writer.commit()?;

        Ok(Self {
            index,
            substitutions,
            semantic,
            kinds: kinds.into_iter().collect(),
            namespaces: namespaces.into_iter().collect(),
            labels: labels.into_iter().collect(),
            annotations: annotations.into_iter().collect(),
            doc_id_field,
            path_field,
            body_field,
            kind_field,
            name_field,
            ns_field,
            labels_field,
            annotations_field,
            symbol_field,
            line_start_field,
            line_end_field,
        })
    }

    pub fn active_semantic_model(&self) -> Option<&str> {
        self.semantic.as_ref().map(|s| s.model_label.as_str())
    }

    #[instrument(skip(self), fields(query = %query))]
    pub fn search(&self, query: &SearchQuery, limit: usize) -> Result<Vec<SearchHit>, IndexError> {
        let reader = self.index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;
        let searcher = reader.searcher();
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.path_field,
                self.body_field,
                self.kind_field,
                self.name_field,
                self.ns_field,
                self.labels_field,
                self.annotations_field,
                self.symbol_field,
            ],
        );
        let parsed_query = query_parser.parse_query(query.as_ref())?;

        // Pure lexical path when semantic search is disabled or no model is installed
        let Some(semantic) = &self.semantic else {
            let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit).order_by_score())?;
            let mut hits = Vec::with_capacity(top_docs.len());
            for (score, doc_address) in top_docs {
                let doc: TantivyDocument = searcher.doc(doc_address)?;
                hits.push(self.build_search_hit(&doc, score));
            }
            return Ok(hits);
        };

        // Hybrid path: gather top lexical + semantic candidates and fuse via Reciprocal Rank Fusion (RRF)
        let candidate_limit = (limit * 4).max(25);
        let lexical_top = searcher.search(&parsed_query, &TopDocs::with_limit(candidate_limit).order_by_score())?;

        let mut docs_by_id: HashMap<usize, TantivyDocument> = HashMap::new();
        let mut rrf_scores: HashMap<usize, f32> = HashMap::new();

        for (rank, (_bm25_score, doc_address)) in lexical_top.into_iter().enumerate() {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            if let Some(id) = doc.get_first(self.doc_id_field).and_then(|v| v.as_u64()) {
                let doc_id = id as usize;
                *rrf_scores.entry(doc_id).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f32);
                docs_by_id.insert(doc_id, doc);
            }
        }

        // If the query uses field filters (kind:, ns:, labels:, etc.), keep semantic fusion
        // scoped to documents matching the structured query so field filters remain strict.
        let restrict_to_lexical = has_field_filter(query.as_ref());
        let semantic_top = semantic.search(query.as_ref(), candidate_limit)?;

        for (rank, (doc_id, _cosine)) in semantic_top.into_iter().enumerate() {
            if restrict_to_lexical && !docs_by_id.contains_key(&doc_id) {
                continue;
            }
            *rrf_scores.entry(doc_id).or_insert(0.0) += 1.0 / (RRF_K + (rank + 1) as f32);
        }

        let mut ranked: Vec<(usize, f32)> = rrf_scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.truncate(limit);

        let mut hits = Vec::with_capacity(ranked.len());
        for (doc_id, fused_score) in ranked {
            if let Some(doc) = docs_by_id.remove(&doc_id) {
                hits.push(self.build_search_hit(&doc, fused_score));
            } else if let Some(doc) = self.fetch_doc_by_id(&searcher, doc_id as u64)? {
                hits.push(self.build_search_hit(&doc, fused_score));
            }
        }

        Ok(hits)
    }

    fn fetch_doc_by_id(&self, searcher: &tantivy::Searcher, target_id: u64) -> Result<Option<TantivyDocument>, IndexError> {
        let query = tantivy::query::TermQuery::new(
            tantivy::Term::from_field_u64(self.doc_id_field, target_id),
            tantivy::schema::IndexRecordOption::Basic,
        );
        let top = searcher.search(&query, &TopDocs::with_limit(1).order_by_score())?;
        if let Some((_score, addr)) = top.into_iter().next() {
            Ok(Some(searcher.doc(addr)?))
        } else {
            Ok(None)
        }
    }

    fn build_search_hit(&self, doc: &TantivyDocument, score: f32) -> SearchHit {
        let path_str = doc.get_first(self.path_field).and_then(|v| v.as_str()).unwrap_or_default();
        let content = doc
            .get_first(self.body_field)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let k8s = self.extract_doc_k8s(doc);
        let symbol = doc
            .get_first(self.symbol_field)
            .and_then(|v| v.as_str())
            .and_then(|s| SymbolName::try_from(s.to_string()).ok());
        let lines = self.extract_doc_lines(doc);

        let resolved_vars = if k8s.is_some() {
            self.substitutions.resolve_in_content(&content)
        } else {
            Vec::new()
        };

        SearchHit {
            path: RepoRelativePath::from(Utf8PathBuf::from(path_str)),
            score,
            k8s,
            symbol,
            lines,
            resolved_vars,
            recent_commits: Vec::new(),
            content,
            drift: None,
        }
    }

    fn extract_doc_k8s(&self, doc: &TantivyDocument) -> Option<K8sMetadata> {
        let kind_str = doc.get_first(self.kind_field).and_then(|v| v.as_str())?;
        let kind = K8sKind::try_from(kind_str.to_string()).ok()?;

        let name = doc
            .get_first(self.name_field)
            .and_then(|v| v.as_str())
            .and_then(|s| K8sName::try_from(s.to_string()).ok());

        let namespace = doc
            .get_first(self.ns_field)
            .and_then(|v| v.as_str())
            .and_then(|s| K8sNamespace::try_from(s.to_string()).ok());

        let labels = doc
            .get_all(self.labels_field)
            .filter_map(|v| v.as_str())
            .filter(|s| s.contains('='))
            .map(ToString::to_string)
            .collect();

        let annotations = doc
            .get_all(self.annotations_field)
            .filter_map(|v| v.as_str())
            .filter(|s| s.contains('='))
            .map(ToString::to_string)
            .collect();

        Some(K8sMetadata {
            kind,
            name,
            namespace,
            labels,
            annotations,
        })
    }

    fn extract_doc_lines(&self, doc: &TantivyDocument) -> Option<LineRange> {
        let start = doc.get_first(self.line_start_field).and_then(|v| v.as_u64())? as usize;
        let end = doc.get_first(self.line_end_field).and_then(|v| v.as_u64())? as usize;
        Some(LineRange { start, end })
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

fn format_doc_for_embedding(item: &IndexDocument) -> String {
    if let Some(parsed) = &item.parsed_yaml {
        let k8s = &parsed.metadata;
        let name = k8s.name.as_deref().map_or("unnamed", |v| v);
        let ns = k8s.namespace.as_deref().map_or("cluster-scoped", |v| v);
        return format!("{} {} in {} ({}):\n{}", k8s.kind, name, ns, item.rel_path, item.content);
    }
    if let Some(sym) = &item.symbol {
        return format!("{} ({}):\n{}", sym, item.rel_path, item.content);
    }
    format!("{}:\n{}", item.rel_path, item.content)
}

fn has_field_filter(query: &str) -> bool {
    ["kind:", "ns:", "name:", "labels:", "annotations:", "symbol:", "path:"]
        .iter()
        .any(|prefix| query.contains(prefix))
}

fn collect_repo_documents(root: &Utf8Path, include_vendor: bool, include_untracked: bool) -> Result<Vec<IndexDocument>, IndexError> {
    let tracked_paths = if include_untracked {
        None
    } else {
        crate::git::collect_tracked_paths(root)
    };

    let raw_files = Mutex::new(Vec::new());
    let mut builder = WalkBuilder::new(root.as_std_path());
    builder.hidden(false).git_ignore(true).add_custom_ignore_filename(".bliteignore");

    if let Ok(cfg_dir) = AppConfig::config_dir() {
        let global_ignore = cfg_dir.join("ignore");
        if global_ignore.is_file() {
            let _ = builder.add_ignore(global_ignore.as_std_path());
        }
    }

    builder.build_parallel().run(|| {
        Box::new(|entry| {
            let Ok(entry) = entry else {
                return ignore::WalkState::Continue;
            };

            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return ignore::WalkState::Continue;
            }

            let Some(utf8_path) = Utf8Path::from_path(entry.path()) else {
                return ignore::WalkState::Continue;
            };

            let rel = utf8_path.strip_prefix(root).unwrap_or(utf8_path).to_path_buf();

            let is_ignored_dir = rel.components().any(|c| {
                let name = c.as_str();
                name == ".git" || (!include_vendor && name == "vendor")
            });
            if is_ignored_dir {
                return ignore::WalkState::Continue;
            }

            let rel_path = RepoRelativePath::from(rel);
            if let Some(tracked) = &tracked_paths
                && !tracked.contains(&rel_path)
            {
                return ignore::WalkState::Continue;
            }

            if let Ok(content) = fs_err::read_to_string(utf8_path)
                && let Ok(mut guard) = raw_files.lock()
            {
                guard.push((rel_path, content));
            }

            ignore::WalkState::Continue
        })
    });

    let docs = raw_files
        .into_inner()
        .unwrap_or_default()
        .into_par_iter()
        .flat_map(|(rel_path, content)| {
            let yaml_docs = parse_k8s_yaml_documents(&rel_path, &content);
            if !yaml_docs.is_empty() {
                return yaml_docs
                    .into_iter()
                    .map(|yd| IndexDocument {
                        rel_path: rel_path.clone(),
                        content: yd.content,
                        parsed_yaml: Some(yd.parsed),
                        symbol: None,
                        lines: yd.lines,
                        is_multi_doc: yd.is_multi_doc,
                    })
                    .collect();
            }

            let chunks = chunk_code_file(&rel_path, &content);
            if chunks.is_empty() {
                vec![IndexDocument {
                    rel_path,
                    content,
                    parsed_yaml: None,
                    symbol: None,
                    lines: None,
                    is_multi_doc: false,
                }]
            } else {
                chunks
                    .into_iter()
                    .map(|chunk| IndexDocument {
                        rel_path: rel_path.clone(),
                        content: chunk.content,
                        parsed_yaml: None,
                        symbol: chunk.symbol,
                        lines: Some(chunk.lines),
                        is_multi_doc: false,
                    })
                    .collect()
            }
        })
        .collect();
    Ok(docs)
}

#[derive(Debug, Clone)]
pub struct DiagnosticDocInput {
    pub label: String,
    pub text: String,
    pub is_multi_doc: bool,
}

pub fn collect_documents_for_diagnostics(
    root: impl AsRef<Utf8Path>,
    include_vendor: bool,
    include_untracked: bool,
) -> Result<Vec<DiagnosticDocInput>, IndexError> {
    let docs = collect_repo_documents(root.as_ref(), include_vendor, include_untracked)?;
    Ok(docs
        .iter()
        .map(|d| {
            let label = match d.lines {
                Some(r) => format!("{}:{}..{}", d.rel_path, r.start, r.end),
                None => d.rel_path.to_string(),
            };
            DiagnosticDocInput {
                label,
                text: format_doc_for_embedding(d),
                is_multi_doc: d.is_multi_doc,
            }
        })
        .collect())
}
