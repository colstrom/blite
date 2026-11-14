use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::git::GitHistoryError;
use crate::git::GitHistoryIndex;
use crate::types::CommitHit;
use crate::types::SearchQuery;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum HistoryError {
    #[diagnostic(transparent)]
    Git(#[from] GitHistoryError),

    #[diagnostic(code(blite::history::json))]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct HistoryCommand {
    /// Query string to search across commit messages, authors, and touched paths
    pub query: SearchQuery,

    /// Path to the target repository root
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

    /// Maximum number of matching commits to return
    #[arg(short = 'n', long, default_value_t = 10)]
    #[builder(default = 10)]
    pub limit: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryPayload {
    pub query: SearchQuery,
    pub total_hits: usize,
    pub hits: Vec<CommitHit>,
}

impl HistoryCommand {
    pub fn execute(&self) -> Result<HistoryPayload, HistoryError> {
        let index = GitHistoryIndex::build_from_dir(&self.repo)?;
        let hits = index.search(&self.query, self.limit)?;

        Ok(HistoryPayload {
            query: self.query.clone(),
            total_hits: hits.len(),
            hits,
        })
    }

    pub fn run(self) -> Result<serde_json::Value, HistoryError> {
        let payload = self.execute()?;
        Ok(serde_json::to_value(payload)?)
    }
}
