// src/render/hits.rs

use jiff::Timestamp;
use syntect::easy::HighlightLines;
use syntect::highlighting::Style;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use syntect::util::as_24_bit_terminal_escaped;

use super::RenderError;
use crate::index::SearchHit;
use crate::types::DriftStatus;
use crate::types::VarResolutionSource;

pub(super) fn render_search_hits(
    syntax_set: &SyntaxSet,
    theme_set: &ThemeSet,
    query: &str,
    hits: &[SearchHit],
) -> Result<String, RenderError> {
    if hits.is_empty() {
        return Ok(format!("No matches found for \"{query}\".\n"));
    }

    let theme = &theme_set.themes["base16-ocean.dark"];
    let mut out = String::new();

    for (idx, hit) in hits.iter().enumerate() {
        let rank = idx + 1;
        let location = match hit.lines {
            Some(range) => format!("{}:{}..{}", hit.path, range.start, range.end),
            None => hit.path.to_string(),
        };

        let badge = match (&hit.k8s, &hit.symbol) {
            (Some(k8s), _) => {
                let name = k8s.name.as_deref().unwrap_or("<unnamed>");
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
            .and_then(|ext| syntax_set.find_syntax_by_extension(ext))
            .unwrap_or_else(|| syntax_set.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, theme);
        let base_line = hit.lines.map_or(1, |r| r.start);

        for (line_idx, line) in LinesWithEndings::from(&hit.content).enumerate() {
            let line_num = base_line + line_idx;
            let ranges: Vec<(Style, &str)> = highlighter.highlight_line(line, syntax_set)?;
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

        if let Some(latest) = hit.recent_commits.first() {
            let relative = format_relative_age(latest.timestamp);
            out.push_str(&format!(
                "  \x1b[2m↳ Last commit:\x1b[0m \x1b[33m{}\x1b[0m {} \x1b[2m({}, {}, {})\x1b[0m\n",
                latest.short_oid, latest.subject, latest.author, latest.committed_at, relative
            ));
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

fn format_relative_age(unix_seconds: i64) -> String {
    format_relative_age_from(Timestamp::now(), unix_seconds)
}

fn format_relative_age_from(now: Timestamp, unix_seconds: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;

    let Ok(then) = Timestamp::from_second(unix_seconds) else {
        return "unknown".to_string();
    };

    let elapsed = now.as_second().saturating_sub(then.as_second());
    if elapsed < MINUTE {
        "just now".to_string()
    } else if elapsed < HOUR {
        format!("{}m ago", elapsed / MINUTE)
    } else if elapsed < DAY {
        format!("{}h ago", elapsed / HOUR)
    } else if elapsed < MONTH {
        format!("{}d ago", elapsed / DAY)
    } else if elapsed < YEAR {
        format!("{}mo ago", elapsed / MONTH)
    } else {
        format!("{}y ago", elapsed / YEAR)
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::format_relative_age_from;

    #[test]
    fn formats_relative_ages_across_units() {
        let now = Timestamp::from_second(1_700_000_000).unwrap();
        let base = now.as_second();

        assert_eq!(format_relative_age_from(now, base + 300), "just now");
        assert_eq!(format_relative_age_from(now, base - 45), "just now");
        assert_eq!(format_relative_age_from(now, base - 60), "1m ago");
        assert_eq!(format_relative_age_from(now, base - 45 * 60), "45m ago");
        assert_eq!(format_relative_age_from(now, base - 3 * 3_600), "3h ago");
        assert_eq!(format_relative_age_from(now, base - 12 * 86_400), "12d ago");
        assert_eq!(format_relative_age_from(now, base - 75 * 86_400), "2mo ago");
        assert_eq!(format_relative_age_from(now, base - 400 * 86_400), "1y ago");
    }
}
