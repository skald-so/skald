//! Acceptance tests of the server's OIDC code: the issuer, the provider's metadata and keys, and the
//! checks of an ID token (OpenID Connect Core 1.0, §3.1.3.7), against the local stand-in for an OIDC
//! provider. No test calls a real provider.

mod discovery;
mod keys;
mod signatures;
mod tokens;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use skald_oidc_stand_in::{Key, Provider};
use skald_server::{ClientSecret, OidcError, OidcIdentity, OidcProvider, OidcSettings};

/// The server's client at the provider.
pub const CLIENT_ID: &str = "skald-test.apps.example";
pub const CLIENT_SECRET: &str = "stand-in-client-secret";
/// The nonce of the sign-in under test.
pub const NONCE: &str = "n-0S6_WzA2Mj";
/// The person's subject and email at the provider.
pub const SUBJECT: &str = "110169484474386276334";
pub const EMAIL: &str = "ada@example.com";

/// A key interval no test waits out: within a test, the keys are read again at most once.
pub const LONG: Duration = Duration::from_secs(600);

/// The server's settings for the provider `issuer`, reading its keys again at most once per
/// `key_refresh`.
pub fn settings(issuer: &str, key_refresh: Duration) -> OidcSettings {
    OidcSettings {
        issuer: issuer.to_owned(),
        client_id: CLIENT_ID.to_owned(),
        client_secret: ClientSecret::new(CLIENT_SECRET.to_owned()).expect("a client secret"),
        key_refresh,
    }
}

/// The server's provider for the stand-in `provider`, discovered.
pub fn discover(provider: &Provider, key_refresh: Duration) -> OidcProvider {
    OidcProvider::discover(settings(provider.issuer(), key_refresh))
        .unwrap_or_else(|err| panic!("discover the stand-in at {}: {err:?}", provider.issuer()))
}

/// A stand-in serving one RSA key, that key, and the server's provider for it.
pub fn started() -> (Provider, Key, OidcProvider) {
    let key = Key::rsa();
    let provider = Provider::start(std::slice::from_ref(&key));
    let oidc = discover(&provider, LONG);
    (provider, key, oidc)
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

/// The claims of a valid ID token from `issuer` for this sign-in, as Google gives them.
pub fn claims(issuer: &str) -> Value {
    let now = now();
    json!({
        "iss": issuer,
        "azp": CLIENT_ID,
        "aud": CLIENT_ID,
        "sub": SUBJECT,
        "email": EMAIL,
        "email_verified": true,
        "iat": now,
        "exp": now + 3600,
        "nonce": NONCE,
    })
}

/// `claims` with the fields of `over` set on it; a `null` in `over` removes the field.
pub fn with(claims: &Value, over: Value) -> Value {
    let mut claims = claims.clone();
    let (Value::Object(fields), Value::Object(over)) = (&mut claims, over) else {
        panic!("claims are a JSON object");
    };
    for (name, value) in over {
        if value.is_null() {
            fields.remove(&name);
        } else {
            fields.insert(name, value);
        }
    }
    claims
}

/// Asserts that `token` passes and gives the test's subject.
#[track_caller]
pub fn passes(oidc: &OidcProvider, token: &str) -> OidcIdentity {
    let identity = oidc
        .check(token, NONCE)
        .unwrap_or_else(|err| panic!("a valid token is refused: {err:?}"));
    assert_eq!(identity.subject, SUBJECT);
    identity
}

/// Asserts that `token` is refused as a token; `what` names the case.
#[track_caller]
pub fn refused(oidc: &OidcProvider, token: &str, what: &str) {
    match oidc.check(token, NONCE) {
        Err(OidcError::Token(..)) => {}
        Err(other) => panic!("{what}: refused, but not as a token: {other:?}"),
        Ok(identity) => panic!("{what}: passes, giving {identity:?}"),
    }
}
