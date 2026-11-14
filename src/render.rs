use miette::Diagnostic;
use syntect::easy::HighlightLines;
use syntect::highlighting::Style;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use syntect::util::as_24_bit_terminal_escaped;
use thiserror::Error;

use crate::index::SearchHit;

#[derive(Debug, Error, Diagnostic)]
pub enum RenderError {
    #[error("syntax highlighting error: {0}")]
    #[diagnostic(code(blite::render::syntect))]
    Syntect(#[from] syntect::Error),
}

pub(crate) struct TerminalRenderer {
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl Default for TerminalRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalRenderer {
    pub(crate) fn new() -> Self {
        Self {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }

    pub(crate) fn render_hits(&self, query: &str, hits: &[SearchHit]) -> Result<String, RenderError> {
        if hits.is_empty() {
            return Ok(format!("No matches found for \"{query}\".\n"));
        }

        let theme = &self.theme_set.themes["base16-ocean.dark"];
        let mut out = String::new();

        for (idx, hit) in hits.iter().enumerate() {
            let rank = idx + 1;
            out.push_str(&format!(
                "\x1b[1;36m[{rank}] {path}\x1b[0m \x1b[2m(score: {score:.2})\x1b[0m\n",
                path = hit.path,
                score = hit.score
            ));

            let syntax = hit
                .path
                .extension()
                .and_then(|ext| self.syntax_set.find_syntax_by_extension(ext))
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

            let mut highlighter = HighlightLines::new(syntax, theme);

            for (line_idx, line) in LinesWithEndings::from(&hit.content).enumerate() {
                let line_num = line_idx + 1;
                let ranges: Vec<(Style, &str)> = highlighter.highlight_line(line, &self.syntax_set)?;
                let escaped = as_24_bit_terminal_escaped(&ranges[..], false);
                out.push_str(&format!("\x1b[2m{line_num:>4} │\x1b[0m {escaped}"));
            }

            // Ensure trailing newline & reset ANSI styling between documents
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("\x1b[0m\n");
        }

        Ok(out)
    }
}
