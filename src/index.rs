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
use tantivy::doc;
use tantivy::query::QueryParser;
use tantivy::schema::Field;
use tantivy::schema::STORED;
use tantivy::schema::Schema;
use tantivy::schema::TEXT;
use tantivy::schema::Value;
use thiserror::Error;
use tracing::debug;
use tracing::instrument;

use crate::k8s::extract_k8s_metadata;
use crate::types::K8sKind;
use crate::types::K8sMetadata;
use crate::types::K8sName;
use crate::types::K8sNamespace;
use crate::types::RepoRelativePath;
use crate::types::SearchQuery;

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
    pub content: String,
}

#[derive(Debug, Clone)]
struct IndexedFile {
    rel_path: RepoRelativePath,
    content: String,
    k8s: Option<K8sMetadata>,
}

#[derive(Builder)]
#[builder(on(PathBuf, into))]
pub struct RepoIndex {
    index: Index,
    path_field: Field,
    body_field: Field,
    kind_field: Field,
    name_field: Field,
    ns_field: Field,
    labels_field: Field,
}

impl RepoIndex {
    /// Walks `root` in parallel, extracts Kubernetes metadata via `rayon`,
    /// and builds an in-memory Tantivy index.
    #[instrument(skip_all, fields(root = %root.as_ref()))]
    pub fn build_from_dir(root: impl AsRef<Utf8Path>) -> Result<Self, IndexError> {
        let root = root.as_ref();
        let files = collect_repo_files(root)?;
        let k8s_count = files.iter().filter(|f| f.k8s.is_some()).count();
        debug!(
            file_count = files.len(),
            k8s_manifests = k8s_count,
            "collected and parsed repository files"
        );

        let mut schema_builder = Schema::builder();
        let path_field = schema_builder.add_text_field("path", TEXT | STORED);
        let body_field = schema_builder.add_text_field("body", TEXT | STORED);
        let kind_field = schema_builder.add_text_field("kind", TEXT | STORED);
        let name_field = schema_builder.add_text_field("name", TEXT | STORED);
        let ns_field = schema_builder.add_text_field("ns", TEXT | STORED);
        let labels_field = schema_builder.add_text_field("labels", TEXT | STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema);
        let mut writer = index.writer(15_000_000)?;

        for file in files {
            let mut document = TantivyDocument::new();
            document.add_text(path_field, file.rel_path.as_str());
            document.add_text(body_field, &file.content);

            if let Some(k8s) = &file.k8s {
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

            writer.add_document(document)?;
        }
        writer.commit()?;

        Ok(Self {
            index,
            path_field,
            body_field,
            kind_field,
            name_field,
            ns_field,
            labels_field,
        })
    }

    /// Executes a lexical BM25 search against the in-memory index.
    /// Supports structured field prefixes: `kind:`, `name:`, `ns:`, `labels:`, `path:`.
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

            hits.push(SearchHit {
                path: RepoRelativePath::from(Utf8PathBuf::from(path_str)),
                score,
                k8s,
                content,
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
}

fn collect_repo_files(root: &Utf8Path) -> Result<Vec<IndexedFile>, IndexError> {
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

    let files = raw_files
        .into_inner()
        .unwrap_or_default()
        .into_par_iter()
        .map(|(rel_path, content)| {
            let k8s = extract_k8s_metadata(&rel_path, &content);
            IndexedFile { rel_path, content, k8s }
        })
        .collect();

    Ok(files)
}
