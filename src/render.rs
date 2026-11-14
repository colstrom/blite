use miette::Diagnostic;
use syntect::easy::HighlightLines;
use syntect::highlighting::Style;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use syntect::util::as_24_bit_terminal_escaped;
use thiserror::Error;

use crate::index::SearchHit;
use crate::types::DriftStatus;
use crate::types::VarResolutionSource;

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
        if hits.is_empty() {
            return Ok(format!("No matches found for \"{query}\".\n"));
        }

        let theme = &self.theme_set.themes["base16-ocean.dark"];
        let mut out = String::new();

        for (idx, hit) in hits.iter().enumerate() {
            let rank = idx + 1;
            let location = match hit.lines {
                Some(range) => format!("{}:{}..{}", hit.path, range.start, range.end),
                None => hit.path.to_string(),
            };

            let badge = match (&hit.k8s, &hit.symbol) {
                (Some(k8s), _) => {
                    let name = k8s.name.as_ref().map_or("<unnamed>", |n| n.as_str());
                    match &k8s.namespace {
                        Some(ns) => format!(" \x1b[1;33m[{}/{name} @ {ns}]\x1b[0m", k8s.kind),
                        None => format!(" \x1b[1;33m[{}/{name}]\x1b[0m", k8s.kind),
                    }
                }
                (None, Some(sym)) => format!(" \x1b[1;35m[fn/def {sym}]\x1b[0m"),
                (None, None) => String::new(),
            };

            let drift_badge = match hit.drift.as_ref().map(|d| &d.status) {
                Some(DriftStatus::InSync) => " \x1b[1;32m[in sync]\x1b[0m",
                Some(DriftStatus::Drifted) => " \x1b[1;31m[drifted]\x1b[0m",
                Some(DriftStatus::ClusterOnly) => " \x1b[1;34m[cluster-only]\x1b[0m",
                None => "",
            };

            out.push_str(&format!(
                "\x1b[1;36m[{rank}] {location}\x1b[0m{badge}{drift_badge} \x1b[2m(score: {score:.2})\x1b[0m\n",
                score = hit.score
            ));

            let syntax = hit
                .path
                .extension()
                .and_then(|ext| self.syntax_set.find_syntax_by_extension(ext))
                .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

            let mut highlighter = HighlightLines::new(syntax, theme);
            let base_line = hit.lines.map_or(1, |r| r.start);

            for (line_idx, line) in LinesWithEndings::from(&hit.content).enumerate() {
                let line_num = base_line + line_idx;
                let ranges: Vec<(Style, &str)> = highlighter.highlight_line(line, &self.syntax_set)?;
                let escaped = as_24_bit_terminal_escaped(&ranges[..], false);
                out.push_str(&format!("\x1b[2m{line_num:>4} │\x1b[0m {escaped}"));
            }

            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("\x1b[0m");

            if !hit.resolved_vars.is_empty() {
                out.push_str("  \x1b[2m↳ Substitutions:\x1b[0m\n");
                for var in &hit.resolved_vars {
                    match (&var.value, &var.source) {
                        (Some(val), VarResolutionSource::FluxSubstitutions) => {
                            out.push_str(&format!(
                                "    \x1b[36m{}\x1b[0m = \x1b[1;32m{:?}\x1b[0m \x1b[2m(flux-system/substitutions)\x1b[0m\n",
                                var.expression, val
                            ));
                        }
                        (Some(val), VarResolutionSource::OtherConfigMap { namespace, name }) => {
                            let cm_ref = match namespace {
                                Some(ns) => format!("{ns}/{name}"),
                                None => name.clone(),
                            };
                            out.push_str(&format!(
                                "    \x1b[36m{}\x1b[0m = \x1b[32m{:?}\x1b[0m \x1b[2m(ConfigMap {cm_ref})\x1b[0m\n",
                                var.expression, val
                            ));
                        }
                        (Some(val), VarResolutionSource::InlineDefault) => {
                            out.push_str(&format!(
                                "    \x1b[36m{}\x1b[0m = \x1b[33m{:?}\x1b[0m \x1b[2m(inline default)\x1b[0m\n",
                                var.expression, val
                            ));
                        }
                        (_, VarResolutionSource::Unresolved) | (None, _) => {
                            out.push_str(&format!("    \x1b[36m{}\x1b[0m = \x1b[1;31m<unresolved>\x1b[0m\n", var.expression));
                        }
                    }
                }
            }

            if let Some(drift) = &hit.drift
                && let Some(diff) = &drift.unified_diff
            {
                out.push_str("  \x1b[2m↳ Kitchen Drift Diff:\x1b[0m\n");
                for line in diff.lines() {
                    if line.starts_with('+') && !line.starts_with("+++") {
                        out.push_str(&format!("    \x1b[32m{line}\x1b[0m\n"));
                    } else if line.starts_with('-') && !line.starts_with("---") {
                        out.push_str(&format!("    \x1b[31m{line}\x1b[0m\n"));
                    } else if line.starts_with("@@") {
                        out.push_str(&format!("    \x1b[36m{line}\x1b[0m\n"));
                    } else {
                        out.push_str(&format!("    \x1b[2m{line}\x1b[0m\n"));
                    }
                }
            }

            out.push('\n');
        }

        Ok(out)
    }
}
