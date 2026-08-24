use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// Computes hex(HMAC-SHA256(secret, "{timestamp}\n{body}")).
pub fn sign_hex(secret: &[u8], timestamp: i64, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(format!("{timestamp}\n").as_bytes());
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time verification of a client-supplied hex signature.
pub fn verify(secret: &[u8], timestamp: i64, body: &[u8], provided_hex: &str) -> bool {
    let expected = sign_hex(secret, timestamp, body);
    let Ok(provided) = hex::decode(provided_hex) else {
        return false;
    };
    // compare decoded bytes to avoid case/length side channels
    let expected_bytes = hex::decode(&expected).expect("hex of own digest");
    provided.len() == expected_bytes.len()
        && bool::from(provided.as_slice().ct_eq(&expected_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let sig = sign_hex(b"secret-key", 1_700_000_000, b"{\"a\":1}");
        assert!(verify(b"secret-key", 1_700_000_000, b"{\"a\":1}", &sig));
        assert!(verify(
            b"secret-key",
            1_700_000_000,
            b"{\"a\":1}",
            &sig.to_uppercase()
        ));
    }

    #[test]
    fn wrong_secret_fails() {
        let sig = sign_hex(b"secret-key", 1_700_000_000, b"body");
        assert!(!verify(b"other-key", 1_700_000_000, b"body", &sig));
    }

    #[test]
    fn tampered_body_fails() {
        let sig = sign_hex(b"secret-key", 1, b"a");
        assert!(!verify(b"secret-key", 1, b"b", &sig));
    }

    #[test]
    fn garbage_signature_fails() {
        assert!(!verify(b"secret-key", 1, b"a", "zz-not-hex"));
        assert!(!verify(b"secret-key", 1, b"a", ""));
    }
}
