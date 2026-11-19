// src/command/history.rs

use bon::Builder;
use clap::Args;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use super::args::RepoTargetArgs;
use crate::command::ExecuteCommand;
use crate::command::json_command;
use crate::git::GitHistoryError;
use crate::git::GitHistoryIndex;
use crate::render::TerminalRenderer;
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
    pub(crate) query: SearchQuery,

    #[command(flatten)]
    #[builder(default)]
    pub(crate) target: RepoTargetArgs,

    /// Maximum number of matching commits to return
    #[arg(short = 'n', long, default_value_t = 10)]
    #[builder(default = 10)]
    pub(crate) limit: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryPayload {
    pub(crate) query: SearchQuery,
    pub(crate) total_hits: usize,
    pub(crate) hits: Vec<CommitHit>,
}

impl ExecuteCommand for HistoryCommand {
    type Error = HistoryError;
    type Output = HistoryPayload;

    fn execute(self) -> Result<Self::Output, Self::Error> {
        let index = GitHistoryIndex::build_from_dir(&self.target.repo)?;
        let hits = index.search(&self.query, self.limit)?;

        Ok(HistoryPayload {
            query: self.query.clone(),
            total_hits: hits.len(),
            hits,
        })
    }

    fn render(output: Self::Output) -> Result<Option<String>, Self::Error> {
        Ok(Some(
            TerminalRenderer::new().render_commit_hits(output.query.as_ref(), &output.hits),
        ))
    }
}

json_command!(HistoryCommand);
