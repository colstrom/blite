use std::collections::BTreeMap;

use camino::Utf8Path;
use serde::Deserialize;

use crate::types::K8sKind;
use crate::types::K8sMetadata;
use crate::types::K8sName;
use crate::types::K8sNamespace;

#[derive(Debug, Deserialize)]
struct RawManifest {
    kind: Option<String>,
    metadata: Option<RawObjectMeta>,
}

#[derive(Debug, Deserialize)]
struct RawObjectMeta {
    name: Option<String>,
    namespace: Option<String>,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

/// Attempts to extract Kubernetes resource metadata if `path` is a YAML file
/// containing a `kind` field. Returns `None` for non-YAML or non-K8s YAML files.
pub fn extract_k8s_metadata(path: &Utf8Path, content: &str) -> Option<K8sMetadata> {
    let ext = path.extension()?;
    if !ext.eq_ignore_ascii_case("yaml") && !ext.eq_ignore_ascii_case("yml") {
        return None;
    }

    let raw: RawManifest = serde_saphyr::from_str(content).ok()?;
    let kind = raw.kind.and_then(|k| K8sKind::try_from(k).ok())?;

    let (name, namespace, labels) = match raw.metadata {
        Some(meta) => {
            let name = meta.name.and_then(|n| K8sName::try_from(n).ok());
            let namespace = meta.namespace.and_then(|ns| K8sNamespace::try_from(ns).ok());
            let labels = meta.labels.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            (name, namespace, labels)
        }
        None => (None, None, Vec::new()),
    };

    Some(K8sMetadata {
        kind,
        name,
        namespace,
        labels,
    })
}
