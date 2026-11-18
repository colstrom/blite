// src/command/mod.rs

pub mod args;
pub mod diagnostics;
pub mod embeddings;
pub mod history;
pub mod models;
pub mod repl;
pub mod search;

use bon::Builder;
use clap::Parser;
use diagnostics::DiagnosticsCommand;
use diagnostics::DiagnosticsError;
use embeddings::EmbeddingsCommand;
use embeddings::EmbeddingsError;
use history::HistoryCommand;
use history::HistoryError;
use miette::Diagnostic;
use models::ModelsCommand;
use models::ModelsError;
use repl::ReplCommand;
use repl::ReplError;
use search::SearchCommand;
use search::SearchError;
use thiserror::Error;

use crate::render::RenderError;
use crate::render::TerminalRenderer;
use crate::render::render_diagnostics;
use crate::render::render_models_table;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum CommandError {
    #[diagnostic(transparent)]
    Diagnostics(#[from] DiagnosticsError),

    #[diagnostic(transparent)]
    Search(#[from] SearchError),

    #[diagnostic(transparent)]
    History(#[from] HistoryError),

    #[diagnostic(transparent)]
    Models(#[from] ModelsError),

    #[diagnostic(transparent)]
    Embeddings(#[from] EmbeddingsError),

    #[diagnostic(transparent)]
    Repl(#[from] ReplError),

    #[diagnostic(transparent)]
    Render(#[from] RenderError),

    #[diagnostic(code(blite::command::json))]
    Json(#[from] serde_json::Error),
}

enum CommandOutput {
    Json(serde_json::Value),
    Text(String),
    Silent,
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

    #[command(flatten)]
    #[builder(default)]
    pub repl: ReplCommand,

    #[command(subcommand)]
    subcommand: Option<Subcommand>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Subcommand {
    /// Search manifests, scripts, and docs using lexical BM25 and optional semantic ranking
    Search(SearchCommand),
    /// Search Git commit history, authors, and touched file paths
    History(HistoryCommand),
    /// Manage local embedding models and HuggingFace cache consent
    Models(ModelsCommand),
    /// Build, prune, or clear cached embedding vectors
    Embeddings(EmbeddingsCommand),
    /// Inspect repository chunk sizes, token counts, truncation, and batch padding
    Diagnostics(DiagnosticsCommand),
    /// Launch an interactive search session with a hot in-memory index
    Repl(ReplCommand),
}

impl Command {
    pub fn run(self) -> Result<(), crate::Error> {
        let output = self.execute().map_err(CommandError::from)?;
        match output {
            CommandOutput::Json(value) => println!("{}", serde_json::to_string_pretty(&value)?),
            CommandOutput::Text(rendered) => print!("{rendered}"),
            CommandOutput::Silent => {}
        }
        Ok(())
    }

    fn execute(self) -> Result<CommandOutput, CommandError> {
        let json = self.json;
        match self.subcommand {
            Some(Subcommand::Search(cmd)) => {
                if json {
                    Ok(CommandOutput::Json(cmd.run()?))
                } else {
                    let payload = cmd.execute()?;
                    let rendered = TerminalRenderer::new().render_hits(payload.query.as_ref(), &payload.hits)?;
                    Ok(CommandOutput::Text(rendered))
                }
            }
            Some(Subcommand::History(cmd)) => {
                if json {
                    Ok(CommandOutput::Json(cmd.run()?))
                } else {
                    let payload = cmd.execute()?;
                    let rendered = TerminalRenderer::new().render_commit_hits(payload.query.as_ref(), &payload.hits);
                    Ok(CommandOutput::Text(rendered))
                }
            }
            Some(Subcommand::Models(cmd)) => {
                if json {
                    Ok(CommandOutput::Json(cmd.run()?))
                } else {
                    let payload = cmd.execute()?;
                    Ok(CommandOutput::Text(render_models_table(&payload)))
                }
            }
            Some(Subcommand::Embeddings(cmd)) => {
                let value = cmd.run()?;
                if json {
                    Ok(CommandOutput::Json(value))
                } else {
                    Ok(CommandOutput::Silent)
                }
            }
            Some(Subcommand::Diagnostics(cmd)) => {
                if json {
                    Ok(CommandOutput::Json(cmd.run()?))
                } else {
                    let payload = cmd.execute()?;
                    Ok(CommandOutput::Text(render_diagnostics(&payload)))
                }
            }
            Some(Subcommand::Repl(cmd)) => {
                cmd.run()?;
                Ok(CommandOutput::Silent)
            }
            None => {
                self.repl.run()?;
                Ok(CommandOutput::Silent)
            }
        }
    }
}
