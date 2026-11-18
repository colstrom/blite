// src/embed/catalog.rs

use camino::Utf8Path;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub hf_repo: &'static str,
    pub origin: &'static str,
    pub license: &'static str,
    pub params: &'static str,
    pub layers: usize,
    pub dimensions: usize,
    pub max_context: usize,
    pub approx_size: &'static str,
    pub speed_tier: &'static str,
    pub compliance_notes: &'static str,
    pub best_for: &'static str,
    pub query_prefix: &'static str,
    pub doc_prefix: &'static str,
}

pub const MODEL_CATALOG: &[ModelSpec] = &[
    ModelSpec {
        id: "mxbai-xsmall",
        hf_repo: "mixedbread-ai/mxbai-embed-xsmall-v1",
        origin: "DE / EU (Mixedbread)",
        license: "Apache-2.0",
        params: "24M",
        layers: 6,
        dimensions: 384,
        max_context: 512,
        approx_size: "~95 MB",
        speed_tier: "Very Fast (CPU & Metal)",
        compliance_notes: "EU-developed (GDPR jurisdiction), Apache-2.0, open weights.",
        best_for: "Ultra-fast indexing & queries on Linux CPU with EU origin.",
        query_prefix: "Represent this sentence for searching relevant passages: ",
        doc_prefix: "",
    },
    ModelSpec {
        id: "mxbai-large",
        hf_repo: "mixedbread-ai/mxbai-embed-large-v1",
        origin: "DE / EU (Mixedbread)",
        license: "Apache-2.0",
        params: "335M",
        layers: 24,
        dimensions: 1024,
        max_context: 512,
        approx_size: "~1.3 GB",
        speed_tier: "Moderate (Best on Metal)",
        compliance_notes: "EU-developed (GDPR jurisdiction), Apache-2.0, open weights.",
        best_for: "Heavyweight semantic accuracy on Apple Silicon Metal.",
        query_prefix: "Represent this sentence for searching relevant passages: ",
        doc_prefix: "",
    },
    ModelSpec {
        id: "granite-30m",
        hf_repo: "ibm-granite/granite-embedding-30m-english",
        origin: "US (IBM Research)",
        license: "Apache-2.0",
        params: "30M",
        layers: 6,
        dimensions: 384,
        max_context: 512,
        approx_size: "~120 MB",
        speed_tier: "Very Fast (CPU & Metal)",
        compliance_notes: "Enterprise-audited training corpus for IP/licensing provenance; Apache-2.0.",
        best_for: "Strict enterprise compliance environments & fast 6-layer CPU runs.",
        query_prefix: "",
        doc_prefix: "",
    },
    ModelSpec {
        id: "arctic-xs",
        hf_repo: "Snowflake/snowflake-arctic-embed-xs",
        origin: "US (Snowflake)",
        license: "Apache-2.0",
        params: "22M",
        layers: 6,
        dimensions: 384,
        max_context: 512,
        approx_size: "~90 MB",
        speed_tier: "Very Fast (CPU & Metal)",
        compliance_notes: "US-developed, Apache-2.0, open weights.",
        best_for: "Independent US-origin option with minimal disk and CPU footprint.",
        query_prefix: "Represent this sentence for searching relevant passages: ",
        doc_prefix: "",
    },
    ModelSpec {
        id: "nomic-v1.5",
        hf_repo: "nomic-ai/nomic-embed-text-v1.5",
        origin: "US (Nomic AI)",
        license: "Apache-2.0",
        params: "137M",
        layers: 12,
        dimensions: 768,
        max_context: 8192,
        approx_size: "~550 MB",
        speed_tier: "Fast (Metal) / Moderate (CPU)",
        compliance_notes: "Fully open weights, training code, and audited open training data; Apache-2.0.",
        best_for: "Long scripts/READMEs (8K context) & maximum open-source transparency.",
        query_prefix: "search_query: ",
        doc_prefix: "search_document: ",
    },
    ModelSpec {
        id: "bge-small",
        hf_repo: "BAAI/bge-small-en-v1.5",
        origin: "CN (BAAI)",
        license: "MIT",
        params: "33M",
        layers: 12,
        dimensions: 384,
        max_context: 512,
        approx_size: "~130 MB",
        speed_tier: "Fast (CPU & Metal)",
        compliance_notes: "Open weights (MIT); training data not itemized; CN-origin.",
        best_for: "Proven, compact baseline for short manifests and code snippets.",
        query_prefix: "Represent this sentence for searching relevant passages: ",
        doc_prefix: "",
    },
];

pub fn find_model_spec(id_or_repo: &str) -> Option<&'static ModelSpec> {
    let needle = id_or_repo.trim().to_ascii_lowercase();
    MODEL_CATALOG
        .iter()
        .find(|spec| spec.id.eq_ignore_ascii_case(&needle) || spec.hf_repo.eq_ignore_ascii_case(&needle))
}

pub(super) fn infer_prefixes_from_dir(dir: &Utf8Path) -> (String, String) {
    let dir_lower = dir.as_str().to_ascii_lowercase();
    if dir_lower.contains("nomic") {
        return ("search_query: ".to_string(), "search_document: ".to_string());
    }
    if dir_lower.contains("bge") || dir_lower.contains("mxbai") || dir_lower.contains("arctic") {
        return (
            "Represent this sentence for searching relevant passages: ".to_string(),
            String::new(),
        );
    }
    (String::new(), String::new())
}
