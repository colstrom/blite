// src/k8s/mod.rs

mod manifest;
mod substitutions;

pub use manifest::ParsedYaml;
pub use manifest::parse_k8s_yaml_documents;
pub use substitutions::SubstitutionTable;
