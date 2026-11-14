// src/types.rs

use camino::Utf8PathBuf;
use nutype::nutype;

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
