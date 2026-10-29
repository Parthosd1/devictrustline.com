//! Browser push notifications (Web Push). Messages are encrypted for the subscribing browser
//! (RFC 8291, aes128gcm) and the server identifies itself with a VAPID key (RFC 8292). Without
//! VAPID keys configured, push is reported as unavailable and nothing is sent.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hkdf::Hkdf;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::EncodePrivateKey;
use p256::{PublicKey, SecretKey};
use sha2_10::Sha256;

#[derive(Clone)]
pub struct PushConfig {
    secret: SecretKey,
    /// The public key browsers subscribe with (uncompressed point, base64url).
    pub public_key: String,
    /// Who runs this server: a mailto: or https: address push services can contact.
    pub subject: String,
}

impl std::fmt::Debug for PushConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushConfig").field("public_key", &self.public_key).field("subject", &self.subject).finish()
    }
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim().trim_end_matches('=')).ok()
}

fn public_of(secret: &SecretKey) -> Vec<u8> {
    secret.public_key().to_encoded_point(false).as_bytes().to_vec()
}

fn random_secret() -> SecretKey {
    loop {
        if let Ok(key) = SecretKey::from_slice(&rand::random::<[u8; 32]>()) {
            return key;
        }
    }
}

impl PushConfig {
    /// From VAPID_PRIVATE_KEY (32 bytes, base64url) and VAPID_SUBJECT. VAPID_PUBLIC_KEY, when given,
    /// must match the private key.
    pub fn new(private_key: &str, public_key: Option<&str>, subject: &str) -> Result<PushConfig, String> {
        let raw = unb64(private_key).ok_or("VAPID_PRIVATE_KEY is not base64url")?;
        let secret = SecretKey::from_slice(&raw).map_err(|_| "VAPID_PRIVATE_KEY is not a P-256 private key")?;
        let derived = b64(&public_of(&secret));
        if public_key.is_some_and(|p| p.trim() != derived) {
            return Err("VAPID_PUBLIC_KEY does not match VAPID_PRIVATE_KEY".into());
        }
        if !(subject.starts_with("mailto:") || subject.starts_with("https://")) {
            return Err("VAPID_SUBJECT must be a mailto: or https:// address".into());
        }
        Ok(PushConfig { secret, public_key: derived, subject: subject.to_string() })
    }

    /// A new key pair, printed by `devicetrustline vapid-keys`: (private, public).
    pub fn generate() -> (String, String) {
        let secret = random_secret();
        (b64(&secret.to_bytes()), b64(&public_of(&secret)))
    }

    /// The Authorization header value for a push service: a short-lived ES256 token for its origin.
    fn authorization(&self, endpoint: &str) -> Result<String, String> {
        let url = url::Url::parse(endpoint).map_err(|e| e.to_string())?;
        let audience = url.origin().ascii_serialization();
        let claims = serde_json::json!({
            "aud": audience,
            "exp": chrono::Utc::now().timestamp() + 12 * 3600,
            "sub": self.subject,
        });
        let der = self.secret.to_pkcs8_der().map_err(|e| e.to_string())?;
        let key = jsonwebtoken::EncodingKey::from_ec_der(der.as_bytes());
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        let jwt = jsonwebtoken::encode(&header, &claims, &key).map_err(|e| e.to_string())?;
        Ok(format!("vapid t={jwt}, k={}", self.public_key))
    }
}

/// Encrypts `payload` for one subscription (RFC 8291, a single aes128gcm record).
pub fn encrypt(p256dh: &str, auth: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
    let ua_public_bytes = unb64(p256dh).ok_or("subscription key is not base64url")?;
    let ua_public = PublicKey::from_sec1_bytes(&ua_public_bytes).map_err(|_| "subscription key is not a P-256 key")?;
    let auth_secret = unb64(auth).filter(|a| a.len() == 16).ok_or("subscription auth secret must be 16 bytes")?;
    if payload.len() > 3000 {
        return Err("payload too large".into());
    }
    let as_secret = random_secret();
    let as_public = public_of(&as_secret);
    let shared = p256::ecdh::diffie_hellman(as_secret.to_nonzero_scalar(), ua_public.as_affine());

    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(&ua_public_bytes);
    key_info.extend_from_slice(&as_public);
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(&auth_secret), shared.raw_secret_bytes().as_slice())
        .expand(&key_info, &mut ikm)
        .map_err(|e| e.to_string())?;

    let salt: [u8; 16] = rand::random();
    let prk = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek).map_err(|e| e.to_string())?;
    prk.expand(b"Content-Encoding: nonce\0", &mut nonce).map_err(|e| e.to_string())?;

    let mut plaintext = payload.to_vec();
    plaintext.push(2); // last-record delimiter, no padding
    let cipher = Aes128Gcm::new_from_slice(&cek).map_err(|e| e.to_string())?;
    let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), plaintext.as_slice()).map_err(|e| e.to_string())?;

    let mut body = salt.to_vec();
    body.extend_from_slice(&4096u32.to_be_bytes());
    body.push(as_public.len() as u8);
    body.extend_from_slice(&as_public);
    body.extend_from_slice(&ciphertext);
    Ok(body)
}

pub enum PushError {
    /// The subscription no longer exists (the browser unsubscribed); stop using it.
    Gone(String),
    /// Worth trying again later.
    Retry(String),
}

/// Sends one encrypted message to a subscription's push service. Returns the service's message id.
pub async fn send(
    config: &PushConfig,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    payload: &[u8],
) -> Result<String, PushError> {
    let body = encrypt(p256dh, auth, payload).map_err(PushError::Gone)?;
    let authorization = config.authorization(endpoint).map_err(PushError::Gone)?;
    let response = http()
        .post(endpoint)
        .header("Authorization", authorization)
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("TTL", "86400")
        .header("Urgency", "normal")
        .body(body)
        .send()
        .await
        .map_err(|e| PushError::Retry(e.to_string()))?;
    let status = response.status().as_u16();
    let location = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    match status {
        200..=299 => Ok(if location.is_empty() { format!("{status}") } else { location }),
        404 | 410 => Err(PushError::Gone(format!("push service answered {status}: subscription expired"))),
        400 | 403 | 413 => Err(PushError::Gone(format!("push service refused the message ({status})"))),
        _ => Err(PushError::Retry(format!("push service answered {status}"))),
    }
}

pub fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_and_mismatches_are_refused() {
        let (private, public) = PushConfig::generate();
        let config = PushConfig::new(&private, Some(&public), "mailto:ops@example.com").unwrap();
        assert_eq!(config.public_key, public);
        assert_eq!(unb64(&public).unwrap().len(), 65);
        let (_, other) = PushConfig::generate();
        assert!(PushConfig::new(&private, Some(&other), "mailto:ops@example.com").is_err());
        assert!(PushConfig::new(&private, None, "ops@example.com").is_err());
        let auth = config.authorization("https://push.example.com/send/abc").unwrap();
        assert!(auth.starts_with("vapid t=") && auth.ends_with(&public));
    }

    #[test]
    fn encrypts_a_record_the_browser_can_read() {
        // Decrypt as the browser would, with the browser's private key.
        let browser = random_secret();
        let browser_public = public_of(&browser);
        let auth: [u8; 16] = rand::random();
        let body = encrypt(&b64(&browser_public), &b64(&auth), b"hello").unwrap();
        let (salt, rest) = body.split_at(16);
        assert_eq!(&rest[..4], &4096u32.to_be_bytes());
        let key_len = rest[4] as usize;
        let as_public = &rest[5..5 + key_len];
        let ciphertext = &rest[5 + key_len..];
        let sender = PublicKey::from_sec1_bytes(as_public).unwrap();
        let shared = p256::ecdh::diffie_hellman(browser.to_nonzero_scalar(), sender.as_affine());
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(&browser_public);
        info.extend_from_slice(as_public);
        let mut ikm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(&auth), shared.raw_secret_bytes().as_slice()).expand(&info, &mut ikm).unwrap();
        let prk = Hkdf::<Sha256>::new(Some(salt), &ikm);
        let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
        prk.expand(b"Content-Encoding: aes128gcm\0", &mut cek).unwrap();
        prk.expand(b"Content-Encoding: nonce\0", &mut nonce).unwrap();
        let plain = Aes128Gcm::new_from_slice(&cek).unwrap().decrypt(Nonce::from_slice(&nonce), ciphertext).unwrap();
        assert_eq!(plain, b"hello\x02");
    }
}
