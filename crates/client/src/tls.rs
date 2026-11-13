//! The client's TLS, for its HTTPS and WSS: rustls with ring, trusting the platform's root
//! certificates, or those `SSL_CERT_FILE` and `SSL_CERT_DIR` name instead.

use std::sync::Arc;

use rustls::crypto::CryptoProvider;
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};
use ureq::tls::{Certificate, RootCerts, TlsConfig};

/// The root certificates the client trusts.
pub struct Roots(Vec<CertificateDer<'static>>);

impl Roots {
    /// The platform's root certificates, or those `SSL_CERT_FILE` and `SSL_CERT_DIR` name, as
    /// rustls-native-certs reads them.
    pub fn load() -> Self {
        Roots(rustls_native_certs::load_native_certs().certs)
    }

    /// The TLS config of a ureq agent that trusts them.
    pub fn ureq(&self) -> TlsConfig {
        let roots: Vec<Certificate<'static>> = self
            .0
            .iter()
            .map(|der| Certificate::from_der(der.as_ref()).to_owned())
            .collect();
        TlsConfig::builder()
            .root_certs(RootCerts::Specific(Arc::new(roots)))
            .unversioned_rustls_crypto_provider(provider())
            .build()
    }

    /// The rustls config of a TLS client that trusts them, for a WebSocket.
    pub fn rustls(&self) -> Arc<ClientConfig> {
        let mut roots = RootCertStore::empty();
        roots.add_parsable_certificates(self.0.iter().cloned());
        let config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("ring supports rustls's default TLS versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}
