//! TOTP enrollment and nonce-bound second-factor verification.
use serde::{Deserialize, Serialize};
use totp_rs::{Algorithm, Builder, Secret, Totp};
use veyndesk_protocol::{ct_eq, random_token, second_factor_proof, sha256_hex};

#[derive(Clone, Serialize, Deserialize)]
pub struct TwoFactorConfig {
    pub secret: String,
    #[serde(default)]
    pub last_used_step: Option<u64>,
    #[serde(default)]
    pub recovery_hashes: Vec<String>,
}

impl std::fmt::Debug for TwoFactorConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TwoFactorConfig").field("secret", &"[redacted]")
            .field("recovery_codes_remaining", &self.recovery_hashes.len()).finish()
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs()
}

pub fn totp(secret: &str, account: &str) -> anyhow::Result<Totp> {
    Ok(Builder::new().with_algorithm(Algorithm::SHA1).with_digits(6)
        .with_step_duration(30).with_skew(1)
        .with_secret(Secret::try_from_base32(secret).map_err(|_| anyhow::anyhow!("Invalid authenticator configuration"))?)
        .with_issuer(Some("Veynremote")).with_account_name(account)
        .build().map_err(|_| anyhow::anyhow!("Invalid authenticator configuration"))?)
}

pub fn new_config() -> TwoFactorConfig {
    let bytes = hex::decode(random_token(20)).expect("random hex");
    TwoFactorConfig { secret: Secret::from(bytes).to_base32(), last_used_step: None, recovery_hashes: vec![] }
}

pub fn normalize_code(value: &str) -> Option<String> {
    if value.len() > 80 { return None; }
    let code: String = value.chars().filter(|c| *c != '-' && !c.is_ascii_whitespace())
        .map(|c| c.to_ascii_lowercase()).collect();
    if (code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())) ||
        (code.len() == 32 && code.bytes().all(|b| b.is_ascii_hexdigit())) { Some(code) } else { None }
}

pub fn recovery_codes(config: &mut TwoFactorConfig) -> Vec<String> {
    let codes: Vec<String> = (0..10).map(|_| random_token(16)).collect();
    config.recovery_hashes = codes.iter().map(|code| sha256_hex(code)).collect();
    codes.iter().map(|s| s.as_bytes().chunks(8).map(|c| std::str::from_utf8(c).unwrap())
        .collect::<Vec<_>>().join("-")).collect()
}

/// Consume an unused token or recovery code. Callers must durably save the
/// updated config before granting access. No plaintext code crosses the relay.
pub fn consume_proof(config: &mut TwoFactorConfig, secret: &str, nonce: &str, proof: &str, time: u64) -> bool {
    if proof.len() != 64 { return false; }
    let Ok(generator) = totp(&config.secret, "host") else { return false; };
    let step = time / 30;
    for candidate in [step.saturating_sub(1), step, step.saturating_add(1)] {
        if config.last_used_step.is_some_and(|last| candidate <= last) { continue; }
        let code = generator.generate(candidate * 30).to_string();
        if ct_eq(proof, &second_factor_proof(secret, nonce, &sha256_hex(&code))) {
            config.last_used_step = Some(candidate);
            return true;
        }
    }
    if let Some(index) = config.recovery_hashes.iter()
        .position(|hash| ct_eq(proof, &second_factor_proof(secret, nonce, hash))) {
        config.recovery_hashes.remove(index);
        return true;
    }
    false
}

pub fn consume_local(config: &mut TwoFactorConfig, code: &str, time: u64) -> bool {
    let Some(code) = normalize_code(code) else { return false; };
    consume_proof(config, "local-settings", "local", &second_factor_proof("local-settings", "local", &sha256_hex(&code)), time)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> TwoFactorConfig {
        TwoFactorConfig { secret: Secret::from(b"12345678901234567890".to_vec()).to_base32(), last_used_step: None, recovery_hashes: vec![] }
    }
    #[test]
    fn enrollment_produces_google_compatible_qr() {
        let cfg = new_config();
        let generator = totp(&cfg.secret, "Ubuntu desktop").unwrap();
        let url = generator.to_url().unwrap();
        assert!(url.starts_with("otpauth://totp/"));
        assert!(url.contains("issuer=Veynremote"));
        assert_eq!(generator.digits(), 6);
        assert_eq!(generator.step(), 30);
        assert_eq!(generator.algorithm(), Algorithm::SHA1);
        use base64::Engine;
        let png = base64::engine::general_purpose::STANDARD.decode(generator.to_qr_base64().unwrap()).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(!format!("{cfg:?}").contains(&cfg.secret));
    }

    #[test]
    fn rfc6238_sha1_vector_and_replay() {
        let mut cfg = fixture();
        assert_eq!(totp(&cfg.secret, "test").unwrap().generate(59).to_string(), "287082");
        let proof = second_factor_proof("password-secret", "nonce", &sha256_hex("287082"));
        assert!(!consume_proof(&mut cfg, "password-secret", "other-nonce", &proof, 59));
        assert!(!consume_proof(&mut cfg, "wrong-secret", "nonce", &proof, 59));
        assert!(consume_proof(&mut cfg, "password-secret", "nonce", &proof, 59));
        assert!(!consume_proof(&mut cfg, "password-secret", "nonce", &proof, 59));
        let mut restarted: TwoFactorConfig = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert!(!consume_local(&mut restarted, "287082", 59));
    }
    #[test]
    fn recovery_is_single_use_and_only_hashes_persist() {
        let mut cfg = fixture(); let codes = recovery_codes(&mut cfg);
        assert_eq!(codes.len(), 10);
        assert!(!serde_json::to_string(&cfg).unwrap().contains(&codes[0]));
        assert!(consume_local(&mut cfg, &codes[0].to_uppercase(), 59));
        assert!(!consume_local(&mut cfg, &codes[0], 59));
        assert_eq!(cfg.recovery_hashes.len(), 9);
    }
    #[test]
    fn rejects_expired_missing_and_malformed_codes() {
        let mut cfg = fixture();
        assert!(!consume_local(&mut cfg, "287082", 300));
        assert!(!consume_local(&mut cfg, "", 59));
        assert!(!consume_local(&mut cfg, "12345", 59));
        assert!(consume_local(&mut cfg, "287 082", 59));
    }
    #[test]
    fn allows_only_one_step_of_clock_skew() {
        let mut cfg = fixture();
        assert!(consume_local(&mut cfg, "287082", 89));
        assert!(!consume_local(&mut fixture(), "287082", 90));
    }
}
