// src/git/mod.rs

mod files;
mod history;

use std::cell::RefCell;

use camino::Utf8Path;
use camino::Utf8PathBuf;
pub use files::FileCommitHistory;
pub use files::collect_tracked_paths;
use gix::bstr::ByteSlice;
use gix::object::tree::diff::Action;
pub use history::GitHistoryIndex;
use miette::Diagnostic;
use thiserror::Error;

use crate::types::CommitOid;
use crate::types::RepoRelativePath;

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

pub(super) struct WalkedCommit {
    pub oid: CommitOid,
    pub short_oid: String,
    pub author: String,
    pub timestamp: i64,
    pub message: String,
    pub touched_paths: Vec<RepoRelativePath>,
}

pub(super) fn discover_repo(root: &Utf8Path, normalize_prefix: bool) -> Result<(gix::Repository, Option<Utf8PathBuf>), GitHistoryError> {
    let repo = gix::discover(root.as_std_path()).map_err(|source| GitHistoryError::Discover {
        path: root.to_path_buf(),
        source,
    })?;
    let repo_prefix = if normalize_prefix { compute_repo_prefix(&repo, root) } else { None };
    Ok((repo, repo_prefix))
}

fn compute_repo_prefix(repo: &gix::Repository, root: &Utf8Path) -> Option<Utf8PathBuf> {
    repo.workdir()
        .and_then(Utf8Path::from_path)
        .and_then(|wd| {
            let canonical_root = root.canonicalize_utf8().ok()?;
            let canonical_wd = wd.canonicalize_utf8().ok()?;
            canonical_root.strip_prefix(&canonical_wd).ok().map(ToOwned::to_owned)
        })
        .filter(|prefix| !prefix.as_str().is_empty())
}

pub(super) fn normalize_repo_path(path: Utf8PathBuf, repo_prefix: Option<&Utf8Path>) -> Option<RepoRelativePath> {
    match repo_prefix {
        Some(prefix) => path
            .strip_prefix(prefix)
            .ok()
            .map(|stripped| RepoRelativePath::from(stripped.to_path_buf())),
        None => Some(RepoRelativePath::from(path)),
    }
}

pub(super) fn walk_repo_commits(root: &Utf8Path, normalize_prefix: bool) -> Result<Vec<WalkedCommit>, GitHistoryError> {
    let (repo, repo_prefix) = discover_repo(root, normalize_prefix)?;

    let Ok(head_id) = repo.head_id() else {
        return Ok(Vec::new());
    };

    let walk = head_id.ancestors().all().map_err(|e| GitHistoryError::Revision(e.to_string()))?;

    let mut commits = Vec::new();
    for info in walk {
        let Ok(info) = info else { continue };
        let Ok(commit) = info.object() else { continue };

        let oid_str = commit.id.to_string();
        let Ok(oid) = CommitOid::try_from(oid_str.as_str()) else {
            continue;
        };
        let short_oid = oid_str.chars().take(8).collect();
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

        let touched_paths = collect_commit_paths(&repo, &commit, repo_prefix.as_deref());
        commits.push(WalkedCommit {
            oid,
            short_oid,
            author,
            timestamp,
            message,
            touched_paths,
        });
    }

    Ok(commits)
}

fn collect_commit_paths(repo: &gix::Repository, commit: &gix::Commit<'_>, repo_prefix: Option<&Utf8Path>) -> Vec<RepoRelativePath> {
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
        if !loc.is_empty()
            && let Some(normalized) = normalize_repo_path(Utf8PathBuf::from(loc.into_owned()), repo_prefix)
        {
            paths.borrow_mut().push(normalized);
        }
        Ok::<_, gix::Error>(Action::Continue(()))
    });

    paths.into_inner()
}
