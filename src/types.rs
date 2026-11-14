use camino::Utf8PathBuf;
use nutype::nutype;
use serde::Deserialize;
use serde::Serialize;

/// A validated, non-empty search query string.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Deref, Display, FromStr, TryFrom, Serialize, Deserialize)
)]
pub struct SearchQuery(String);

/// A UTF-8 path normalized relative to the indexed repository root.
#[nutype(derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Deref, Display, From, Serialize, Deserialize))]
pub struct RepoRelativePath(Utf8PathBuf);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Deref, Display, TryFrom, Serialize, Deserialize)
)]
pub struct K8sKind(String);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Deref, Display, TryFrom, Serialize, Deserialize)
)]
pub struct K8sName(String);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Deref, Display, TryFrom, Serialize, Deserialize)
)]
pub struct K8sNamespace(String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct K8sMetadata {
    pub kind: K8sKind,
    pub name: Option<K8sName>,
    pub namespace: Option<K8sNamespace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VarResolutionSource {
    /// Resolved from ConfigMap `flux-system/substitutions`
    FluxSubstitutions,
    /// Resolved from another ConfigMap in the repository
    OtherConfigMap { namespace: Option<String>, name: String },
    /// Variable was absent/empty in ConfigMaps, resolved via inline `${VAR:-default}`
    InlineDefault,
    /// Variable referenced in manifest but not found in any ConfigMap or default
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedVar {
    /// Raw expression as written in the manifest (e.g. `${CLUSTER_DOMAIN:-example.com}`)
    pub expression: String,
    /// Extracted variable name (e.g. `CLUSTER_DOMAIN`)
    pub name: String,
    /// Resolved value, if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub source: VarResolutionSource,
}
