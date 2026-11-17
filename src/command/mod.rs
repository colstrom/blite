// src/command/mod.rs

pub mod history;
pub mod repl;
pub mod search;

use bon::Builder;
use camino::Utf8PathBuf;
use clap::Parser;
use history::HistoryCommand;
use history::HistoryError;
use history::HistoryPayload;
use miette::Diagnostic;
use repl::ReplCommand;
use repl::ReplError;
use search::SearchCommand;
use search::SearchError;
use search::SearchPayload;
use thiserror::Error;

use crate::render::RenderError;
use crate::render::TerminalRenderer;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum CommandError {
    #[diagnostic(transparent)]
    Search(#[from] SearchError),

    #[diagnostic(transparent)]
    History(#[from] HistoryError),

    #[diagnostic(transparent)]
    Repl(#[from] ReplError),

    #[diagnostic(transparent)]
    Render(#[from] RenderError),

    #[diagnostic(code(blite::command::json))]
    Json(#[from] serde_json::Error),
}

/// Boring Local Information Telling Engine.
///
/// Tells you things you might want to know using information you already have.
#[derive(Debug, Clone, Parser, Builder)]
#[command(about)]
#[command(version)]
pub struct Command {
    /// Emit machine-readable JSON instead of syntax-highlighted terminal output
    #[arg(long, global = true)]
    #[builder(default = false)]
    pub json: bool,

    /// Default repository root when launching interactive mode directly
    #[arg(short, long, default_value = ".")]
    #[builder(default = Utf8PathBuf::from("."))]
    pub repo: Utf8PathBuf,

    /// Optional path to the kitchen repository for drift comparison
    #[arg(short, long, global = true)]
    pub kitchen: Option<Utf8PathBuf>,

    /// Default result limit when launching interactive mode directly
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

    #[command(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Subcommand {
    /// Search manifests, scripts, and docs using lexical BM25 ranking
    Search(SearchCommand),
    /// Search Git commit history, authors, and touched file paths
    History(HistoryCommand),
    /// Launch an interactive search session with a hot in-memory index
    Repl(ReplCommand),
}

impl Command {
    pub fn run(self) -> Result<(), crate::Error> {
        match self.subcommand {
            Some(Subcommand::Search(mut cmd)) => {
                cmd.kitchen = cmd.kitchen.or(self.kitchen);
                if self.json {
                    let value = cmd.run().map_err(CommandError::from)?;
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    let payload: SearchPayload = cmd.execute().map_err(CommandError::from)?;
                    let rendered = TerminalRenderer::new()
                        .render_hits(payload.query.as_ref(), &payload.hits)
                        .map_err(CommandError::from)?;
                    print!("{rendered}");
                }
            }
            Some(Subcommand::History(cmd)) => {
                if self.json {
                    let value = cmd.run().map_err(CommandError::from)?;
                    println!("{}", serde_json::to_string_pretty(&value)?);
                } else {
                    let payload: HistoryPayload = cmd.execute().map_err(CommandError::from)?;
                    let rendered = TerminalRenderer::new().render_commit_hits(payload.query.as_ref(), &payload.hits);
                    print!("{rendered}");
                }
            }
            Some(Subcommand::Repl(mut cmd)) => {
                cmd.kitchen = cmd.kitchen.or(self.kitchen);
                cmd.run().map_err(CommandError::from)?;
            }
            None => {
                ReplCommand::builder()
                    .repo(self.repo)
                    .maybe_kitchen(self.kitchen)
                    .limit(self.limit)
                    .maybe_history_file(self.history_file)
                    .repo_history(self.repo_history)
                    .no_history(self.no_history)
                    .build()
                    .run()
                    .map_err(CommandError::from)?;
            }
        }

        Ok(())
    }
}
