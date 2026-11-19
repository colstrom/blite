// src/command/repl/history.rs

use camino::Utf8Path;
use camino::Utf8PathBuf;
use rustyline::Editor;
use rustyline::history::DefaultHistory;
use tracing::debug;

use super::helper::ReplHelper;
use crate::command::args::ReplHistoryArgs;
use crate::hash::fnv1a_64;

#[derive(Debug, Clone)]
pub(super) enum HistoryTarget {
    Global,
    RepoScoped(Utf8PathBuf),
    Custom(Utf8PathBuf),
    Disabled,
}

impl HistoryTarget {
    pub fn from_args(args: &ReplHistoryArgs, repo: &Utf8Path) -> Self {
        if args.no_history {
            Self::Disabled
        } else if let Some(path) = &args.history_file {
            Self::Custom(path.clone())
        } else if args.repo_history {
            Self::RepoScoped(repo.to_path_buf())
        } else {
            Self::Global
        }
    }

    pub fn resolve_path(&self) -> Option<Utf8PathBuf> {
        match self {
            Self::Disabled => None,
            Self::Custom(path) => Some(path.clone()),
            Self::Global => default_state_dir().map(|dir| dir.join("history")),
            Self::RepoScoped(repo) => {
                let canonical = repo.canonicalize_utf8().unwrap_or_else(|_| repo.clone());
                let key = fnv1a_64(canonical.as_str().as_bytes());
                default_state_dir().map(|dir| dir.join(format!("history-{key:016x}")))
            }
        }
    }
}

pub(super) fn load_history(rl: &mut Editor<ReplHelper, DefaultHistory>, path: &Utf8Path) {
    if path.exists()
        && let Err(err) = rl.load_history(path.as_std_path())
    {
        debug!(path = %path, error = %err, "failed to load repl history");
    }
}

pub(super) fn save_history(rl: &mut Editor<ReplHelper, DefaultHistory>, path: &Utf8Path) {
    if let Some(parent) = path.parent()
        && let Err(err) = fs_err::create_dir_all(parent)
    {
        debug!(path = %parent, error = %err, "failed to create repl history directory");
        return;
    }

    if let Err(err) = rl.save_history(path.as_std_path()) {
        debug!(path = %path, error = %err, "failed to save repl history");
    }
}

fn default_state_dir() -> Option<Utf8PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Utf8PathBuf::try_from(base).ok().map(|dir| dir.join("blite"))
}
