// src/render/mod.rs

mod commits;
mod diagnostics;
mod hits;
mod models;

pub use diagnostics::render_diagnostics;
use miette::Diagnostic;
pub use models::render_models_table;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use thiserror::Error;

use crate::index::SearchHit;
use crate::types::CommitHit;

#[derive(Debug, Error, Diagnostic)]
pub enum RenderError {
    #[error("syntax highlighting error: {0}")]
    #[diagnostic(code(blite::render::syntect))]
    Syntect(#[from] syntect::Error),
}

pub struct TerminalRenderer {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl Default for TerminalRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalRenderer {
    pub fn new() -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }

    pub fn render_hits(&self, query: &str, hits: &[SearchHit]) -> Result<String, RenderError> {
        hits::render_search_hits(&self.syntax_set, &self.theme_set, query, hits)
    }

    pub fn render_commit_hits(&self, query: &str, hits: &[CommitHit]) -> String {
        commits::render_commit_hits(query, hits)
    }
}
