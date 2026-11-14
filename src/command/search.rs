// src/command/search.rs

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;

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
    pub query: SearchQuery,

    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

    #[arg(short = 'n', long, default_value_t = 10)]
    #[builder(default = 10)]
    pub limit: usize,
}

#[derive(Serialize)]
struct SearchPayload {
    query: SearchQuery,
    total_hits: usize,
    hits: Vec<SearchHit>,
}

impl SearchCommand {
    pub fn run(self) -> Result<serde_json::Value, SearchError> {
        let index = RepoIndex::build_from_dir(&self.repo)?;
        let hits = index.search(&self.query, self.limit)?;

        let payload = SearchPayload {
            query: self.query,
            total_hits: hits.len(),
            hits,
        };

        Ok(serde_json::to_value(payload)?)
    }
}
