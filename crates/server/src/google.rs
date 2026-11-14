//! Google sign-in in the browser: OpenID Connect's authorization code flow (OpenID Connect Core 1.0,
//! §3.1) with PKCE (RFC 7636), with the OIDC provider at Google's issuer. It ends in a web session
//! as GitHub sign-in does.

use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::accounts::{self, Method};
use crate::oidc::{OidcError, OidcProvider, OidcSettings};
use crate::sign_in::{
    Failure, Flow, Secrets, SignInSettings, begin, param, random_token, redirect, signed_in_page,
    take,
};
use crate::{AppState, problem};

/// The sign-in start's path under the public address.
pub(crate) const START_PATH: &str = "/auth/google";
/// The callback's path under the public address: the redirect URI of the server's Google client.
pub(crate) const CALLBACK_PATH: &str = "/auth/google/callback";

/// Google, as the server reaches it: discovered at the first sign-in that reaches it, so that the
/// server starts while Google is unreachable.
pub(crate) struct Google {
    settings: OidcSettings,
    provider: Mutex<Option<Arc<OidcProvider>>>,
}

impl Google {
    pub(crate) fn new(settings: OidcSettings) -> Self {
        Google {
            settings,
            provider: Mutex::new(None),
        }
    }

    fn flow(&self) -> Flow<'_> {
        Flow {
            name: "Google",
            provider: &self.settings.issuer,
            start: START_PATH,
            callback: CALLBACK_PATH,
        }
    }

    /// The provider, discovered now if no sign-in has reached it yet.
    async fn provider(&self) -> Result<Arc<OidcProvider>, Failure> {
        let found = self
            .provider
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(provider) = found {
            return Ok(provider);
        }
        let settings = self.settings.clone();
        let provider = tokio::task::spawn_blocking(move || OidcProvider::discover(settings))
            .await
            .map_err(|err| Failure::Provider(format!("the discovery failed: {err}")))?
            .map_err(|err| Failure::Provider(err.to_string()))?;
        let provider = Arc::new(provider);
        // Of discoveries at once, the last one's stays; any of them serves.
        *self.provider.lock().unwrap_or_else(PoisonError::into_inner) = Some(provider.clone());
        Ok(provider)
    }
}

/// The sign-in settings and Google, while the server has Google's settings.
fn google(state: &AppState) -> Option<(&SignInSettings, &Google)> {
    Some((state.sign_in.as_deref()?, state.google.as_deref()?))
}

/// The answer while the server has no Google settings.
fn off() -> Response {
    problem(
        StatusCode::NOT_FOUND,
        "google-sign-in-off",
        "Google sign-in is off",
        "This server has no Google settings.",
    )
}

/// The S256 PKCE challenge of `verifier` (RFC 7636, §4.2).
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The local part of `email`: what is before its last `@`.
fn local_part(email: &str) -> &str {
    email.rsplit_once('@').map_or(email, |(local, _)| local)
}

/// `GET /auth/google`: sends the browser to Google with a new `state`, bound to it by a cookie, and
/// a new `nonce` and PKCE challenge.
pub(crate) async fn start(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    let Some((settings, google)) = google(&state) else {
        return off();
    };
    let flow = google.flow();
    let provider = match google.provider().await {
        Ok(provider) => provider,
        Err(failure) => return failure.page(&flow),
    };
    let secrets = Secrets {
        nonce: random_token(),
        verifier: random_token(),
    };
    let token = match begin(&state.db, settings, &flow, query.as_deref(), Some(&secrets)).await {
        Ok(token) => token,
        Err(failure) => return failure.page(&flow),
    };
    let authorize = provider.authorization_url(
        &settings.public_url.at(CALLBACK_PATH),
        &token,
        &secrets.nonce,
        &challenge(&secrets.verifier),
    );
    redirect(settings, &flow, authorize.as_str(), &token)
}

/// `GET /auth/google/callback`: checks `state`, exchanges the code, checks the ID token, finds or
/// creates the account of its subject and starts a session.
pub(crate) async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let Some((settings, google)) = google(&state) else {
        return off();
    };
    let flow = google.flow();
    match finish(&state.db, settings, google, &headers, query.as_deref()).await {
        Ok((session, return_to)) => signed_in_page(settings, &flow, &session, &return_to),
        Err(failure) => failure.page(&flow),
    }
}

/// The session token and the return address of a sign-in that Google's callback with `query`
/// completes.
async fn finish(
    db: &PgPool,
    settings: &SignInSettings,
    google: &Google,
    headers: &HeaderMap,
    query: Option<&str>,
) -> Result<(String, String), Failure> {
    let (return_to, secrets) = take(db, &google.flow(), headers, query).await?;
    let Secrets { nonce, verifier } =
        secrets.ok_or(Failure::Refused("the sign-in is not an OIDC one."))?;
    if param(query, "error").is_some() {
        return Err(Failure::Refused("Google did not authorize it."));
    }
    let code = param(query, "code")
        .filter(|code| !code.is_empty())
        .ok_or(Failure::Refused("Google gave no code."))?;

    let provider = google.provider().await?;
    let redirect_uri = settings.public_url.at(CALLBACK_PATH);
    let identity = tokio::task::spawn_blocking(move || {
        let id_token = provider.exchange(&code, &redirect_uri, &verifier)?;
        provider.check(&id_token, &nonce)
    })
    .await
    .map_err(|err| Failure::Provider(format!("the request to Google failed: {err}")))?
    .map_err(|err| match err {
        OidcError::Code => Failure::Refused("Google refused the code."),
        OidcError::Token(reason) => {
            eprintln!("skald-server: Google sign-in refused an ID token: {reason}");
            Failure::Refused("Google's ID token failed a check.")
        }
        other => Failure::Provider(other.to_string()),
    })?;
    let method = Method {
        provider: &google.settings.issuer,
        subject: identity.subject,
        login: None,
    };
    let source = identity.email.as_deref().map_or("", local_part);
    let session = accounts::sign_in(db, &method, source)
        .await
        .map_err(Failure::Database)?;
    Ok((session, return_to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_s256_of_the_verifier() {
        // RFC 7636, Appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn local_part_is_before_the_last_at() {
        assert_eq!(local_part("ada.lovelace@example.com"), "ada.lovelace");
        assert_eq!(local_part("\"a@b\"@example.com"), "\"a@b\"");
        assert_eq!(local_part("no-at"), "no-at");
    }
}
