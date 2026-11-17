// src/command/search.rs

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::drift::KitchenComparator;
use crate::git::FileCommitHistory;
use crate::index::IndexError;
use crate::index::RepoIndex;
use crate::index::SearchHit;
use crate::types::SearchQuery;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum SearchError {
    #[diagnostic(transparent)]
    Index(#[from] IndexError),

    #[diagnostic(code(blite::search::json))]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct SearchCommand {
    /// Query string to search across repository manifests and code
    pub query: SearchQuery,

    /// Path to the target repository root
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

    /// Optional path to the kitchen repository for drift comparison
    #[arg(skip)]
    pub kitchen: Option<Utf8PathBuf>,

    /// Maximum number of matching documents to return
    #[arg(short = 'n', long, default_value_t = 10)]
    #[builder(default = 10)]
    pub limit: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchPayload {
    pub query: SearchQuery,
    pub total_hits: usize,
    pub hits: Vec<SearchHit>,
}

impl SearchCommand {
    pub fn execute(&self) -> Result<SearchPayload, SearchError> {
        let index = RepoIndex::build_from_dir(&self.repo)?;
        let mut hits = index.search(&self.query, self.limit)?;

        if let Some(kitchen_root) = &self.kitchen {
            let comparator = KitchenComparator::new(&self.repo, kitchen_root);
            for hit in &mut hits {
                hit.drift = Some(comparator.compare(&hit.path));
            }
        }

        let file_history = FileCommitHistory::build_from_dir(&self.repo);
        for hit in &mut hits {
            let commits = file_history.commits_for(&hit.path);
            if !commits.is_empty() {
                hit.recent_commits = commits.to_vec();
            }
        }

        Ok(SearchPayload {
            query: self.query.clone(),
            total_hits: hits.len(),
            hits,
        })
    }

    pub fn run(self) -> Result<serde_json::Value, SearchError> {
        let payload = self.execute()?;
        Ok(serde_json::to_value(payload)?)
    }
}
