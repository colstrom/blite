// src/command/repl/session.rs

use std::time::Instant;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use tracing::debug;
use tracing::info;

use super::ReplError;
use crate::drift::KitchenComparator;
use crate::git::FileCommitHistory;
use crate::git::GitHistoryError;
use crate::git::GitHistoryIndex;
use crate::index::IndexBuildOptions;
use crate::index::IndexError;
use crate::index::RepoIndex;
use crate::index::SearchHit;
use crate::types::CommitHit;
use crate::types::SearchQuery;

pub struct ReplSession {
    repo: Utf8PathBuf,
    build_opts: IndexBuildOptions,
    index: RepoIndex,
    git_index: Option<GitHistoryIndex>,
    file_history: Option<FileCommitHistory>,
    comparator: Option<KitchenComparator>,
    limit: usize,
}

impl ReplSession {
    pub fn new(repo: Utf8PathBuf, kitchen: Option<&Utf8Path>, limit: usize, build_opts: IndexBuildOptions) -> Result<Self, IndexError> {
        let index = RepoIndex::build_with_options(&repo, &build_opts)?;
        let comparator = kitchen.map(|k| KitchenComparator::new(&repo, k));

        Ok(Self {
            repo,
            build_opts,
            index,
            git_index: None,
            file_history: None,
            comparator,
            limit,
        })
    }

    pub fn index(&self) -> &RepoIndex {
        &self.index
    }

    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    pub fn reload(&mut self) -> Result<(), IndexError> {
        let reload_start = Instant::now();
        self.index = RepoIndex::build_with_options(&self.repo, &self.build_opts)?;
        self.git_index = None;
        self.file_history = None;
        info!(elapsed = ?reload_start.elapsed(), "reloaded repository index");
        Ok(())
    }

    pub fn search(&mut self, query: &SearchQuery) -> Result<Vec<SearchHit>, ReplError> {
        let query_start = Instant::now();
        let repo = &self.repo;
        let history = self.file_history.get_or_insert_with(|| FileCommitHistory::build_from_dir(repo));
        let mut hits = self.index.search(query, self.limit)?;

        for hit in &mut hits {
            let commits = history.commits_for(&hit.path);
            if !commits.is_empty() {
                hit.recent_commits = commits.to_vec();
            }
            if let Some(comp) = &self.comparator {
                hit.drift = Some(comp.compare(&hit.path));
            }
        }

        debug!(
            hits = hits.len(),
            elapsed = ?query_start.elapsed(),
            "executed interactive search"
        );

        Ok(hits)
    }

    pub fn search_history(&mut self, query: &SearchQuery) -> Result<Vec<CommitHit>, GitHistoryError> {
        if self.git_index.is_none() {
            let git_start = Instant::now();
            let built = GitHistoryIndex::build_from_dir(&self.repo)?;
            eprintln!("\x1b[2mIndexed git history in {:.0?}.\x1b[0m", git_start.elapsed());
            self.git_index = Some(built);
        }

        let g_idx = self.git_index.as_ref().expect("git_index initialized above");
        g_idx.search(query, self.limit)
    }
}
