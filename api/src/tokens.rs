//! Random one-time tokens. The raw token goes to the person (in a link or on screen); only its
//! SHA-256 hash is stored, so a database leak does not leak usable links.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

pub fn new_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

pub fn hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// A short code a person types (agent pairing). Unambiguous characters only.
#[allow(dead_code)]
pub fn short_code(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    rand::random::<[u8; 32]>()[..len].iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokens_are_random_and_hashes_stable() {
        let (a, b) = (super::new_token(), super::new_token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert_eq!(super::hash(&a), super::hash(&a));
        assert_eq!(super::short_code(8).len(), 8);
    }
}
