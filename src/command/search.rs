// src/command/search.rs

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use super::args::IndexFilterArgs;
use super::args::RepoTargetArgs;
use super::args::SemanticArgs;
use crate::command::ExecuteCommand;
use crate::command::json_command;
use crate::drift::KitchenComparator;
use crate::git::FileCommitHistory;
use crate::index::IndexError;
use crate::index::RepoIndex;
use crate::index::SearchHit;
use crate::render::TerminalRenderer;
use crate::types::SearchQuery;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum SearchError {
    #[diagnostic(transparent)]
    Index(#[from] IndexError),

    #[diagnostic(code(blite::search::json))]
    Json(#[from] serde_json::Error),

    #[diagnostic(transparent)]
    Render(#[from] crate::render::RenderError),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct SearchCommand {
    /// Query string to search across repository manifests and code
    pub(crate) query: SearchQuery,

    #[command(flatten)]
    #[builder(default)]
    pub(crate) target: RepoTargetArgs,

    /// Optional path to the kitchen repository for drift comparison
    #[arg(short, long)]
    pub(crate) kitchen: Option<Utf8PathBuf>,

    /// Maximum number of matching documents to return
    #[arg(short = 'n', long, default_value_t = 10)]
    #[builder(default = 10)]
    pub(crate) limit: usize,

    #[command(flatten)]
    #[builder(default)]
    pub(crate) semantic: SemanticArgs,

    #[command(flatten)]
    #[builder(default)]
    pub(crate) filter: IndexFilterArgs,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchPayload {
    pub query: SearchQuery,
    pub total_hits: usize,
    pub hits: Vec<SearchHit>,
}

impl ExecuteCommand for SearchCommand {
    type Output = SearchPayload;
    type Error = SearchError;

    fn execute(self) -> Result<Self::Output, Self::Error> {
        let opts = self.semantic.to_build_options(&self.filter, false);
        let index = RepoIndex::build_with_options(&self.target.repo, &opts)?;
        let mut hits = index.search(&self.query, self.limit)?;

        let comparator = self
            .kitchen
            .as_deref()
            .map(|kitchen_root| KitchenComparator::new(&self.target.repo, kitchen_root));
        let file_history = FileCommitHistory::build_from_dir(&self.target.repo);
        enrich_search_hits(&mut hits, &file_history, comparator.as_ref());

        Ok(SearchPayload {
            query: self.query,
            total_hits: hits.len(),
            hits,
        })
    }

    fn render(output: Self::Output) -> Result<Option<String>, Self::Error> {
        let rendered = TerminalRenderer::new().render_hits(output.query.as_ref(), &output.hits)?;
        Ok(Some(rendered))
    }
}

json_command!(SearchCommand);

pub(crate) fn enrich_search_hits(hits: &mut [SearchHit], history: &FileCommitHistory, comparator: Option<&KitchenComparator>) {
    for hit in hits {
        let commits = history.commits_for(&hit.path);
        if !commits.is_empty() {
            hit.recent_commits = commits.to_vec();
        }
        if let Some(comp) = comparator {
            hit.drift = Some(comp.compare(&hit.path));
        }
    }
}
