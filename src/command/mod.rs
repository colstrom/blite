// src/command/mod.rs

pub mod search;

use bon::Builder;
use clap::Parser;
use miette::Diagnostic;
use search::SearchCommand;
use search::SearchError;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum CommandError {
    #[diagnostic(transparent)]
    Search(#[from] SearchError),

    #[diagnostic(code(blite::command::json))]
    Json(#[from] serde_json::Error),
}

/// Boring Local Information Telling Engine.
///
/// Tells you things you might want to know using information you already have.
#[derive(Debug, Clone, Parser, Builder)]
#[command(version)]
#[command(about)]
pub struct Command {
    #[command(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Subcommand {
    Search(SearchCommand),
}

impl Command {
    pub fn run(self) -> Result<(), crate::Error> {
        let value: serde_json::Value = match self.subcommand {
            Subcommand::Search(cmd) => cmd.run().map_err(CommandError::from)?,
        };

        println!("{}", serde_json::to_string_pretty(&value)?);
        Ok(())
    }
}
