//! The server's HTTP client for the sign-in providers: GitHub and OIDC providers.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ureq::Agent;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

/// How long a request to a provider may take.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The HTTP client: no redirects, HTTPS checked against the platform's root certificates, which it
/// loads at its first use.
pub(crate) fn agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::Specific(Arc::new(platform_roots())))
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("skald-server/", env!("CARGO_PKG_VERSION")))
            .tls_config(tls)
            .build()
            .new_agent()
    })
}

/// The platform's root certificates; those it cannot load are left out.
fn platform_roots() -> Vec<Certificate<'static>> {
    rustls_native_certs::load_native_certs()
        .certs
        .iter()
        .map(|cert| Certificate::from_der(cert.as_ref()).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_platform_root_certificates() {
        assert!(!platform_roots().is_empty());
    }
}
