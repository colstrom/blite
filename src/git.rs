// src/git.rs

use std::cell::RefCell;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use gix::bstr::ByteSlice;
use gix::object::tree::diff::Action;
use miette::Diagnostic;
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

use crate::types::CommitHit;
use crate::types::CommitOid;
use crate::types::RepoRelativePath;
use crate::types::SearchQuery;

const DEFAULT_MAX_COMMITS: usize = 1_000;

#[derive(Debug, Error, Diagnostic)]
pub enum GitHistoryError {
    #[error("failed to discover git repository at {path}: {source}")]
    #[diagnostic(code(blite::git::discover))]
    Discover {
        path: Utf8PathBuf,
        #[source]
        source: gix::Error,
    },

    #[error("git object or revision error: {0}")]
    #[diagnostic(code(blite::git::revision))]
    Revision(String),

    #[error("tantivy search engine error: {0}")]
    #[diagnostic(code(blite::git::tantivy))]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("invalid history search query: {0}")]
    #[diagnostic(code(blite::git::query_parse))]
    QueryParse(#[from] tantivy::query::QueryParserError),
}

pub struct GitHistoryIndex {
    index: Index,
    oid_field: Field,
    author_field: Field,
    timestamp_field: Field,
    message_field: Field,
    paths_field: Field,
}

impl GitHistoryIndex {
    /// Opens the Git repository at `root` and indexes up to `DEFAULT_MAX_COMMITS`
    /// commits (messages + touched paths) into an in-memory Tantivy index.
    #[instrument(skip_all, fields(root = %root.as_ref()))]
    pub fn build_from_dir(root: impl AsRef<Utf8Path>) -> Result<Self, GitHistoryError> {
        let root = root.as_ref();
        let repo = gix::discover(root.as_std_path()).map_err(|source| GitHistoryError::Discover {
            path: root.to_path_buf(),
            source,
        })?;

        let mut schema_builder = Schema::builder();
        let oid_field = schema_builder.add_text_field("oid", TEXT | STORED);
        let author_field = schema_builder.add_text_field("author", TEXT | STORED);
        let timestamp_field = schema_builder.add_i64_field("timestamp", STORED);
        let message_field = schema_builder.add_text_field("message", TEXT | STORED);
        let paths_field = schema_builder.add_text_field("path", TEXT | STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema);
        let mut writer = index.writer(15_000_000)?;

        let Ok(head_id) = repo.head_id() else {
            // Empty/unborn repository with zero commits
            writer.commit()?;
            return Ok(Self {
                index,
                oid_field,
                author_field,
                timestamp_field,
                message_field,
                paths_field,
            });
        };

        let walk = head_id.ancestors().all().map_err(|e| GitHistoryError::Revision(e.to_string()))?;

        let mut indexed_count = 0usize;
        for info in walk.take(DEFAULT_MAX_COMMITS) {
            let Ok(info) = info else { continue };
            let Ok(commit) = info.object() else { continue };

            let oid_str = commit.id.to_string();
            let message = commit.message_raw_sloppy().to_str_lossy().into_owned();
            let (author, timestamp) = match commit.author() {
                Ok(sig) => {
                    let ts = sig
                        .time()
                        .map(|t| t.seconds)
                        .unwrap_or_else(|_| sig.time.split_whitespace().next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0));
                    (format!("{} <{}>", sig.name.to_str_lossy(), sig.email.to_str_lossy()), ts)
                }
                Err(_) => ("unknown".to_string(), 0),
            };

            let touched_paths = collect_commit_paths(&repo, &commit);

            let mut doc = TantivyDocument::new();
            doc.add_text(oid_field, &oid_str);
            doc.add_text(author_field, &author);
            doc.add_i64(timestamp_field, timestamp);
            doc.add_text(message_field, &message);
            for path in &touched_paths {
                doc.add_text(paths_field, path.as_str());
            }

            writer.add_document(doc)?;
            indexed_count += 1;
        }

        writer.commit()?;
        debug!(commits = indexed_count, "indexed git commit history");

        Ok(Self {
            index,
            oid_field,
            author_field,
            timestamp_field,
            message_field,
            paths_field,
        })
    }

    #[instrument(skip(self), fields(query = %query))]
    pub fn search(&self, query: &SearchQuery, limit: usize) -> Result<Vec<CommitHit>, GitHistoryError> {
        let reader = self.index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;

        let searcher = reader.searcher();
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.oid_field, self.author_field, self.message_field, self.paths_field],
        );
        let parsed_query = query_parser.parse_query(query.as_ref())?;

        let top_docs = searcher.search(&parsed_query, &TopDocs::with_limit(limit).order_by_score())?;
        let mut hits = Vec::with_capacity(top_docs.len());

        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let oid_str = doc
                .get_first(self.oid_field)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let short_oid = oid_str.chars().take(8).collect();
            let author = doc
                .get_first(self.author_field)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let timestamp = doc.get_first(self.timestamp_field).and_then(|v| v.as_i64()).unwrap_or(0);
            let message = doc
                .get_first(self.message_field)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();

            let touched_paths = doc
                .get_all(self.paths_field)
                .filter_map(|v| v.as_str())
                .map(|p| RepoRelativePath::from(Utf8PathBuf::from(p)))
                .collect();

            let Ok(oid) = CommitOid::try_from(oid_str) else {
                continue;
            };

            hits.push(CommitHit {
                oid,
                short_oid,
                author,
                timestamp,
                message,
                touched_paths,
                score,
            });
        }

        Ok(hits)
    }
}

fn collect_commit_paths(repo: &gix::Repository, commit: &gix::Commit<'_>) -> Vec<RepoRelativePath> {
    let Ok(new_tree) = commit.tree() else {
        return Vec::new();
    };

    let old_tree = commit
        .parent_ids()
        .next()
        .and_then(|parent_id| parent_id.object().ok())
        .and_then(|obj| obj.try_into_commit().ok())
        .and_then(|parent_commit| parent_commit.tree().ok())
        .unwrap_or_else(|| repo.empty_tree());

    let paths = RefCell::new(Vec::new());
    let Ok(mut changes) = old_tree.changes() else {
        return Vec::new();
    };

    let _ = changes.for_each_to_obtain_tree(&new_tree, |change| {
        let loc = change.location().to_str_lossy();
        if !loc.is_empty() {
            paths.borrow_mut().push(RepoRelativePath::from(Utf8PathBuf::from(loc.into_owned())));
        }
        Ok::<_, gix::Error>(Action::Continue(()))
    });

    paths.into_inner()
}
