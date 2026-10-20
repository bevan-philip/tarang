use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};
use std::sync::Arc;

// SHA-256 hashes of the SubjectPublicKeyInfo of the eight eMudhra
// intermediates whose private keys were compromised in October 2026
// (CVE-2026-111943). Same list as Go's crypto/x509 distrustedSPKIHashes.
pub const DISTRUSTED_SPKI_HASHES: &[&str] = &[
    "ebed6475c2c2d80fcb199f3fe8dd6e6b3ce60b77eb15d4e42c3dec0c6f5b5105",
    "49dd353dc2b919a3770645117d0e60c16115e7d1bc259eefe734b00ea434fe5f",
    "e0254c7fcdc265e2f0f8a428dabb75ab4ae039b4872d62f5ee57f0877d5a8a41",
    "5c0660a2b164910d8ff589c18e14552e9e0ec237dcefe2cb736d41a4811f81a0",
    "43bffa19d6ef651451f6841e019931e96f6f2eaeff28ab4b6f1ce4576de2717b",
    "63ecefee431d49523bc1136c4e72d0206a477086caf606cd5cf025686c109cf8",
    "ec4ffa1ad0ac88081314b7aaffef6a91f97cd57291bb5ba686c5415f5eb4f6b6",
    "92c063d67e8f14db71d288e11266ffdaf69cbf115f7bab14f3bc2f68d25e0e30",
];

pub fn spki_sha256(cert: &CertificateDer<'_>) -> Option<String> {
    let cert = webpki::EndEntityCert::try_from(cert).ok()?;
    let hash = Sha256::digest(cert.subject_public_key_info().as_ref());
    Some(hash.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn contains_distrusted(certs: &[CertificateDer<'_>], distrusted: &[&str]) -> bool {
    certs
        .iter()
        .filter_map(spki_sha256)
        .any(|hash| distrusted.contains(&hash.as_str()))
}

// The system trust store still chains to these intermediates until the OS
// catches up, and only browsers get OneCRL/CRLSet, so reject them here.
#[derive(Debug)]
pub struct DistrustingVerifier {
    inner: Arc<dyn ServerCertVerifier>,
    distrusted: &'static [&'static str],
}

impl DistrustingVerifier {
    pub fn new(inner: Arc<dyn ServerCertVerifier>, distrusted: &'static [&'static str]) -> Self {
        Self { inner, distrusted }
    }
}

impl ServerCertVerifier for DistrustingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if contains_distrusted(std::slice::from_ref(end_entity), self.distrusted)
            || contains_distrusted(intermediates, self.distrusted)
        {
            return Err(Error::InvalidCertificate(CertificateError::Revoked));
        }
        self.inner
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

// Mirrors reqwest's default rustls setup (platform verifier, h2 + http/1.1),
// with the distrust check in front of the verifier.
pub fn client_config() -> rustls::ClientConfig {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let platform = rustls_platform_verifier::Verifier::new(provider.clone())
        .expect("failed to load platform certificate verifier");
    let verifier = DistrustingVerifier::new(Arc::new(platform), DISTRUSTED_SPKI_HASHES);

    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("invalid TLS versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::pem::PemObject;

    const TEST_CERT: &str = "-----BEGIN CERTIFICATE-----
MIIBhDCCASmgAwIBAgIUI7itEvpBSe/rluSvEylj+LNxouEwCgYIKoZIzj0EAwIw
FjEUMBIGA1UEAwwLdGFyYW5nIHRlc3QwIBcNMjYxMDIwMTczNzM1WhgPMjEyNjA5
MjYxNzM3MzVaMBYxFDASBgNVBAMMC3RhcmFuZyB0ZXN0MFkwEwYHKoZIzj0CAQYI
KoZIzj0DAQcDQgAENOwCFw/cGVZnEnD2tQ/aayh6c3Np4LJq4pSNbdtbJPvYEZKj
awXj2AsjJyKAwHxPzGA1b1QeAFebD0T5iJnlIqNTMFEwHQYDVR0OBBYEFE94PtIY
3jfBteuLIyG683MP0KNsMB8GA1UdIwQYMBaAFE94PtIY3jfBteuLIyG683MP0KNs
MA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSQAwRgIhAKKQPH5dXeWWMdxJ
VjqSOKg4jjQEBSpluluJUiZidIEDAiEA952FX62yoAb4QwiDmskN1iUKY+U9FB+/
k6OOP8UNXws=
-----END CERTIFICATE-----";

    // openssl x509 -pubkey -noout | openssl pkey -pubin -outform der | sha256sum
    const TEST_SPKI_HASH: &str = "36f4e49cade59f7603c402a0311bda6744be84f7b8d03f388933528390c1cf94";

    fn test_cert() -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(TEST_CERT.as_bytes()).unwrap()
    }

    #[test]
    fn spki_sha256_hashes_the_whole_spki() {
        assert_eq!(spki_sha256(&test_cert()).as_deref(), Some(TEST_SPKI_HASH));
    }

    #[test]
    fn contains_distrusted_matches_any_cert_in_chain() {
        let certs = [CertificateDer::from(b"not a cert".to_vec()), test_cert()];
        assert!(contains_distrusted(&certs, &[TEST_SPKI_HASH]));
        assert!(!contains_distrusted(&certs, DISTRUSTED_SPKI_HASHES));
    }

    #[test]
    fn verifier_rejects_distrusted_intermediate() {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let platform = rustls_platform_verifier::Verifier::new(provider).unwrap();
        let verifier = DistrustingVerifier::new(Arc::new(platform), &[TEST_SPKI_HASH]);

        let result = verifier.verify_server_cert(
            &CertificateDer::from(b"leaf".to_vec()),
            &[test_cert()],
            &ServerName::try_from("example.com").unwrap(),
            &[],
            UnixTime::now(),
        );

        assert_eq!(
            result.unwrap_err(),
            Error::InvalidCertificate(CertificateError::Revoked)
        );
    }

    #[test]
    fn verifier_defers_to_inner_otherwise() {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let platform = rustls_platform_verifier::Verifier::new(provider).unwrap();
        let verifier = DistrustingVerifier::new(Arc::new(platform), DISTRUSTED_SPKI_HASHES);

        let result = verifier.verify_server_cert(
            &test_cert(),
            &[],
            &ServerName::try_from("example.com").unwrap(),
            &[],
            UnixTime::now(),
        );

        assert_ne!(
            result.unwrap_err(),
            Error::InvalidCertificate(CertificateError::Revoked)
        );
    }
}
