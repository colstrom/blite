// src/command/repl/mod.rs

mod helper;
mod history;
pub mod session;

use std::time::Instant;

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use helper::ReplHelper;
use history::HistoryTarget;
use history::load_history;
use history::save_history;
use miette::Diagnostic;
use rustyline::CompletionType;
use rustyline::Config;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use session::ReplSession;
use thiserror::Error;

use crate::command::args::IndexFilterArgs;
use crate::command::args::ReplHistoryArgs;
use crate::command::args::RepoTargetArgs;
use crate::command::args::SemanticArgs;
use crate::index::IndexError;
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
    #[command(flatten)]
    #[builder(default)]
    pub target: RepoTargetArgs,

    /// Optional path to the kitchen repository for drift comparison
    #[arg(short, long)]
    pub kitchen: Option<Utf8PathBuf>,

    /// Maximum number of matching documents to return per query
    #[arg(short = 'n', long, default_value_t = 5)]
    #[builder(default = 5)]
    pub limit: usize,

    #[command(flatten)]
    #[builder(default)]
    pub history: ReplHistoryArgs,

    #[command(flatten)]
    #[builder(default)]
    pub semantic: SemanticArgs,

    #[command(flatten)]
    #[builder(default)]
    pub filter: IndexFilterArgs,
}

impl Default for ReplCommand {
    fn default() -> Self {
        Self::builder().build()
    }
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
    pub fn run(self) -> Result<serde_json::Value, ReplError> {
        let start = Instant::now();
        let build_opts = self.semantic.to_build_options(&self.filter, true);
        let mut session = ReplSession::new(self.target.repo.clone(), self.kitchen.as_deref(), self.limit, build_opts)?;
        let renderer = TerminalRenderer::new();

        let rl_config = Config::builder()
            .max_history_size(1_000)?
            .history_ignore_dups(true)?
            .history_ignore_space(true)
            .completion_type(CompletionType::List)
            .build();
        let mut rl: Editor<ReplHelper, DefaultHistory> = Editor::with_config(rl_config)?;
        let mut helper = ReplHelper::default();
        helper.update_from_index(session.index());
        rl.set_helper(Some(helper));

        let history_path = HistoryTarget::from_args(&self.history, &self.target.repo).resolve_path();
        if let Some(path) = &history_path {
            load_history(&mut rl, path);
        }

        let history_status = history_path.as_deref().map_or("disabled", |p| p.as_str());
        let semantic_status = match session.index().active_semantic_info() {
            Some((model, device)) => format!("{model} on {device}"),
            None => "lexical only".to_string(),
        };

        eprintln!(
            "\x1b[1;32mIndexed {}\x1b[0m in {:.0?} \x1b[2m(search: {}, history: {})\x1b[0m\nType :help for commands, :exit to quit",
            self.target.repo,
            start.elapsed(),
            semantic_status,
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
                            session.set_limit(new_limit);
                            eprintln!("Result limit set to {new_limit}.");
                        }
                        ReplAction::Reload => {
                            let reload_start = Instant::now();
                            match session.reload() {
                                Ok(()) => {
                                    if let Some(helper) = rl.helper_mut() {
                                        helper.update_from_index(session.index());
                                    }
                                    eprintln!("Reloaded index in {:.0?}.", reload_start.elapsed());
                                }
                                Err(err) => {
                                    eprintln!("\x1b[1;31mReload failed:\x1b[0m {err}");
                                }
                            }
                        }
                        ReplAction::History(query) => match session.search_history(&query) {
                            Ok(hits) => {
                                let rendered = renderer.render_commit_hits(query.as_ref(), &hits);
                                print!("{rendered}");
                            }
                            Err(err) => {
                                eprintln!("\x1b[1;31mHistory error:\x1b[0m {err}");
                            }
                        },
                        ReplAction::HistoryPath => match &history_path {
                            Some(path) => eprintln!("REPL history: {path}"),
                            None => eprintln!("REPL history: disabled"),
                        },
                        ReplAction::Search(query) => match session.search(&query) {
                            Ok(hits) => {
                                let rendered = renderer.render_hits(query.as_ref(), &hits)?;
                                print!("{rendered}");
                            }
                            Err(err) => {
                                eprintln!("\x1b[1;31mQuery error:\x1b[0m {err}");
                            }
                        },
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
  <query>           Search manifests & code (supports kind:, ns:, labels:, annotations:, symbol:)
  :history <query>  Search Git commit messages, authors, and touched file paths
  :history-path     Show the active REPL history file path (or if disabled)
  :limit <N>        Set max results shown per query
  :reload           Re-index the repository from disk
  :help             Show this help message
  :exit             Exit the REPL (or press Ctrl+D)"
    );
}
