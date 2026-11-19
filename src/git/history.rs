// src/git/history.rs

use camino::Utf8Path;
use camino::Utf8PathBuf;
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
use tracing::debug;
use tracing::instrument;

use super::GitHistoryError;
use super::walk_repo_commits;
use crate::types::CommitHit;
use crate::types::CommitOid;
use crate::types::RepoRelativePath;
use crate::types::SearchQuery;

pub struct GitHistoryIndex {
    index: Index,
    oid_field: Field,
    author_field: Field,
    timestamp_field: Field,
    message_field: Field,
    paths_field: Field,
}

impl GitHistoryIndex {
    /// Opens the Git repository at `root` and indexes commit messages, authors,
    /// and touched paths into an in-memory Tantivy index.
    #[instrument(skip_all, fields(root = %root.as_ref()))]
    pub fn build_from_dir(root: impl AsRef<Utf8Path>) -> Result<Self, GitHistoryError> {
        let root = root.as_ref();
        let commits = walk_repo_commits(root, false)?;

        let mut schema_builder = Schema::builder();
        let oid_field = schema_builder.add_text_field("oid", TEXT | STORED);
        let author_field = schema_builder.add_text_field("author", TEXT | STORED);
        let timestamp_field = schema_builder.add_i64_field("timestamp", STORED);
        let message_field = schema_builder.add_text_field("message", TEXT | STORED);
        let paths_field = schema_builder.add_text_field("path", TEXT | STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema);
        let mut writer = index.writer(15_000_000)?;
        let indexed_count = commits.len();

        for commit in commits {
            let mut doc = TantivyDocument::new();
            doc.add_text(oid_field, commit.oid);
            doc.add_text(author_field, &commit.author);
            doc.add_i64(timestamp_field, commit.timestamp);
            doc.add_text(message_field, &commit.message);
            for path in &commit.touched_paths {
                doc.add_text(paths_field, path.as_str());
            }
            writer.add_document(doc)?;
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

            let Ok(oid) = CommitOid::try_from(oid_str.as_str()) else {
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
