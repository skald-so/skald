//! Browser sign-in through an OpenID Connect issuer, discovered on first use.

use std::marker::PhantomData;
use std::sync::{Arc, Mutex, PoisonError};

use crate::AppState;
use crate::links;
use crate::oidc::{OidcError, OidcProvider, OidcSettings};
use crate::sign_in::{
    Failure, Flow, Identity, Provider, ProviderKind, Secrets, SignInSettings, blocking,
};

pub(crate) const GOOGLE_START: &str = "/auth/google";
pub(crate) const GOOGLE_CALLBACK: &str = "/auth/google/callback";
pub(crate) const CORPORATE_START: &str = "/auth/corporate";
pub(crate) const CORPORATE_CALLBACK: &str = "/auth/corporate/callback";

pub(crate) struct Google;
pub(crate) struct Corporate;

pub(crate) trait Issuer: Sized {
    const KIND: ProviderKind;
    const START: &'static str;
    const CALLBACK: &'static str;
    const LINK: Option<&'static str>;

    fn configured(state: &AppState) -> Option<&OidcSignIn<Self>>;
}

impl Issuer for Google {
    const KIND: ProviderKind = ProviderKind::Google;
    const START: &'static str = GOOGLE_START;
    const CALLBACK: &'static str = GOOGLE_CALLBACK;
    const LINK: Option<&'static str> = Some(links::GOOGLE_PATH);

    fn configured(state: &AppState) -> Option<&OidcSignIn<Self>> {
        state.google.as_deref()
    }
}

impl Issuer for Corporate {
    const KIND: ProviderKind = ProviderKind::Corporate;
    const START: &'static str = CORPORATE_START;
    const CALLBACK: &'static str = CORPORATE_CALLBACK;
    const LINK: Option<&'static str> = None;

    fn configured(state: &AppState) -> Option<&OidcSignIn<Self>> {
        state.corporate.as_deref()
    }
}

pub(crate) struct OidcSignIn<I> {
    settings: OidcSettings,
    method_provider: String,
    provider: Mutex<Option<Arc<OidcProvider>>>,
    issuer: PhantomData<I>,
}

impl<I: Issuer> OidcSignIn<I> {
    pub(crate) fn new(settings: OidcSettings) -> Self {
        let method_provider = I::KIND.issuer_key(&settings.issuer);
        Self {
            settings,
            method_provider,
            provider: Mutex::new(None),
            issuer: PhantomData,
        }
    }

    /// Cache successful discovery. A failure allows the next sign-in to try again.
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
        let provider = blocking(Self::NAME, move || {
            if I::KIND == ProviderKind::Corporate {
                OidcProvider::discover_with_token_auth(settings)
            } else {
                OidcProvider::discover(settings)
            }
        })
        .await?
        .map_err(|err| Self::failed(err, "could not discover the issuer"))?;
        let provider = Arc::new(provider);
        *self.provider.lock().unwrap_or_else(PoisonError::into_inner) = Some(provider.clone());
        Ok(provider)
    }

    fn failed(err: OidcError, description: &str) -> Failure {
        // A corporate issuer controls its endpoints and response bodies, which can appear in errors.
        let reason = if I::KIND == ProviderKind::Corporate {
            description.to_owned()
        } else {
            err.to_string()
        };
        Failure::Provider(reason)
    }
}

/// The part of the email used as a username source.
fn local_part(email: &str) -> &str {
    email.rsplit_once('@').map_or(email, |(local, _)| local)
}

impl<I: Issuer> Provider for OidcSignIn<I> {
    const NAME: &'static str = I::KIND.label();

    fn of(state: &AppState) -> Option<(&SignInSettings, &Self)> {
        Some((state.sign_in.as_deref()?, I::configured(state)?))
    }

    fn flow(&self) -> Flow<'_> {
        Flow {
            name: Self::NAME,
            provider: &self.method_provider,
            start: I::START,
            callback: I::CALLBACK,
            link: I::LINK,
        }
    }

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
        Ok(provider
            .authorization_url(
                &settings.web().at(I::CALLBACK),
                state,
                nonce,
                &secrets.challenge(),
                link.then_some("select_account"),
            )
            .into())
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
        let redirect_uri = settings.web().at(I::CALLBACK);
        let identity = blocking(Self::NAME, move || {
            let id_token = provider.exchange(&code, &redirect_uri, &verifier)?;
            provider.check(&id_token, &nonce)
        })
        .await?
        .map_err(|err| match err {
            OidcError::Code => Failure::ProviderRefused("refused the code."),
            OidcError::Token(reason) => {
                eprintln!(
                    "skald-server: {} sign-in refused an ID token: {reason}",
                    Self::NAME
                );
                if I::KIND == ProviderKind::Google {
                    Failure::Refused("Google's ID token failed a check.")
                } else {
                    Failure::ProviderRefused("returned an ID token that failed a check.")
                }
            }
            other => Self::failed(other, "could not verify the issuer's response"),
        })?;
        let google = I::KIND == ProviderKind::Google;
        Ok(Identity {
            source: identity.email.as_deref().map_or("", local_part).to_owned(),
            subject: identity.subject,
            login: None,
            verified_email: identity.email.filter(|_| google && identity.email_verified),
            hosted_domain: identity.hosted_domain.filter(|_| google),
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
