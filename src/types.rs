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
