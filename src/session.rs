use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

use crate::queue::SessionId;

type HmacSha256 = Hmac<Sha256>;

pub struct SessionManager {
    key: Vec<u8>,
}

impl SessionManager {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            key: secret.to_vec(),
        }
    }

    /// Create a signed token for the given session ID.
    /// Format: base64(session_id_bytes[16] || issued_at_secs[8] || hmac[32])
    pub fn create_token(&self, session_id: SessionId) -> String {
        let id_bytes = session_id.0.as_bytes();
        let issued_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let issued_bytes = issued_at.to_be_bytes();

        let mut mac =
            HmacSha256::new_from_slice(&self.key).expect("HMAC key should be valid");
        mac.update(id_bytes);
        mac.update(&issued_bytes);
        let signature = mac.finalize().into_bytes();

        let mut payload = Vec::with_capacity(56);
        payload.extend_from_slice(id_bytes);
        payload.extend_from_slice(&issued_bytes);
        payload.extend_from_slice(&signature);

        URL_SAFE_NO_PAD.encode(&payload)
    }

    /// Parse and verify a token, returning the session ID if valid.
    pub fn verify_token(&self, token: &str) -> Option<SessionId> {
        let payload = URL_SAFE_NO_PAD.decode(token).ok()?;
        if payload.len() != 56 {
            return None;
        }

        let id_bytes = &payload[..16];
        let issued_bytes = &payload[16..24];
        let signature = &payload[24..56];

        let mut mac =
            HmacSha256::new_from_slice(&self.key).expect("HMAC key should be valid");
        mac.update(id_bytes);
        mac.update(issued_bytes);
        mac.verify_slice(signature).ok()?;

        let uuid = Uuid::from_slice(id_bytes).ok()?;
        Some(SessionId(uuid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_roundtrip() {
        let mgr = SessionManager::new(b"test-secret-key-1234567890");
        let id = SessionId::new();
        let token = mgr.create_token(id);
        let parsed = mgr.verify_token(&token).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn test_tampered_token_rejected() {
        let mgr = SessionManager::new(b"test-secret-key-1234567890");
        let id = SessionId::new();
        let mut token = mgr.create_token(id);
        // Tamper with the token
        let bytes = token.as_bytes().to_vec();
        let mut modified = bytes;
        if let Some(b) = modified.get_mut(5) {
            *b = b.wrapping_add(1);
        }
        let tampered = String::from_utf8(modified).unwrap();
        assert!(mgr.verify_token(&tampered).is_none());
    }

    #[test]
    fn test_wrong_key_rejected() {
        let mgr1 = SessionManager::new(b"key-one");
        let mgr2 = SessionManager::new(b"key-two");
        let id = SessionId::new();
        let token = mgr1.create_token(id);
        assert!(mgr2.verify_token(&token).is_none());
    }

    #[test]
    fn test_invalid_token() {
        let mgr = SessionManager::new(b"key");
        assert!(mgr.verify_token("not-a-valid-token").is_none());
        assert!(mgr.verify_token("").is_none());
    }
}
