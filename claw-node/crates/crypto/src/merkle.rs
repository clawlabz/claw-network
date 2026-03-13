//! Simple Merkle tree for computing state roots.

/// Compute the Merkle root of a list of leaf hashes.
///
/// - Empty input → all-zero hash
/// - Single leaf → that leaf's hash
/// - Multiple leaves → binary Merkle tree using blake3
pub fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    if leaves.len() == 1 {
        return leaves[0];
    }

    let mut current_level: Vec<[u8; 32]> = leaves.to_vec();

    while current_level.len() > 1 {
        let mut next_level = Vec::with_capacity((current_level.len() + 1) / 2);

        for chunk in current_level.chunks(2) {
            if chunk.len() == 2 {
                let mut hasher = blake3::Hasher::new();
                hasher.update(&chunk[0]);
                hasher.update(&chunk[1]);
                next_level.push(*hasher.finalize().as_bytes());
            } else {
                // Odd leaf: promote as-is
                next_level.push(chunk[0]);
            }
        }

        current_level = next_level;
    }

    current_level[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_leaves_returns_zeros() {
        assert_eq!(merkle_root(&[]), [0u8; 32]);
    }

    #[test]
    fn single_leaf_returns_itself() {
        let leaf = *blake3::hash(b"hello").as_bytes();
        assert_eq!(merkle_root(&[leaf]), leaf);
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
    }
}
