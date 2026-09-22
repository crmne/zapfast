//! Publisher authorization, separate from GitHub's build attestations.
//! Sign the exact checksums.txt bytes with Ed25519 (not Ed25519ph).

use anyhow::{Context, Result, ensure};
use ring::signature::{ED25519, UnparsedPublicKey};

pub(super) fn trusted_key() -> Result<Vec<u8>> {
    let hex = include_str!("../../assets/update-public-key.hex").trim();
    ensure!(
        hex.len() == 64 && hex.is_ascii(),
        "Invalid embedded update public key"
    );
    (0..hex.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&hex[at..at + 2], 16).context("Invalid embedded update public key")
        })
        .collect()
}

pub(super) fn verify(manifest: &[u8], signature: &[u8], key: &[u8]) -> Result<()> {
    ensure!(
        key.len() == 32 && signature.len() == 64,
        "Invalid update publisher signature"
    );
    UnparsedPublicKey::new(&ED25519, key)
        .verify(manifest, signature)
        .map_err(|_| anyhow::anyhow!("The update publisher signature could not be verified"))
}

#[cfg(test)]
pub(super) fn fixture_key() -> ring::signature::Ed25519KeyPair {
    // A test-only key, never trusted by a production or demo download.
    ring::signature::Ed25519KeyPair::from_seed_unchecked(&[42; 32]).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair;

    #[test]
    fn native_packages_signature_is_accepted_without_format_conversion() {
        // Shared with native-packages' Ruby/OpenSSL release signer test.
        // The disposable seed is [42; 32], never the publisher's real key.
        let manifest =
            b"f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d  app-v1.2.3.zip\n";
        let encoded = "562fce6b4dc6e8d30a907f104a3b1c5fbf777c3b849ee23181de624c1398f6af3d1ee6b4a272f87c7100b6f58d34854c1d2285c854335f23edf1c0fa5a9e2a0f";
        let signature: Vec<_> = (0..encoded.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&encoded[at..at + 2], 16).unwrap())
            .collect();
        assert!(verify(manifest, &signature, fixture_key().public_key().as_ref()).is_ok());
        assert!(verify(manifest, &signature, &trusted_key().unwrap()).is_err());
    }

    #[test]
    fn publisher_verification_rejects_changed_manifests_keys_and_signatures() {
        let key = fixture_key();
        let manifest = b"fixture checksums\n";
        let signature = key.sign(manifest);
        assert!(verify(manifest, signature.as_ref(), key.public_key().as_ref()).is_ok());
        for bad in [b"fixture checksums".as_slice(), b"forged checksums\n", b""] {
            assert!(verify(bad, signature.as_ref(), key.public_key().as_ref()).is_err());
        }
        assert!(verify(manifest, &[], key.public_key().as_ref()).is_err());
        assert!(
            verify(
                manifest,
                &signature.as_ref()[..63],
                key.public_key().as_ref()
            )
            .is_err()
        );
        let mut altered = signature.as_ref().to_vec();
        altered[0] ^= 1;
        assert!(verify(manifest, &altered, key.public_key().as_ref()).is_err());
        let other = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[43; 32]).unwrap();
        assert!(verify(manifest, signature.as_ref(), other.public_key().as_ref()).is_err());
        let production = trusted_key().unwrap();
        assert_eq!(production.len(), 32);
        assert_ne!(production, key.public_key().as_ref());
        assert!(verify(manifest, signature.as_ref(), &production).is_err());
    }
}
