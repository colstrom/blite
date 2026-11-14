// src/error.rs

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
#[error(transparent)]
pub enum Error {
    #[diagnostic(transparent)]
    Command(#[from] crate::command::CommandError),

    #[diagnostic(transparent)]
    Index(#[from] crate::index::IndexError),

    #[diagnostic(transparent)]
    Render(#[from] crate::render::RenderError),

    #[diagnostic(code(blite::io))]
    Io(#[from] std::io::Error),

    #[diagnostic(code(blite::json))]
    Json(#[from] serde_json::Error),
}
