// src/k8s.rs

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::OnceLock;

use camino::Utf8Path;
use regex_lite::Regex;
use serde::Deserialize;

use crate::types::K8sKind;
use crate::types::K8sMetadata;
use crate::types::K8sName;
use crate::types::K8sNamespace;
use crate::types::LineRange;
use crate::types::ResolvedVar;
use crate::types::VarResolutionSource;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawManifest {
    kind: Option<String>,
    metadata: Option<RawObjectMeta>,
    #[serde(default)]
    data: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    string_data: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RawObjectMeta {
    name: Option<String>,
    namespace: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ParsedYaml {
    pub metadata: K8sMetadata,
    pub configmap_data: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct ParsedYamlDocument {
    pub parsed: ParsedYaml,
    pub content: String,
    pub lines: Option<LineRange>,
    pub doc_index: usize,
    pub is_multi_doc: bool,
}

/// Splits a YAML stream on top-level `---` document markers and parses each
/// Kubernetes resource along with its 1-based line range.
pub fn parse_k8s_yaml_documents(path: &Utf8Path, content: &str) -> Vec<ParsedYamlDocument> {
    let Some(ext) = path.extension() else {
        return Vec::new();
    };
    if !ext.eq_ignore_ascii_case("yaml") && !ext.eq_ignore_ascii_case("yml") {
        return Vec::new();
    }

    let raw_lines: Vec<&str> = content.lines().collect();
    if raw_lines.is_empty() {
        return Vec::new();
    }

    // Find 0-based line indices where a top-level `---` separator begins a document
    let mut doc_starts: Vec<usize> = Vec::new();
    for (idx, line) in raw_lines.iter().enumerate() {
        let trimmed = line.trim_end();
        if trimmed == "---" || trimmed.starts_with("--- ") {
            doc_starts.push(idx);
        }
    }

    // Single-document YAML (no `---` or only a single leading `---`)
    if doc_starts.len() <= 1 {
        return match parse_k8s_yaml(path, content) {
            Some(parsed) => vec![ParsedYamlDocument {
                parsed,
                content: content.to_string(),
                lines: None,
                doc_index: 0,
                is_multi_doc: false,
            }],
            None => Vec::new(),
        };
    }

    // Build `(start_row, end_row_inclusive)` spans for each document in the stream
    let mut spans: Vec<(usize, usize)> = Vec::new();
    if doc_starts[0] > 0 {
        let first_end = doc_starts[0] - 1;
        if raw_lines[..=first_end]
            .iter()
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        {
            spans.push((0, first_end));
        }
    }

    for (i, &start) in doc_starts.iter().enumerate() {
        let end = if i + 1 < doc_starts.len() {
            doc_starts[i + 1] - 1
        } else {
            raw_lines.len() - 1
        };
        if start <= end {
            spans.push((start, end));
        }
    }

    let is_multi = spans.len() > 1;
    let mut results = Vec::with_capacity(spans.len());

    for (doc_idx, (start_row, end_row)) in spans.into_iter().enumerate() {
        let slice = raw_lines[start_row..=end_row].join("\n");
        if let Some(parsed) = parse_k8s_yaml(path, &slice) {
            results.push(ParsedYamlDocument {
                parsed,
                content: slice,
                lines: if is_multi {
                    Some(LineRange {
                        start: start_row + 1,
                        end: end_row + 1,
                    })
                } else {
                    None
                },
                doc_index: doc_idx,
                is_multi_doc: is_multi,
            });
        }
    }

    results
}

fn json_scalar_to_string(val: serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => Some(String::new()),
        _ => None,
    }
}

/// Extracts Kubernetes resource metadata and, if `kind == "ConfigMap"`, its `data` + `stringData`.
pub fn parse_k8s_yaml(path: &Utf8Path, content: &str) -> Option<ParsedYaml> {
    let ext = path.extension()?;
    if !ext.eq_ignore_ascii_case("yaml") && !ext.eq_ignore_ascii_case("yml") {
        return None;
    }

    let raw: RawManifest = serde_saphyr::from_str(content).ok()?;
    let kind = raw.kind.and_then(|k| K8sKind::try_from(k).ok())?;

    let (name, namespace, labels, annotations) = match raw.metadata {
        Some(meta) => {
            let name = meta.name.and_then(|n| K8sName::try_from(n).ok());
            let namespace = meta.namespace.and_then(|ns| K8sNamespace::try_from(ns).ok());
            let labels = meta.labels.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            let annotations = meta.annotations.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            (name, namespace, labels, annotations)
        }
        None => (None, None, Vec::new(), Vec::new()),
    };

    let configmap_data = if kind.as_str() == "ConfigMap" {
        let merged: BTreeMap<String, String> = raw
            .data
            .into_iter()
            .chain(raw.string_data)
            .filter_map(|(k, v)| json_scalar_to_string(v).map(|s| (k, s)))
            .collect();

        if merged.is_empty() { None } else { Some(merged) }
    } else {
        None
    };

    Some(ParsedYaml {
        metadata: K8sMetadata {
            kind,
            name,
            namespace,
            labels,
            annotations,
        },
        configmap_data,
    })
}

/// Holds substitution variables discovered across repository ConfigMaps,
/// prioritizing Flux's `flux-system/substitutions` ConfigMap.
#[derive(Debug, Clone, Default)]
pub struct SubstitutionTable {
    flux_vars: BTreeMap<String, String>,
    fallback_vars: BTreeMap<String, (String, Option<String>, String)>, // key -> (val, ns, cm_name)
}

impl SubstitutionTable {
    pub fn insert_configmap(&mut self, path: &Utf8Path, meta: &K8sMetadata, data: BTreeMap<String, String>) {
        let ns = meta.namespace.as_ref().map(|s| s.as_str());
        let name = meta.name.as_ref().map(|s| s.as_str());

        let is_flux_substitutions = (ns == Some("flux-system") && name == Some("substitutions"))
            || path.ends_with("flux-system/resources/ConfigMap/substitutions.yaml");

        if is_flux_substitutions {
            self.flux_vars.extend(data);
        } else {
            let cm_name = name.unwrap_or(path.as_str()).to_string();
            let ns_owned = ns.map(ToString::to_string);
            for (k, v) in data {
                self.fallback_vars
                    .entry(k)
                    .or_insert_with(|| (v, ns_owned.clone(), cm_name.clone()));
            }
        }
    }

    pub fn flux_var_count(&self) -> usize {
        self.flux_vars.len()
    }

    /// Scans `content` for `drone/envsubst`-style `${VAR}` / `${VAR:-default}` expressions
    /// and resolves them in order of appearance (deduplicated by raw expression).
    pub fn resolve_in_content(&self, content: &str) -> Vec<ResolvedVar> {
        static ENVSUBST_RE: OnceLock<Regex> = OnceLock::new();
        let re = ENVSUBST_RE.get_or_init(|| {
            Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?:(:?=|:=|:?-|-|:?\+|\+|:\?|\?)([^}]*))?\}").expect("valid envsubst regex")
        });

        let mut seen = HashSet::new();
        let mut resolved = Vec::new();

        for caps in re.captures_iter(content) {
            let full_expr = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
            if !seen.insert(full_expr.to_string()) {
                continue;
            }

            let var_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let op = caps.get(2).map(|m| m.as_str());
            let operand = caps.get(3).map(|m| m.as_str());

            resolved.push(self.resolve_one(full_expr, var_name, op, operand));
        }

        resolved
    }

    fn resolve_one(&self, expression: &str, name: &str, op: Option<&str>, operand: Option<&str>) -> ResolvedVar {
        let lookup = self
            .flux_vars
            .get(name)
            .map(|v| (v.as_str(), VarResolutionSource::FluxSubstitutions))
            .or_else(|| {
                self.fallback_vars.get(name).map(|(v, ns, cm_name)| {
                    (
                        v.as_str(),
                        VarResolutionSource::OtherConfigMap {
                            namespace: ns.clone(),
                            name: cm_name.clone(),
                        },
                    )
                })
            });

        let is_set = lookup.is_some();
        let is_non_empty = lookup.as_ref().is_some_and(|(v, _)| !v.is_empty());

        match (op, operand) {
            (Some(":+"), Some(alt)) if is_non_empty => ResolvedVar {
                expression: expression.to_string(),
                name: name.to_string(),
                value: Some(alt.to_string()),
                source: lookup.unwrap().1,
            },
            (Some("+"), Some(alt)) if is_set => ResolvedVar {
                expression: expression.to_string(),
                name: name.to_string(),
                value: Some(alt.to_string()),
                source: lookup.unwrap().1,
            },
            (Some(":-" | ":="), Some(default_val)) if !is_non_empty => ResolvedVar {
                expression: expression.to_string(),
                name: name.to_string(),
                value: Some(default_val.to_string()),
                source: VarResolutionSource::InlineDefault,
            },
            (Some("-" | "="), Some(default_val)) if !is_set => ResolvedVar {
                expression: expression.to_string(),
                name: name.to_string(),
                value: Some(default_val.to_string()),
                source: VarResolutionSource::InlineDefault,
            },
            _ => match lookup {
                Some((val, source)) => ResolvedVar {
                    expression: expression.to_string(),
                    name: name.to_string(),
                    value: Some(val.to_string()),
                    source,
                },
                None => ResolvedVar {
                    expression: expression.to_string(),
                    name: name.to_string(),
                    value: None,
                    source: VarResolutionSource::Unresolved,
                },
            },
        }
    }
}
