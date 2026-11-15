//! Google sign-in in the browser: OpenID Connect's authorization code flow (OpenID Connect Core 1.0,
//! §3.1) with PKCE (RFC 7636), with the OIDC provider at Google's issuer. It ends in a web session
//! as GitHub sign-in does.

use std::sync::{Arc, Mutex, PoisonError};

use crate::AppState;
use crate::links;
use crate::oidc::{OidcError, OidcProvider, OidcSettings};
use crate::sign_in::{Failure, Flow, Identity, Provider, Secrets, SignInSettings, blocking};

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
        let provider = blocking(Self::NAME, move || OidcProvider::discover(settings))
            .await?
            .map_err(|err| Failure::Provider(err.to_string()))?;
        let provider = Arc::new(provider);
        // Of discoveries at once, the last one's stays; any of them serves.
        *self.provider.lock().unwrap_or_else(PoisonError::into_inner) = Some(provider.clone());
        Ok(provider)
    }
}

/// The local part of `email`: what is before its last `@`.
fn local_part(email: &str) -> &str {
    email.rsplit_once('@').map_or(email, |(local, _)| local)
}

impl Provider for Google {
    const NAME: &'static str = "Google";

    fn of(state: &AppState) -> Option<(&SignInSettings, &Self)> {
        Some((state.sign_in.as_deref()?, state.google.as_deref()?))
    }

    fn flow(&self) -> Flow<'_> {
        Flow {
            name: Self::NAME,
            provider: &self.settings.issuer,
            start: START_PATH,
            callback: CALLBACK_PATH,
            link: links::GOOGLE_PATH,
        }
    }

    /// A link asks Google to let the person choose the Google account.
    async fn authorize(
        &self,
        settings: &SignInSettings,
        state: &str,
        secrets: &Secrets,
        link: bool,
    ) -> Result<String, Failure> {
        let provider = self.provider().await?;
        let nonce = secrets
            .nonce
            .as_deref()
            .expect("an OIDC sign-in has a nonce");
        let authorize = provider.authorization_url(
            &settings.public_url.at(CALLBACK_PATH),
            state,
            nonce,
            &secrets.challenge(),
            link.then_some("select_account"),
        );
        Ok(authorize.into())
    }

    async fn identify(
        &self,
        settings: &SignInSettings,
        code: String,
        secrets: Secrets,
    ) -> Result<Identity, Failure> {
        let Secrets { verifier, nonce } = secrets;
        let nonce = nonce.ok_or(Failure::Refused("the sign-in is not an OIDC one."))?;
        let provider = self.provider().await?;
        let redirect_uri = settings.public_url.at(CALLBACK_PATH);
        let identity = blocking(Self::NAME, move || {
            let id_token = provider.exchange(&code, &redirect_uri, &verifier)?;
            provider.check(&id_token, &nonce)
        })
        .await?
        .map_err(|err| match err {
            OidcError::Code => Failure::ProviderRefused("refused the code."),
            OidcError::Token(reason) => {
                eprintln!("skald-server: Google sign-in refused an ID token: {reason}");
                Failure::Refused("Google's ID token failed a check.")
            }
            other => Failure::Provider(other.to_string()),
        })?;
        Ok(Identity {
            source: identity.email.as_deref().map_or("", local_part).to_owned(),
            subject: identity.subject,
            login: None,
            verified_email: identity.email.filter(|_| identity.email_verified),
            hosted_domain: identity.hosted_domain,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_part_is_before_the_last_at() {
        assert_eq!(local_part("ada.lovelace@example.com"), "ada.lovelace");
        assert_eq!(local_part("\"a@b\"@example.com"), "\"a@b\"");
        assert_eq!(local_part("no-at"), "no-at");
    }
}
