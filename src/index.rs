use std::sync::Mutex;

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
use crate::k8s::ParsedYaml;
use crate::k8s::SubstitutionTable;
use crate::k8s::parse_k8s_yaml;
use crate::types::DriftReport;
use crate::types::K8sKind;
use crate::types::K8sMetadata;
use crate::types::K8sName;
use crate::types::K8sNamespace;
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
}

pub struct RepoIndex {
    index: Index,
    substitutions: SubstitutionTable,
    path_field: Field,
    body_field: Field,
    kind_field: Field,
    name_field: Field,
    ns_field: Field,
    labels_field: Field,
    symbol_field: Field,
    line_start_field: Field,
    line_end_field: Field,
}

impl RepoIndex {
    #[instrument(skip_all, fields(root = %root.as_ref()))]
    pub fn build_from_dir(root: impl AsRef<Utf8Path>) -> Result<Self, IndexError> {
        let root = root.as_ref();
        let docs = collect_repo_documents(root)?;

        let mut substitutions = SubstitutionTable::default();
        for doc in &docs {
            if let Some(parsed) = &doc.parsed_yaml
                && let Some(data) = &parsed.configmap_data
            {
                substitutions.insert_configmap(&doc.rel_path, &parsed.metadata, data.clone());
            }
        }

        debug!(
            document_count = docs.len(),
            flux_substitutions = substitutions.flux_var_count(),
            "collected and parsed repository documents"
        );

        let mut schema_builder = Schema::builder();
        let path_field = schema_builder.add_text_field("path", TEXT | STORED);
        let body_field = schema_builder.add_text_field("body", TEXT | STORED);
        let kind_field = schema_builder.add_text_field("kind", TEXT | STORED);
        let name_field = schema_builder.add_text_field("name", TEXT | STORED);
        let ns_field = schema_builder.add_text_field("ns", TEXT | STORED);
        let labels_field = schema_builder.add_text_field("labels", TEXT | STORED);
        let symbol_field = schema_builder.add_text_field("symbol", TEXT | STORED);
        let line_start_field = schema_builder.add_u64_field("line_start", STORED);
        let line_end_field = schema_builder.add_u64_field("line_end", STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema);
        let mut writer = index.writer(15_000_000)?;

        for item in docs {
            let mut document = TantivyDocument::new();
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
            path_field,
            body_field,
            kind_field,
            name_field,
            ns_field,
            labels_field,
            symbol_field,
            line_start_field,
            line_end_field,
        })
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
                self.symbol_field,
            ],
        );
        let parsed_query = query_parser.parse_query(query.as_ref())?;

        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut hits = Vec::with_capacity(top_docs.len());

        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let path_str = doc.get_first(self.path_field).and_then(|v| v.as_str()).unwrap_or_default();
            let content = doc
                .get_first(self.body_field)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let k8s = self.extract_doc_k8s(&doc);
            let symbol = doc
                .get_first(self.symbol_field)
                .and_then(|v| v.as_str())
                .and_then(|s| SymbolName::try_from(s.to_string()).ok());
            let lines = self.extract_doc_lines(&doc);

            let resolved_vars = if k8s.is_some() {
                self.substitutions.resolve_in_content(&content)
            } else {
                Vec::new()
            };

            hits.push(SearchHit {
                path: RepoRelativePath::from(Utf8PathBuf::from(path_str)),
                score,
                k8s,
                symbol,
                lines,
                resolved_vars,
                content,
                drift: None,
            });
        }

        Ok(hits)
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
            .filter_map(|v| v.as_str().map(ToString::to_string))
            .collect();

        Some(K8sMetadata {
            kind,
            name,
            namespace,
            labels,
        })
    }

    fn extract_doc_lines(&self, doc: &TantivyDocument) -> Option<LineRange> {
        let start = doc.get_first(self.line_start_field).and_then(|v| v.as_u64())? as usize;
        let end = doc.get_first(self.line_end_field).and_then(|v| v.as_u64())? as usize;
        Some(LineRange { start, end })
    }
}

fn collect_repo_documents(root: &Utf8Path) -> Result<Vec<IndexDocument>, IndexError> {
    let raw_files = Mutex::new(Vec::new());

    WalkBuilder::new(root.as_std_path())
        .hidden(false)
        .git_ignore(true)
        .build_parallel()
        .run(|| {
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

                if utf8_path.components().any(|c| c.as_str() == ".git") {
                    return ignore::WalkState::Continue;
                }

                if let Ok(content) = fs_err::read_to_string(utf8_path) {
                    let rel = utf8_path.strip_prefix(root).unwrap_or(utf8_path).to_path_buf();

                    if let Ok(mut guard) = raw_files.lock() {
                        guard.push((RepoRelativePath::from(rel), content));
                    }
                }

                ignore::WalkState::Continue
            })
        });
    let docs = raw_files
        .into_inner()
        .unwrap_or_default()
        .into_par_iter()
        .flat_map(|(rel_path, content)| {
            if let Some(parsed_yaml) = parse_k8s_yaml(&rel_path, &content) {
                return vec![IndexDocument {
                    rel_path,
                    content,
                    parsed_yaml: Some(parsed_yaml),
                    symbol: None,
                    lines: None,
                }];
            }
            let chunks = chunk_code_file(&rel_path, &content);
            if chunks.is_empty() {
                vec![IndexDocument {
                    rel_path,
                    content,
                    parsed_yaml: None,
                    symbol: None,
                    lines: None,
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
                    })
                    .collect()
            }
        })
        .collect();
    Ok(docs)
}
