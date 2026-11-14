// src/index.rs

use std::sync::Mutex;

use bon::Builder;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use ignore::WalkBuilder;
use miette::Diagnostic;
use serde::Serialize;
use tantivy::Index;
use tantivy::ReloadPolicy;
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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SearchHit {
    pub path: RepoRelativePath,
    pub score: f32,
    pub content: String,
}

#[derive(Debug, Clone)]
struct IndexedFile {
    rel_path: RepoRelativePath,
    content: String,
}

#[derive(Builder)]
#[builder(on(PathBuf, into))]
pub(crate) struct RepoIndex {
    index: Index,
    path_field: Field,
    body_field: Field,
}

impl RepoIndex {
    #[instrument(skip_all, fields(root = %root.as_ref()))]
    pub(crate) fn build_from_dir(root: impl AsRef<Utf8Path>) -> Result<Self, IndexError> {
        let root = root.as_ref();
        let files = collect_repo_files(root)?;
        debug!(file_count = files.len(), "collected repository files");

        let mut schema_builder = Schema::builder();
        let path_field = schema_builder.add_text_field("path", TEXT | STORED);
        let body_field = schema_builder.add_text_field("body", TEXT | STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema);
        let mut writer = index.writer(15_000_000)?;

        for file in files {
            writer.add_document(doc!(
                path_field => file.rel_path.as_str(),
                body_field => file.content,
            ))?;
        }
        writer.commit()?;

        Ok(Self {
            index,
            path_field,
            body_field,
        })
    }

    #[instrument(skip(self), fields(query = %query))]
    pub(crate) fn search(&self, query: &SearchQuery, limit: usize) -> Result<Vec<SearchHit>, IndexError> {
        let reader = self.index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;

        let searcher = reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.path_field, self.body_field]);
        let parsed_query = query_parser.parse_query(query.as_ref())?;

        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit).order_by_score())?;

        let mut hits = Vec::with_capacity(top_docs.len());

        for (score, doc_address) in top_docs {
            let doc: tantivy::TantivyDocument = searcher.doc(doc_address)?;
            let path_str = doc.get_first(self.path_field).and_then(|v| v.as_str()).unwrap_or_default();
            let content = doc
                .get_first(self.body_field)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            hits.push(SearchHit {
                path: RepoRelativePath::from(Utf8PathBuf::from(path_str)),
                score,
                content,
            });
        }

        Ok(hits)
    }
}

fn collect_repo_files(root: &Utf8Path) -> Result<Vec<IndexedFile>, IndexError> {
    let collected = Mutex::new(Vec::new());

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

                let Ok(utf8_path) = Utf8Path::from_path(entry.path()).ok_or(()) else {
                    return ignore::WalkState::Continue;
                };

                // Skip .git internals even if hidden(false) is set
                if utf8_path.components().any(|c| c.as_str() == ".git") {
                    return ignore::WalkState::Continue;
                }

                // fs-err accepts anything implementing AsRef<std::path::Path>, which Utf8Path does
                if let Ok(content) = fs_err::read_to_string(utf8_path) {
                    let rel = utf8_path.strip_prefix(root).unwrap_or(utf8_path).to_path_buf();

                    if let Ok(mut guard) = collected.lock() {
                        guard.push(IndexedFile {
                            rel_path: RepoRelativePath::from(rel),
                            content,
                        });
                    }
                }

                ignore::WalkState::Continue
            })
        });

    Ok(collected.into_inner().unwrap_or_default())
}
