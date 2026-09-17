use crate::error::{BackendError, Result};
use crate::execution::config::PrivateKeySecret;
use crate::execution::transaction::hex_0x;
use crate::signing::eip712::keccak256;
use crate::types::AccountId;
use k256::ecdsa::SigningKey;
use std::fmt;

#[derive(Clone)]
pub struct ExecutorSigner {
    signing_key: SigningKey,
    address: AccountId,
}

impl fmt::Debug for ExecutorSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutorSigner")
            .field("address", &self.address)
            .field("private_key", &"<redacted>")
            .finish()
    }
}

impl ExecutorSigner {
    pub fn from_private_key(secret: &PrivateKeySecret) -> Result<Self> {
        let key_bytes = parse_private_key(secret.expose_secret())?;
        let signing_key = SigningKey::from_slice(&key_bytes).map_err(|error| {
            BackendError::Config(format!("invalid EXECUTOR_PRIVATE_KEY: {error}"))
        })?;
        let verifying_key = signing_key.verifying_key();
        let public_key = verifying_key.to_encoded_point(false);
        let public_key = public_key.as_bytes();
        let hash = keccak256(&public_key[1..]);
        let address = AccountId::new(hex_0x(&hash[12..]));
        Ok(Self {
            signing_key,
            address,
        })
    }

    /// PERPS_BASE_SEPOLIA_CLOSED_TEST_EXECUTION_LIFECYCLE_AND_LOCAL_KEYSTORE_V1
    /// — decrypt a Web3 v3 keystore JSON file using a password
    /// read from an ephemeral password file. Both files MUST have
    /// permissions `≤ 0o600` (owner read/write only); anything more
    /// permissive is refused. The password buffer is not written to
    /// any log; error messages omit the secret. The decrypted
    /// private key never leaves this function except as the k256
    /// `SigningKey` inside the returned signer.
    ///
    /// Fails closed on:
    /// - missing keystore or password file;
    /// - keystore or password file with group/world read bits set;
    /// - invalid JSON or unsupported keystore version;
    /// - wrong password (surfaced as `invalid keystore password`);
    /// - private key bytes that fail `k256::SigningKey::from_slice`.
    pub fn from_v3_keystore(
        keystore_path: &std::path::Path,
        password_file_path: &std::path::Path,
    ) -> Result<Self> {
        require_secret_file_permissions(keystore_path, "EXECUTOR_KEYSTORE_PATH")?;
        require_secret_file_permissions(password_file_path, "EXECUTOR_KEYSTORE_PASSWORD_FILE")?;

        let mut password = std::fs::read_to_string(password_file_path).map_err(|error| {
            BackendError::Config(format!(
                "EXECUTOR_KEYSTORE_PASSWORD_FILE: read failed: {error}"
            ))
        })?;
        // Trim trailing newline / whitespace introduced by editors —
        // this is a policy choice and is documented in the operator
        // procedure.
        let password_trimmed = password.trim().to_string();
        // Best-effort zeroization of the on-heap password buffer. We
        // do not add the `zeroize` dependency for a single u8 vec —
        // overwriting the bytes in place is sufficient for the
        // process-lifetime threat model (no core dumps, no swap on the
        // rehearsal box).
        for byte in unsafe { password.as_bytes_mut() } {
            *byte = 0;
        }
        drop(password);

        let key_bytes: Vec<u8> = eth_keystore::decrypt_key(keystore_path, &password_trimmed)
            .map_err(|error| {
                // eth-keystore surfaces "MacMismatch" for wrong password;
                // map it to a clear message that never echoes the input.
                let display = error.to_string().to_ascii_lowercase();
                let hint = if display.contains("mac") {
                    "invalid keystore password".to_string()
                } else if display.contains("kdf") || display.contains("scrypt") {
                    format!("unsupported keystore KDF: {error}")
                } else {
                    format!("keystore decrypt failed: {error}")
                };
                BackendError::Config(format!("EXECUTOR_KEYSTORE_PATH: {hint}"))
            })?;
        // Zero the password now that decryption is complete.
        let mut password_trimmed = password_trimmed;
        for byte in unsafe { password_trimmed.as_bytes_mut() } {
            *byte = 0;
        }
        drop(password_trimmed);

        if key_bytes.len() != 32 {
            return Err(BackendError::Config(format!(
                "EXECUTOR_KEYSTORE_PATH: decrypted key must be 32 bytes (got {})",
                key_bytes.len()
            )));
        }
        let signing_key = SigningKey::from_slice(&key_bytes).map_err(|error| {
            BackendError::Config(format!(
                "EXECUTOR_KEYSTORE_PATH: k256 rejected decrypted key: {error}"
            ))
        })?;
        // Zero the decrypted-key vec — SigningKey now owns a copy.
        let mut key_bytes = key_bytes;
        for byte in key_bytes.iter_mut() {
            *byte = 0;
        }
        drop(key_bytes);

        let verifying_key = signing_key.verifying_key();
        let public_key = verifying_key.to_encoded_point(false);
        let public_key = public_key.as_bytes();
        let hash = keccak256(&public_key[1..]);
        let address = AccountId::new(hex_0x(&hash[12..]));
        Ok(Self {
            signing_key,
            address,
        })
    }

    pub fn address(&self) -> &AccountId {
        &self.address
    }

    pub fn sign_prehash(&self, hash: &[u8; 32]) -> Result<RecoverableSignature> {
        let (signature, recovery_id) =
            self.signing_key
                .sign_prehash_recoverable(hash)
                .map_err(|error| {
                    BackendError::BroadcastRejected(format!("transaction signing failed: {error}"))
                })?;
        Ok(RecoverableSignature {
            y_parity: recovery_id.to_byte(),
            r: signature.r().to_bytes().into(),
            s: signature.s().to_bytes().into(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoverableSignature {
    pub y_parity: u8,
    pub r: [u8; 32],
    pub s: [u8; 32],
}

/// Reject secret files (keystore JSON, password file) whose Unix
/// permissions grant read/write access beyond the owner. Any
/// group/world bit forces a `Config` error — this is the same
/// posture ssh, gpg, and cargo credentials take. Symlinks are
/// resolved by `metadata()`.
fn require_secret_file_permissions(path: &std::path::Path, env_key: &str) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let md = std::fs::metadata(path).map_err(|error| {
        BackendError::Config(format!(
            "{env_key}: cannot stat {}: {error}",
            path.display()
        ))
    })?;
    if !md.is_file() {
        return Err(BackendError::Config(format!(
            "{env_key}: {} is not a regular file",
            path.display()
        )));
    }
    let mode = md.mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(BackendError::Config(format!(
            "{env_key}: {} has unsafe permissions {mode:04o} (must be ≤ 0600)",
            path.display()
        )));
    }
    Ok(())
}

fn parse_private_key(value: &str) -> Result<[u8; 32]> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BackendError::Config(
            "invalid EXECUTOR_PRIVATE_KEY: expected 32-byte hex".to_string(),
        ));
    }
    let mut bytes = [0u8; 32];
    for index in 0..32 {
        bytes[index] = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).map_err(|error| {
            BackendError::Config(format!("invalid EXECUTOR_PRIVATE_KEY: {error}"))
        })?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    #[test]
    fn signer_derives_executor_address_without_exposing_key_in_debug() {
        let signer =
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(TEST_KEY.to_string())).unwrap();
        let debug = format!("{signer:?}");

        assert_eq!(
            signer.address().0,
            "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23"
        );
        assert!(!debug.contains("4c0883"));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn invalid_private_key_is_rejected() {
        let error = ExecutorSigner::from_private_key(&PrivateKeySecret::new("0x1234".to_string()))
            .unwrap_err();

        assert!(error.to_string().contains("invalid EXECUTOR_PRIVATE_KEY"));
    }

    // ================================================================
    // PERPS_BASE_SEPOLIA_CLOSED_TEST_EXECUTION_LIFECYCLE_AND_LOCAL_KEYSTORE_V1
    // — LocalKeystore signer tests. Uses only EPHEMERAL keystores;
    // never the real operator password.
    // ================================================================

    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    struct EphemeralDir(PathBuf);

    impl Drop for EphemeralDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tmp_dir(tag: &str) -> EphemeralDir {
        let base =
            std::env::temp_dir().join(format!("deopt_keystore_test_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        // Owner-only for the containing dir.
        let mut perm = fs::metadata(&base).unwrap().permissions();
        perm.set_mode(0o700);
        fs::set_permissions(&base, perm).unwrap();
        EphemeralDir(base)
    }

    fn write_secret_file(path: &std::path::Path, contents: &str, mode: u32) {
        fs::write(path, contents).unwrap();
        let mut perm = fs::metadata(path).unwrap().permissions();
        perm.set_mode(mode);
        fs::set_permissions(path, perm).unwrap();
    }

    fn generate_ephemeral_keystore(dir: &std::path::Path, password: &str) -> (PathBuf, [u8; 32]) {
        use rand::RngCore;
        let mut rng = rand::thread_rng();
        let mut pk = [0u8; 32];
        // Ensure scalar is in-range for secp256k1: any 32 bytes with
        // MSB cleared and non-zero LSB works for these tests.
        loop {
            rng.fill_bytes(&mut pk);
            pk[0] &= 0x7f;
            if pk.iter().any(|b| *b != 0) {
                break;
            }
        }
        // encrypt_key returns the keystore id (UUID); we chose the
        // filename explicitly via `Some("ks.json")` so the file lands
        // at dir/ks.json regardless of the return value.
        let _id = eth_keystore::encrypt_key(dir, &mut rng, pk, password, Some("ks.json")).unwrap();
        let path = dir.join("ks.json");
        let mut perm = fs::metadata(&path).unwrap().permissions();
        perm.set_mode(0o600);
        fs::set_permissions(&path, perm).unwrap();
        (path, pk)
    }

    fn signer_from_ephemeral(dir: &std::path::Path, password: &str) -> ExecutorSigner {
        let (ks, _pk) = generate_ephemeral_keystore(dir, password);
        let pw = dir.join("pw");
        write_secret_file(&pw, password, 0o600);
        ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap()
    }

    #[test]
    fn keystore_correct_password_loads_signer() {
        let dir = tmp_dir("ok");
        let signer = signer_from_ephemeral(&dir.0, "correct-horse-battery");
        assert_eq!(signer.address().0.len(), 42);
        assert!(signer.address().0.starts_with("0x"));
    }

    #[test]
    fn keystore_wrong_password_fails_closed() {
        let dir = tmp_dir("wrongpw");
        let (ks, _) = generate_ephemeral_keystore(&dir.0, "expected");
        let pw = dir.0.join("pw");
        write_secret_file(&pw, "wrong-password", 0o600);
        let err = ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid keystore password"), "got: {msg}");
        // Must not echo the password we tried.
        assert!(!msg.contains("wrong-password"));
    }

    #[test]
    fn keystore_malformed_json_fails() {
        let dir = tmp_dir("malformed");
        let ks = dir.0.join("bad.json");
        write_secret_file(&ks, "{not: valid json", 0o600);
        let pw = dir.0.join("pw");
        write_secret_file(&pw, "irrelevant", 0o600);
        let err = ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap_err();
        assert!(err.to_string().to_ascii_lowercase().contains("decrypt"));
    }

    #[test]
    fn keystore_missing_file_fails() {
        let dir = tmp_dir("missing_ks");
        let ks = dir.0.join("does_not_exist.json");
        let pw = dir.0.join("pw");
        write_secret_file(&pw, "any", 0o600);
        let err = ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap_err();
        assert!(err.to_string().contains("EXECUTOR_KEYSTORE_PATH"));
    }

    #[test]
    fn keystore_missing_password_file_fails() {
        let dir = tmp_dir("missing_pw");
        let (ks, _) = generate_ephemeral_keystore(&dir.0, "any");
        let pw = dir.0.join("does_not_exist");
        let err = ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap_err();
        assert!(err.to_string().contains("EXECUTOR_KEYSTORE_PASSWORD_FILE"));
    }

    #[test]
    fn keystore_unsafe_permissions_fail() {
        let dir = tmp_dir("badperm_ks");
        let (ks, _) = generate_ephemeral_keystore(&dir.0, "any");
        let mut perm = fs::metadata(&ks).unwrap().permissions();
        perm.set_mode(0o644);
        fs::set_permissions(&ks, perm).unwrap();
        let pw = dir.0.join("pw");
        write_secret_file(&pw, "any", 0o600);
        let err = ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unsafe permissions"), "got: {msg}");
        assert!(msg.contains("EXECUTOR_KEYSTORE_PATH"));
    }

    #[test]
    fn keystore_unsafe_password_file_permissions_fail() {
        let dir = tmp_dir("badperm_pw");
        let (ks, _) = generate_ephemeral_keystore(&dir.0, "any");
        let pw = dir.0.join("pw");
        write_secret_file(&pw, "any", 0o644);
        let err = ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap_err();
        assert!(err.to_string().contains("EXECUTOR_KEYSTORE_PASSWORD_FILE"));
    }

    #[test]
    fn keystore_derived_address_signs_recoverably() {
        let dir = tmp_dir("sign");
        let (ks, pk) = generate_ephemeral_keystore(&dir.0, "x");
        let pw = dir.0.join("pw");
        write_secret_file(&pw, "x", 0o600);
        let signer = ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap();
        // Reference signer built directly from the raw private key.
        let hex = pk.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let reference =
            ExecutorSigner::from_private_key(&PrivateKeySecret::new(format!("0x{hex}"))).unwrap();
        assert_eq!(signer.address().0, reference.address().0);
        // Same 32-byte digest → same recovery.
        let digest = [7u8; 32];
        let sig_a = signer.sign_prehash(&digest).unwrap();
        let sig_b = reference.sign_prehash(&digest).unwrap();
        assert_eq!(sig_a.r, sig_b.r);
        assert_eq!(sig_a.s, sig_b.s);
        assert_eq!(sig_a.y_parity, sig_b.y_parity);
    }

    #[test]
    fn keystore_error_does_not_echo_secret_material() {
        let dir = tmp_dir("noleak");
        let (ks, _) = generate_ephemeral_keystore(&dir.0, "my-secret-password-1234");
        let pw = dir.0.join("pw");
        write_secret_file(&pw, "different-wrong-value-5678", 0o600);
        let err = ExecutorSigner::from_v3_keystore(&ks, &pw).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("my-secret-password-1234"));
        assert!(!msg.contains("different-wrong-value-5678"));
    }
}
