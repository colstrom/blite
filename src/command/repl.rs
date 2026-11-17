// src/command/repl.rs

use std::time::Instant;

use bon::Builder;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use clap::Args;
use miette::Diagnostic;
use rustyline::Config;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use thiserror::Error;
use tracing::debug;
use tracing::info;

use crate::drift::KitchenComparator;
use crate::git::GitHistoryIndex;
use crate::index::IndexError;
use crate::index::RepoIndex;
use crate::render::RenderError;
use crate::render::TerminalRenderer;
use crate::types::SearchQuery;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum ReplError {
    #[diagnostic(transparent)]
    Index(#[from] IndexError),

    #[diagnostic(transparent)]
    Render(#[from] RenderError),

    #[error("terminal line editor error: {0}")]
    #[diagnostic(code(blite::repl::readline))]
    Readline(#[from] ReadlineError),
}

#[derive(Debug, Clone, Args, Builder)]
pub struct ReplCommand {
    /// Path to the target repository root
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

    /// Optional path to the kitchen repository for drift comparison
    #[arg(skip)]
    pub kitchen: Option<Utf8PathBuf>,

    /// Maximum number of matching documents to return per query
    #[arg(short = 'n', long, default_value_t = 5)]
    #[builder(default = 5)]
    pub limit: usize,

    /// Explicit path to the REPL history file
    #[arg(long, conflicts_with_all = ["repo_history", "no_history"])]
    pub history_file: Option<Utf8PathBuf>,

    /// Scope REPL history per repository instead of sharing globally
    #[arg(long, conflicts_with_all = ["history_file", "no_history"])]
    #[builder(default = false)]
    pub repo_history: bool,

    /// Disable reading and writing REPL history
    #[arg(long, conflicts_with_all = ["history_file", "repo_history"])]
    #[builder(default = false)]
    pub no_history: bool,
}

#[derive(Debug, Clone)]
enum HistoryTarget {
    Global,
    RepoScoped(Utf8PathBuf),
    Custom(Utf8PathBuf),
    Disabled,
}

impl HistoryTarget {
    fn resolve_path(&self) -> Option<Utf8PathBuf> {
        match self {
            Self::Disabled => None,
            Self::Custom(path) => Some(path.clone()),
            Self::Global => default_state_dir().map(|dir| dir.join("history")),
            Self::RepoScoped(repo) => {
                let canonical = repo.canonicalize_utf8().unwrap_or_else(|_| repo.clone());
                let key = hash_repo_path(&canonical);
                default_state_dir().map(|dir| dir.join(format!("history-{key:016x}")))
            }
        }
    }
}

fn default_state_dir() -> Option<Utf8PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_local_dir)?;
    Utf8PathBuf::try_from(base).ok().map(|dir| dir.join("blite"))
}

fn hash_repo_path(path: &Utf8Path) -> u64 {
    // 64-bit FNV-1a for deterministic cross-run filenames without extra dependencies
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for byte in path.as_str().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

enum ReplAction {
    Continue,
    Exit,
    Reload,
    SetLimit(usize),
    Help,
    HistoryPath,
    History(SearchQuery),
    Search(SearchQuery),
}

impl ReplCommand {
    fn history_target(&self) -> HistoryTarget {
        if self.no_history {
            HistoryTarget::Disabled
        } else if let Some(path) = &self.history_file {
            HistoryTarget::Custom(path.clone())
        } else if self.repo_history {
            HistoryTarget::RepoScoped(self.repo.clone())
        } else {
            HistoryTarget::Global
        }
    }

    pub fn run(self) -> Result<serde_json::Value, ReplError> {
        let start = Instant::now();
        let mut index = RepoIndex::build_from_dir(&self.repo)?;
        let mut git_index: Option<GitHistoryIndex> = None;
        let renderer = TerminalRenderer::new();
        let comparator = self.kitchen.as_ref().map(|k| KitchenComparator::new(&self.repo, k));
        let mut limit = self.limit;

        let rl_config = Config::builder()
            .max_history_size(1_000)?
            .history_ignore_dups(true)?
            .history_ignore_space(true)
            .build();
        let mut rl = DefaultEditor::with_config(rl_config)?;
        let history_path = self.history_target().resolve_path();

        if let Some(path) = &history_path
            && path.exists()
            && let Err(err) = rl.load_history(path.as_std_path())
        {
            debug!(path = %path, error = %err, "failed to load repl history");
        }

        let history_status = match &history_path {
            Some(path) => path.as_str(),
            None => "disabled",
        };

        eprintln!(
            "\x1b[1;32mIndexed {}\x1b[0m in {:.0?} \x1b[2m(history: {})\x1b[0m\nType :help for commands, :exit to quit",
            self.repo,
            start.elapsed(),
            history_status,
        );

        loop {
            match rl.readline("\x1b[1;34mblite>\x1b[0m ") {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if history_path.is_some() {
                        let _ = rl.add_history_entry(trimmed);
                    }

                    match parse_input(trimmed) {
                        ReplAction::Continue => {}
                        ReplAction::Exit => break,
                        ReplAction::Help => print_help(),
                        ReplAction::SetLimit(new_limit) => {
                            limit = new_limit;
                            eprintln!("Result limit set to {limit}.");
                        }
                        ReplAction::Reload => {
                            let reload_start = Instant::now();
                            match RepoIndex::build_from_dir(&self.repo) {
                                Ok(new_index) => {
                                    index = new_index;
                                    // Clear lazy git index so next :history picks up new commits
                                    git_index = None;
                                    info!(elapsed = ?reload_start.elapsed(), "reloaded repository index");
                                    eprintln!("Reloaded index in {:.0?}.", reload_start.elapsed());
                                }
                                Err(err) => {
                                    eprintln!("\x1b[1;31mReload failed:\x1b[0m {err}");
                                }
                            }
                        }
                        ReplAction::History(query) => {
                            if git_index.is_none() {
                                let git_start = Instant::now();
                                match GitHistoryIndex::build_from_dir(&self.repo) {
                                    Ok(built) => {
                                        eprintln!("\x1b[2mIndexed git history in {:.0?}.\x1b[0m", git_start.elapsed());
                                        git_index = Some(built);
                                    }
                                    Err(err) => {
                                        eprintln!("\x1b[1;31mGit history error:\x1b[0m {err}");
                                        continue;
                                    }
                                }
                            }

                            if let Some(g_idx) = &git_index {
                                match g_idx.search(&query, limit) {
                                    Ok(hits) => {
                                        let rendered = renderer.render_commit_hits(query.as_ref(), &hits);
                                        print!("{rendered}");
                                    }
                                    Err(err) => {
                                        eprintln!("\x1b[1;31mHistory query error:\x1b[0m {err}");
                                    }
                                }
                            }
                        }
                        ReplAction::HistoryPath => match &history_path {
                            Some(path) => eprintln!("REPL history: {path}"),
                            None => eprintln!("REPL history: disabled"),
                        },
                        ReplAction::Search(query) => {
                            let query_start = Instant::now();
                            match index.search(&query, limit) {
                                Ok(mut hits) => {
                                    if let Some(comp) = &comparator {
                                        for hit in &mut hits {
                                            hit.drift = Some(comp.compare(&hit.path));
                                        }
                                    }

                                    debug!(
                                        hits = hits.len(),
                                        elapsed = ?query_start.elapsed(),
                                        "executed interactive search"
                                    );
                                    let rendered = renderer.render_hits(query.as_ref(), &hits)?;
                                    print!("{rendered}");
                                }
                                Err(err) => {
                                    eprintln!("\x1b[1;31mQuery error:\x1b[0m {err}");
                                }
                            }
                        }
                    }
                }
                Err(ReadlineError::Interrupted) => continue,
                Err(ReadlineError::Eof) => break,
                Err(err) => return Err(err.into()),
            }
        }

        if let Some(path) = &history_path {
            save_history(&mut rl, path);
        }

        Ok(serde_json::json!({ "status": "exited" }))
    }
}

fn save_history(rl: &mut DefaultEditor, path: &Utf8Path) {
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

fn parse_input(input: &str) -> ReplAction {
    if let Some(cmd) = input.strip_prefix(':') {
        let mut parts = cmd.split_whitespace();
        match parts.next().unwrap_or_default() {
            "exit" | "quit" | "q" => return ReplAction::Exit,
            "reload" | "r" => return ReplAction::Reload,
            "help" | "h" | "?" => return ReplAction::Help,
            "history" | "git" | "log" => {
                let rest: String = parts.collect::<Vec<_>>().join(" ");
                if let Ok(query) = SearchQuery::try_from(rest) {
                    return ReplAction::History(query);
                }
                eprintln!("Usage: :history <query>");
                return ReplAction::Continue;
            }
            "history-path" | "history-file" | "hp" => return ReplAction::HistoryPath,
            "limit" => {
                if let Some(n) = parts.next().and_then(|s| s.parse::<usize>().ok())
                    && n > 0
                {
                    return ReplAction::SetLimit(n);
                }
                eprintln!("Usage: :limit <positive integer>");
                return ReplAction::Continue;
            }
            other => {
                eprintln!("Unknown command ':{other}'. Type :help for available commands.");
                return ReplAction::Continue;
            }
        }
    }

    match SearchQuery::try_from(input.to_string()) {
        Ok(query) => ReplAction::Search(query),
        Err(_) => ReplAction::Continue,
    }
}

fn print_help() {
    eprintln!(
        "\
Commands:
  <query>           Search manifests & code (supports kind:Deployment, ns:prod, symbol:foo)
  :history <query>  Search Git commit messages, authors, and touched file paths
  :history-path     Show the active REPL history file path (or if disabled)
  :limit <N>        Set max results shown per query
  :reload           Re-index the repository from disk
  :help             Show this help message
  :exit             Exit the REPL (or press Ctrl+D)"
    );
}
