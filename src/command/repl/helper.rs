// src/command/repl/helper.rs

use rustyline::Context;
use rustyline::Helper;
use rustyline::Highlighter;
use rustyline::Hinter;
use rustyline::Validator;
use rustyline::completion::Completer;
use rustyline::completion::Pair;

use crate::index::RepoIndex;

const REPL_COMMANDS: &[&str] = &[":exit", ":help", ":history", ":history-path", ":limit", ":reload"];

const QUERY_FIELDS: &[&str] = &["kind:", "ns:", "labels:", "annotations:"];

#[derive(Debug, Default, Helper, Highlighter, Hinter, Validator)]
pub(super) struct ReplHelper {
    kinds: Vec<String>,
    namespaces: Vec<String>,
    labels: Vec<String>,
    annotations: Vec<String>,
}

impl ReplHelper {
    pub fn update_from_index(&mut self, index: &RepoIndex) {
        self.kinds = index.kinds().to_vec();
        self.namespaces = index.namespaces().to_vec();
        self.labels = index.labels().to_vec();
        self.annotations = index.annotations().to_vec();
    }
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let before_cursor = &line[..pos];
        let token_start = before_cursor.rfind(char::is_whitespace).map_or(0, |idx| idx + 1);
        let token = &before_cursor[token_start..];

        // 1. Complete `:commands` when typing the first token on the line
        if token_start == 0 && token.starts_with(':') {
            let matches = REPL_COMMANDS
                .iter()
                .filter(|cmd| cmd.starts_with(token))
                .map(|cmd| Pair {
                    display: (*cmd).to_string(),
                    replacement: format!("{cmd} "),
                })
                .collect();
            return Ok((token_start, matches));
        }

        // 2. Complete `kind:` and `ns:` using case-insensitive prefix matching
        for (prefix_key, candidates) in [("kind:", &self.kinds), ("ns:", &self.namespaces)] {
            if let Some(prefix) = token.strip_prefix(prefix_key) {
                let needle = prefix.trim_start_matches('"').to_ascii_lowercase();
                let value_start = token_start + prefix_key.len();
                let matches = candidates
                    .iter()
                    .filter(|item| item.to_ascii_lowercase().starts_with(&needle))
                    .map(|item| Pair {
                        display: item.clone(),
                        replacement: format_query_value(item),
                    })
                    .collect();
                return Ok((value_start, matches));
            }
        }

        // 3. Complete `labels:` and `annotations:` with prefix matches first, then substring matches
        for (prefix_key, candidates) in [("labels:", &self.labels), ("annotations:", &self.annotations)] {
            if let Some(prefix) = token.strip_prefix(prefix_key) {
                let needle = prefix.trim_start_matches('"').to_ascii_lowercase();
                let value_start = token_start + prefix_key.len();
                let matches = complete_key_value_candidates(candidates, &needle);
                return Ok((value_start, matches));
            }
        }

        // 4. Complete field prefixes (`kind:`, `ns:`, `labels:`, `annotations:`)
        let field_matches = QUERY_FIELDS
            .iter()
            .filter(|field| !token.is_empty() && field.starts_with(token))
            .map(|field| Pair {
                display: (*field).to_string(),
                replacement: (*field).to_string(),
            })
            .collect();

        Ok((token_start, field_matches))
    }
}

fn complete_key_value_candidates(candidates: &[String], needle_lower: &str) -> Vec<Pair> {
    let mut prefix_matches = Vec::new();
    let mut substring_matches = Vec::new();

    for item in candidates {
        let lower = item.to_ascii_lowercase();
        let pair = || Pair {
            display: item.clone(),
            replacement: format_query_value(item),
        };

        if lower.starts_with(needle_lower) {
            prefix_matches.push(pair());
        } else if lower.contains(needle_lower) {
            substring_matches.push(pair());
        }
    }

    prefix_matches.extend(substring_matches);
    prefix_matches
}

fn format_query_value(val: &str) -> String {
    let needs_quotes = val.contains([':', ' ', '\t', '"', '(', ')', '[', ']', '{', '}']);
    if needs_quotes {
        let escaped = val.replace('"', "\\\"");
        format!("\"{escaped}\" ")
    } else {
        format!("{val} ")
    }
}
