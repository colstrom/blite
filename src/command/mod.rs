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
use serde::Serialize;
use thiserror::Error;

use crate::render::RenderError;

#[doc(hidden)]
pub trait ExecuteCommand: Sized {
    type Output;
    type Error: std::error::Error;

    fn execute(self) -> Result<Self::Output, Self::Error>;

    /// Formats `Output` for terminal display. Return `Ok(None)` if `execute` already handled output.
    fn render(output: Self::Output) -> Result<Option<String>, Self::Error> {
        let _ = output;
        Ok(None)
    }
}

pub trait RunCommand {
    type Error: std::error::Error;
    fn run(self) -> Result<(), Self::Error>;
}

impl<T: ExecuteCommand> RunCommand for T {
    type Error = T::Error;

    fn run(self) -> Result<(), Self::Error> {
        let output = self.execute()?;
        if let Some(rendered) = T::render(output)? {
            print!("{rendered}");
        }
        Ok(())
    }
}

pub trait JsonCommand {
    type Error: std::error::Error;
    fn json(self) -> Result<serde_json::Value, Self::Error>;
}

impl<T> JsonCommand for T
where
    T: ExecuteCommand,
    T::Output: Serialize,
    serde_json::Error: Into<T::Error>,
{
    type Error = T::Error;

    fn json(self) -> Result<serde_json::Value, Self::Error> {
        let output = self.execute()?;
        serde_json::to_value(output).map_err(Into::into)
    }
}

macro_rules! runnable {
    ($T:ty) => {
        impl $T {
            pub fn run(self) -> Result<(), <Self as $crate::command::RunCommand>::Error> {
                <Self as $crate::command::RunCommand>::run(self)
            }
        }
    };
}

macro_rules! json_command {
    ($T:ty) => {
        crate::command::runnable!($T);

        impl $T {
            pub fn json(self) -> Result<serde_json::Value, <Self as $crate::command::JsonCommand>::Error> {
                <Self as $crate::command::JsonCommand>::json(self)
            }
        }
    };
}

pub(crate) use json_command;
pub(crate) use runnable;

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
    pub(crate) json: bool,

    #[command(flatten)]
    #[builder(default)]
    pub(crate) repl: ReplCommand,

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

impl RunCommand for Command {
    type Error = crate::Error;

    fn run(self) -> Result<(), crate::Error> {
        run_top_level(self).map_err(Into::into)
    }
}

runnable!(Command);

fn run_top_level(cmd: Command) -> Result<(), CommandError> {
    let as_json = cmd.json;
    match cmd.subcommand {
        Some(Subcommand::Search(sub)) => dispatch_json_or_run(as_json, sub),
        Some(Subcommand::History(sub)) => dispatch_json_or_run(as_json, sub),
        Some(Subcommand::Models(sub)) => dispatch_json_or_run(as_json, sub),
        Some(Subcommand::Embeddings(sub)) => dispatch_json_or_run(as_json, sub),
        Some(Subcommand::Diagnostics(sub)) => dispatch_json_or_run(as_json, sub),
        Some(Subcommand::Repl(sub)) => Ok(sub.run()?),
        None => Ok(cmd.repl.run()?),
    }
}

fn dispatch_json_or_run<T>(as_json: bool, cmd: T) -> Result<(), CommandError>
where
    T: JsonCommand + RunCommand,
    <T as JsonCommand>::Error: Into<CommandError>,
    <T as RunCommand>::Error: Into<CommandError>,
{
    if as_json {
        let value = cmd.json().map_err(Into::into)?;
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        cmd.run().map_err(Into::into)?;
    }
    Ok(())
}
