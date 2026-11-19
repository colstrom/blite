// src/hash.rs

const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const PRIME: u64 = 0x100000001b3;

/// Computes a deterministic 64-bit FNV-1a hash over a single byte slice.
pub(crate) fn fnv1a_64(bytes: &[u8]) -> u64 {
    fnv1a_64_parts([bytes])
}

/// Computes a deterministic 64-bit FNV-1a hash across multiple byte slices in sequence.
pub(crate) fn fnv1a_64_parts<'a>(parts: impl IntoIterator<Item = &'a [u8]>) -> u64 {
    let mut hash = OFFSET_BASIS;
    for part in parts {
        for &byte in part {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    hash
}
