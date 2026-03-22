//! Simple Merkle tree for computing state roots.
//!
//! Uses domain separation per RFC 6962:
//! - Leaf nodes: prefix with `0x00` before hashing
//! - Internal nodes: prefix with `0x01` before hashing

/// Compute the Merkle root of a list of leaf hashes.
///
/// - Empty input → all-zero hash
/// - Single leaf → hashed with `0x00` domain tag
/// - Multiple leaves → binary Merkle tree using blake3 with domain separation
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }

    // Hash leaves with 0x00 domain tag
    let mut current: Vec<[u8; 32]> = leaves.iter()
        .map(|leaf| {
            let mut hasher = blake3::Hasher::new();
            hasher.update(&[0x00]);
            hasher.update(leaf);
            *hasher.finalize().as_bytes()
        })
        .collect();

    while current.len() > 1 {
        let mut next = Vec::new();
        for chunk in current.chunks(2) {
            if chunk.len() == 2 {
                let mut hasher = blake3::Hasher::new();
                hasher.update(&[0x01]);
                hasher.update(&chunk[0]);
                hasher.update(&chunk[1]);
                next.push(*hasher.finalize().as_bytes());
            } else {
                // Odd element: hash it as internal node with itself
                let mut hasher = blake3::Hasher::new();
                hasher.update(&[0x01]);
                hasher.update(&chunk[0]);
                hasher.update(&chunk[0]);
                next.push(*hasher.finalize().as_bytes());
            }
        }
        current = next;
    }
    current[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_leaves_returns_zeros() {
        assert_eq!(merkle_root(&[]), [0u8; 32]);
    }

    #[test]
    fn single_leaf_is_domain_hashed() {
        let leaf = *blake3::hash(b"hello").as_bytes();
        // Single leaf should NOT be returned as-is; it must be domain-hashed
        let root = merkle_root(&[leaf]);
        assert_ne!(root, leaf);

        // Verify it equals the expected domain-tagged hash
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[0x00]);
        hasher.update(&leaf);
        let expected = *hasher.finalize().as_bytes();
        assert_eq!(root, expected);
    }

    #[test]
    fn two_leaves_deterministic() {
        let a = *blake3::hash(b"a").as_bytes();
        let b = *blake3::hash(b"b").as_bytes();
        let root1 = merkle_root(&[a, b]);
        let root2 = merkle_root(&[a, b]);
        assert_eq!(root1, root2);

        // Different order → different root
        let root3 = merkle_root(&[b, a]);
        assert_ne!(root1, root3);
    }

    #[test]
    fn two_leaves_uses_domain_separation() {
        let a = *blake3::hash(b"a").as_bytes();
        let b = *blake3::hash(b"b").as_bytes();
        let root = merkle_root(&[a, b]);

        // Manually compute expected: hash(0x01 || hash(0x00 || a) || hash(0x00 || b))
        let leaf_a = {
            let mut h = blake3::Hasher::new();
            h.update(&[0x00]);
            h.update(&a);
            *h.finalize().as_bytes()
        };
        let leaf_b = {
            let mut h = blake3::Hasher::new();
            h.update(&[0x00]);
            h.update(&b);
            *h.finalize().as_bytes()
        };
        let expected = {
            let mut h = blake3::Hasher::new();
            h.update(&[0x01]);
            h.update(&leaf_a);
            h.update(&leaf_b);
            *h.finalize().as_bytes()
        };
        assert_eq!(root, expected);
    }

    #[test]
    fn four_leaves() {
        let leaves: Vec<[u8; 32]> = (0..4u8)
            .map(|i| *blake3::hash(&[i]).as_bytes())
            .collect();
        let root = merkle_root(&leaves);
        assert_ne!(root, [0u8; 32]);
    }

    #[test]
    fn odd_number_of_leaves() {
        let leaves: Vec<[u8; 32]> = (0..3u8)
            .map(|i| *blake3::hash(&[i]).as_bytes())
            .collect();
        let root = merkle_root(&leaves);
        assert_ne!(root, [0u8; 32]);

        // Odd leaf should be hashed as internal node with itself, not promoted raw
        let two_leaf_root = merkle_root(&leaves[..2]);
        assert_ne!(root, two_leaf_root);
    }
}
