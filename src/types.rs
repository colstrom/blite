// src/types.rs

use std::ops::Deref;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use nutype::nutype;
use serde::Deserialize;
use serde::Serialize;

macro_rules! impl_str_deref {
    ($ty:ty) => {
        impl Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_ref()
            }
        }
    };
}

/// A validated, non-empty search query string.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, AsRef, Display, FromStr, TryFrom, Serialize, Deserialize)
)]
pub struct SearchQuery(String);

impl_str_deref!(SearchQuery);

/// A validated, non-empty embedding model identifier.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, TryFrom, Serialize, Deserialize)
)]
pub struct ModelId(String);

impl_str_deref!(ModelId);

/// A UTF-8 path normalized relative to the indexed repository root.
#[nutype(derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, From, Serialize, Deserialize))]
pub struct RepoRelativePath(Utf8PathBuf);

impl Deref for RepoRelativePath {
    type Target = Utf8Path;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, TryFrom, Serialize, Deserialize)
)]
pub struct K8sKind(String);

impl_str_deref!(K8sKind);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, TryFrom, Serialize, Deserialize)
)]
pub struct K8sName(String);

impl_str_deref!(K8sName);

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, TryFrom, Serialize, Deserialize)
)]
pub struct K8sNamespace(String);

impl_str_deref!(K8sNamespace);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct K8sMetadata {
    pub kind: K8sKind,
    pub name: Option<K8sName>,
    pub namespace: Option<K8sNamespace>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<String>,
}

impl K8sMetadata {
    pub fn label_keys(&self) -> impl Iterator<Item = &str> {
        self.labels.iter().filter_map(|entry| entry.split_once('=').map(|(k, _)| k))
    }

    pub fn annotation_keys(&self) -> impl Iterator<Item = &str> {
        self.annotations.iter().filter_map(|entry| entry.split_once('=').map(|(k, _)| k))
    }
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

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, TryFrom, Serialize, Deserialize)
)]
pub struct SymbolName(String);

impl_str_deref!(SymbolName);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    /// 1-based inclusive starting line number in the source file
    pub start: usize,
    /// 1-based inclusive ending line number in the source file
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftStatus {
    /// Identical to the corresponding file in the kitchen-sink repository
    InSync,
    /// Exists in the kitchen-sink repository with differences
    Drifted,
    /// Exists only in the target cluster repository (not present in kitchen)
    ClusterOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftReport {
    pub status: DriftStatus,
    /// Unified diff from kitchen (old) -> cluster (new), populated when `status == DriftStatus::Drifted`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unified_diff: Option<String>,
}

#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, TryFrom, Serialize, Deserialize)
)]
pub struct CommitOid(String);

impl_str_deref!(CommitOid);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitHit {
    pub oid: CommitOid,
    pub short_oid: String,
    pub author: String,
    pub timestamp: i64,
    pub message: String,
    pub touched_paths: Vec<RepoRelativePath>,
    pub score: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCommit {
    pub oid: CommitOid,
    pub short_oid: String,
    pub committed_at: String,
    #[serde(skip)]
    pub timestamp: i64,
    pub author: String,
    pub subject: String,
}
