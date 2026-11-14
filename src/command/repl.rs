use std::time::Instant;

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Args;
use miette::Diagnostic;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use thiserror::Error;
use tracing::debug;
use tracing::info;

use crate::drift::KitchenComparator;
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
}

enum ReplAction {
    Continue,
    Exit,
    Reload,
    SetLimit(usize),
    Help,
    Search(SearchQuery),
}

impl ReplCommand {
    pub fn run(self) -> Result<serde_json::Value, ReplError> {
        let start = Instant::now();
        let mut index = RepoIndex::build_from_dir(&self.repo)?;
        let renderer = TerminalRenderer::new();
        let comparator = self.kitchen.as_ref().map(|k| KitchenComparator::new(&self.repo, k));
        let mut limit = self.limit;

        eprintln!(
            "\x1b[1;32mIndexed {}\x1b[0m in {:.0?} (type :help for commands, :exit to quit)",
            self.repo,
            start.elapsed()
        );

        let mut rl = DefaultEditor::new()?;

        loop {
            match rl.readline("\x1b[1;34mblite>\x1b[0m ") {
                Ok(line) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let _ = rl.add_history_entry(trimmed);

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
                                    info!(elapsed = ?reload_start.elapsed(), "reloaded repository index");
                                    eprintln!("Reloaded index in {:.0?}.", reload_start.elapsed());
                                }
                                Err(err) => {
                                    eprintln!("\x1b[1;31mReload failed:\x1b[0m {err}");
                                }
                            }
                        }
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
  <query>       Search manifests & code (supports kind:Deployment, ns:prod, symbol:foo)
  :limit <N>    Set max results shown per query
  :reload       Re-index the repository from disk
  :help         Show this help message
  :exit         Exit the REPL (or press Ctrl+D)"
    );
}
