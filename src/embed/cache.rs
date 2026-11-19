// src/embed/cache.rs

use std::collections::HashMap;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use tracing::debug;

use super::Embedder;
use super::truncate_for_tokenizer;
use crate::hash::fnv1a_64_parts;

const CACHE_MAGIC: &[u8; 8] = b"BLITEEMB";
const CACHE_VERSION: u32 = 1;

pub(super) fn hash_embed_text(prefix: &str, text: &str) -> u64 {
    fnv1a_64_parts([prefix.as_bytes(), b"\0", text.as_bytes()])
}

pub fn embedding_cache_dir() -> Option<Utf8PathBuf> {
    let base = dirs::cache_dir()?;
    Utf8PathBuf::try_from(base).ok().map(|dir| dir.join("blite").join("embeddings"))
}

pub fn embedding_cache_path(model_label: &str) -> Option<Utf8PathBuf> {
    let cache_dir = embedding_cache_dir()?;
    let safe_name: String = model_label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    Some(cache_dir.join(format!("{safe_name}.bin")))
}

pub fn hash_doc_with_prefix(prefix: &str, text: &str, max_tokens: usize) -> u64 {
    hash_embed_text(prefix, truncate_for_tokenizer(text, max_tokens))
}

pub fn compute_doc_embed_hash(embedder: &Embedder, text: &str) -> u64 {
    hash_doc_with_prefix(embedder.doc_prefix(), text, embedder.max_tokens())
}

pub fn read_embedding_cache(path: &Utf8Path) -> HashMap<u64, Vec<f32>> {
    let Ok(bytes) = fs_err::read(path) else {
        return HashMap::new();
    };
    if bytes.len() < 16 || &bytes[0..8] != CACHE_MAGIC {
        return HashMap::new();
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap_or_default());
    if version != CACHE_VERSION {
        return HashMap::new();
    }
    let dims = u32::from_le_bytes(bytes[12..16].try_into().unwrap_or_default()) as usize;
    if dims == 0 {
        return HashMap::new();
    }

    let record_bytes = 8 + dims * 4;
    let payload = &bytes[16..];
    let complete_records = payload.len() / record_bytes;
    let mut map = HashMap::with_capacity(complete_records);

    for chunk in payload.chunks_exact(record_bytes) {
        let hash = u64::from_le_bytes(chunk[0..8].try_into().unwrap_or_default());
        let mut vec = Vec::with_capacity(dims);
        for f_bytes in chunk[8..].chunks_exact(4) {
            vec.push(f32::from_le_bytes(f_bytes.try_into().unwrap_or_default()));
        }
        map.insert(hash, vec);
    }

    map
}

pub fn write_embedding_cache(path: &Utf8Path, entries: &HashMap<u64, Vec<f32>>) {
    let Some(first_vec) = entries.values().next() else {
        return;
    };
    let dims = first_vec.len() as u32;
    if dims == 0 {
        return;
    }

    if let Some(parent) = path.parent()
        && let Err(err) = fs_err::create_dir_all(parent)
    {
        debug!(path = %parent, error = %err, "failed to create embedding cache directory");
        return;
    }

    let mut buf = Vec::with_capacity(16 + entries.len() * (8 + first_vec.len() * 4));
    buf.extend_from_slice(CACHE_MAGIC);
    buf.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    buf.extend_from_slice(&dims.to_le_bytes());

    for (&hash, vec) in entries {
        if vec.len() != first_vec.len() {
            continue;
        }
        buf.extend_from_slice(&hash.to_le_bytes());
        for &val in vec {
            buf.extend_from_slice(&val.to_le_bytes());
        }
    }

    if let Err(err) = fs_err::write(path, &buf) {
        debug!(path = %path, error = %err, "failed to write embedding cache");
    }
}
