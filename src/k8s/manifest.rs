// src/k8s/manifest.rs

use std::collections::BTreeMap;

use camino::Utf8Path;
use serde::Deserialize;

use crate::types::K8sKind;
use crate::types::K8sMetadata;
use crate::types::K8sName;
use crate::types::K8sNamespace;
use crate::types::LineRange;

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

    let configmap_data = if kind.as_ref() == "ConfigMap" {
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

#[cfg(test)]
mod tests {
    use camino::Utf8Path;

    use super::parse_k8s_yaml;
    use super::parse_k8s_yaml_documents;
    use crate::types::LineRange;

    #[test]
    fn parses_single_k8s_manifest_with_metadata() {
        let yaml = "\
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  namespace: prod
  labels:
    app: web
  annotations:
    example.com/test: \"true\"
spec:
  replicas: 2
";
        let docs = parse_k8s_yaml_documents(Utf8Path::new("apps/web.yaml"), yaml);
        assert_eq!(docs.len(), 1);

        let doc = docs.first().unwrap();
        assert!(!doc.is_multi_doc);
        assert_eq!(doc.lines, None);
        assert_eq!(doc.parsed.metadata.kind.as_ref(), "Deployment");
        assert_eq!(doc.parsed.metadata.name.as_deref(), Some("web"));
        assert_eq!(doc.parsed.metadata.namespace.as_deref(), Some("prod"));
        assert_eq!(doc.parsed.metadata.labels, vec!["app=web"]);
        assert_eq!(doc.parsed.metadata.annotations, vec!["example.com/test=true"]);
    }

    #[test]
    fn splits_multi_doc_stream_with_accurate_line_ranges() {
        let stream = "\
# leading comment before first separator
---
apiVersion: v1
kind: Namespace
metadata:
  name: media
---
# non-k8s document that should be skipped
foo: bar
---
apiVersion: v1
kind: Service
metadata:
  name: jellyfin
  namespace: media
";
        let docs = parse_k8s_yaml_documents(Utf8Path::new("apps/media.yml"), stream);
        assert_eq!(docs.len(), 2);

        let first = docs.first().unwrap();
        assert_eq!(first.parsed.metadata.kind.as_ref(), "Namespace");
        assert_eq!(first.parsed.metadata.name.as_deref(), Some("media"));
        assert_eq!(first.lines, Some(LineRange { start: 2, end: 6 }));
        assert!(first.is_multi_doc);

        let second = docs.get(1).unwrap();
        assert_eq!(second.parsed.metadata.kind.as_ref(), "Service");
        assert_eq!(second.parsed.metadata.name.as_deref(), Some("jellyfin"));
        assert_eq!(second.lines, Some(LineRange { start: 10, end: 15 }));
        assert!(second.is_multi_doc);
    }

    #[test]
    fn merges_configmap_data_and_coerces_scalars() {
        let yaml = "\
apiVersion: v1
kind: ConfigMap
metadata:
  name: settings
  namespace: flux-system
data:
  DOMAIN: example.com
  RETRIES: 3
  ENABLED: true
  EMPTY_VAL: null
stringData:
  OVERRIDE: from-string-data
";
        let parsed = parse_k8s_yaml(Utf8Path::new("cm.yaml"), yaml).expect("valid ConfigMap");
        let data = parsed.configmap_data.expect("populated configmap data");

        assert_eq!(data.get("DOMAIN").map(String::as_str), Some("example.com"));
        assert_eq!(data.get("RETRIES").map(String::as_str), Some("3"));
        assert_eq!(data.get("ENABLED").map(String::as_str), Some("true"));
        assert_eq!(data.get("EMPTY_VAL").map(String::as_str), Some(""));
        assert_eq!(data.get("OVERRIDE").map(String::as_str), Some("from-string-data"));
    }

    #[test]
    fn ignores_non_yaml_extensions() {
        let yaml = "kind: ConfigMap\nmetadata:\n  name: ignored\n";
        assert!(parse_k8s_yaml(Utf8Path::new("cm.json"), yaml).is_none());
        assert!(parse_k8s_yaml_documents(Utf8Path::new("cm.txt"), yaml).is_empty());
    }
}
