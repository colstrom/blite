// src/command/mod.rs
pub mod search;

use bon::Builder;
use clap::Parser;
use miette::Diagnostic;
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

    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Subcommand {
    /// Search manifests, scripts, and docs using lexical BM25 ranking
    Search(SearchCommand),
}

impl Command {
    pub fn run(self) -> Result<(), crate::Error> {
        match self.subcommand {
            Subcommand::Search(cmd) => {
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
        }

        Ok(())
    }
}
