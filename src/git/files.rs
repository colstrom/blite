// src/git/files.rs

use std::collections::HashMap;
use std::collections::HashSet;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use gix::bstr::ByteSlice;
use jiff::Timestamp;
use tracing::instrument;

use super::compute_repo_prefix;
use super::normalize_repo_path;
use super::walk_repo_commits;
use crate::types::FileCommit;
use crate::types::RepoRelativePath;

#[derive(Debug, Clone, Default)]
pub struct FileCommitHistory {
    by_path: HashMap<RepoRelativePath, Vec<FileCommit>>,
}

impl FileCommitHistory {
    /// Walks commits from `HEAD` (newest to oldest) and maps each touched
    /// `RepoRelativePath` to the ordered commits that modified it.
    /// Returns an empty map if `root` is not inside a Git repository.
    #[instrument(skip_all, fields(root = %root.as_ref()))]
    pub fn build_from_dir(root: impl AsRef<Utf8Path>) -> Self {
        let Ok(commits) = walk_repo_commits(root.as_ref(), true) else {
            return Self::default();
        };

        let mut by_path: HashMap<RepoRelativePath, Vec<FileCommit>> = HashMap::new();
        for commit in commits {
            let subject = commit.message.lines().next().unwrap_or_default().trim().to_string();
            let committed_at = Timestamp::from_second(commit.timestamp)
                .map(|ts| ts.to_string())
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

            let file_commit = FileCommit {
                oid: commit.oid,
                short_oid: commit.short_oid,
                committed_at,
                timestamp: commit.timestamp,
                author: commit.author,
                subject,
            };

            for path in commit.touched_paths {
                by_path.entry(path).or_default().push(file_commit.clone());
            }
        }

        Self { by_path }
    }

    pub fn commits_for(&self, path: &RepoRelativePath) -> &[FileCommit] {
        self.by_path.get(path).map_or(&[], Vec::as_slice)
    }
}

/// Returns the set of `RepoRelativePath` entries currently tracked in the Git index
/// under `root`, or `None` if `root` is not inside a Git repository or has no index yet.
pub fn collect_tracked_paths(root: impl AsRef<Utf8Path>) -> Option<HashSet<RepoRelativePath>> {
    let root = root.as_ref();
    let repo = gix::discover(root.as_std_path()).ok()?;
    let index = repo.index().ok()?;
    let repo_prefix = compute_repo_prefix(&repo, root);

    let mut tracked = HashSet::with_capacity(index.entries().len());
    for entry in index.entries() {
        let raw_path = entry.path(&index).to_str_lossy();
        if raw_path.is_empty() {
            continue;
        }
        if let Some(normalized) = normalize_repo_path(Utf8PathBuf::from(raw_path.into_owned()), repo_prefix.as_deref()) {
            tracked.insert(normalized);
        }
    }

    if tracked.is_empty() { None } else { Some(tracked) }
}
