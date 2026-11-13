//! The server's HTTP client for the sign-in providers: GitHub and OIDC providers.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::de::DeserializeOwned;
use ureq::http::Response;
use ureq::tls::{Certificate, RootCerts, TlsConfig};
use ureq::{Agent, Body};

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

/// The JSON body of a `200` answer, or why there is none, such as `answered 404`.
pub(crate) fn json_body<T: DeserializeOwned>(
    response: Result<Response<Body>, ureq::Error>,
) -> Result<T, String> {
    let mut response = response.map_err(|err| format!("is unreachable: {err}"))?;
    let status = response.status();
    if status != 200 {
        return Err(format!("answered {status}"));
    }
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| format!("gave an unreadable answer: {err}"))?;
    serde_json::from_str(&body).map_err(|err| format!("gave an unexpected answer: {err}"))
}

/// `text` from a provider, such as an error code, cut to printable ASCII for the server's output.
pub(crate) fn printable(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_graphic())
        .take(64)
        .collect()
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
    fn printable_keeps_printable_ascii_only() {
        assert_eq!(printable("bad\n code\u{1b}[31m"), "badcode[31m");
        assert_eq!(printable(&"x".repeat(100)).len(), 64);
    }

    #[test]
    fn loads_the_platform_root_certificates() {
        assert!(!platform_roots().is_empty());
    }
}
