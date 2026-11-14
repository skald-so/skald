//! An OpenID Connect provider, given by its issuer: its metadata (OpenID Connect Discovery 1.0, §4),
//! its keys from the metadata's `jwks_uri`, the authorization code flow's requests (OpenID Connect
//! Core 1.0, §3.1.2 and §3.1.3) with PKCE (RFC 7636), and the checks of the ID tokens it signs
//! (§3.1.3.7). Nothing here is specific to one provider.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::signature::{
    ECDSA_P256_SHA256_FIXED, RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::{Host, Url};

use crate::github::ClientSecret;
use crate::http_client::{agent, json_body, printable};

/// The metadata's path under the issuer.
const METADATA_PATH: &str = "/.well-known/openid-configuration";
/// How far, in seconds, `exp` may have passed and `iat` be ahead: the provider's clock and the
/// server's differ.
const CLOCK_SKEW: f64 = 60.0;
/// The scopes a sign-in asks for: `openid` for the ID token, `email` for its `email`.
const SCOPES: &str = "openid email";

/// An OIDC provider's settings.
#[derive(Debug, Clone)]
pub struct OidcSettings {
    /// The provider's issuer, such as `https://accounts.google.com`: https, or http on a loopback
    /// address.
    pub issuer: String,
    /// The server's client at the provider.
    pub client_id: String,
    pub client_secret: ClientSecret,
    /// The least time between two reads of the provider's keys.
    pub key_refresh: Duration,
}

/// Why the provider or an ID token is refused.
#[derive(Debug)]
pub enum OidcError {
    /// The issuer setting.
    Issuer(String),
    /// The provider is unreachable, or its metadata or keys are malformed.
    Provider(String),
    /// The token endpoint refused the code: wrong, expired, used or not of the PKCE verifier.
    Code,
    /// The ID token fails a check.
    Token(String),
}

impl fmt::Display for OidcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OidcError::Issuer(reason) | OidcError::Provider(reason) => f.write_str(reason),
            OidcError::Code => f.write_str("the provider refused the code"),
            OidcError::Token(reason) => write!(f, "the ID token: {reason}"),
        }
    }
}

/// The person an ID token names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcIdentity {
    /// `sub`: the person's ID at the provider.
    pub subject: String,
    /// `email`, if the token has one.
    pub email: Option<String>,
}

/// A JWS algorithm the server verifies ID tokens with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alg {
    Rs256,
    Es256,
}

impl Alg {
    /// The algorithm `name`, if it is RS256 or ES256: never `none` or an HMAC.
    fn named(name: &str) -> Option<Alg> {
        match name {
            "RS256" => Some(Alg::Rs256),
            "ES256" => Some(Alg::Es256),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Alg::Rs256 => "RS256",
            Alg::Es256 => "ES256",
        }
    }
}

/// A public key of the provider, from its JWK Set.
#[derive(Debug)]
struct Key {
    kid: Option<String>,
    /// The JWK's `alg`, which limits the key to that algorithm.
    alg: Option<String>,
    public: Public,
}

#[derive(Debug)]
enum Public {
    /// The modulus and exponent, big-endian.
    Rsa { n: Vec<u8>, e: Vec<u8> },
    /// An uncompressed P-256 point.
    P256(Vec<u8>),
}

impl Key {
    /// Whether `signature` is this key's of `input` under `alg`.
    fn verifies(&self, alg: Alg, input: &[u8], signature: &[u8]) -> bool {
        if self.alg.as_deref().is_some_and(|own| own != alg.name()) {
            return false;
        }
        match (&self.public, alg) {
            (Public::Rsa { n, e }, Alg::Rs256) => RsaPublicKeyComponents { n, e }
                .verify(&RSA_PKCS1_2048_8192_SHA256, input, signature)
                .is_ok(),
            (Public::P256(point), Alg::Es256) => {
                ring::signature::UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
                    .verify(input, signature)
                    .is_ok()
            }
            _ => false,
        }
    }
}

/// A JWK (RFC 7517, §4; RFC 7518, §6) as the provider lists it.
#[derive(Deserialize)]
struct Jwk {
    kty: String,
    kid: Option<String>,
    #[serde(rename = "use")]
    usage: Option<String>,
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
}

impl Jwk {
    /// The key, if it is a signing key of RSA or P-256; an error if such a key is malformed.
    fn key(self) -> Result<Option<Key>, String> {
        if self.usage.as_deref().is_some_and(|usage| usage != "sig") {
            return Ok(None);
        }
        let field = |name: &str, value: Option<String>| {
            let value = value.ok_or(format!("a {} key without `{name}`", self.kty))?;
            URL_SAFE_NO_PAD
                .decode(value)
                .map_err(|_| format!("a {} key's `{name}` is not base64url", self.kty))
        };
        let public = match self.kty.as_str() {
            "RSA" => Public::Rsa {
                n: field("n", self.n)?,
                e: field("e", self.e)?,
            },
            "EC" if self.crv.as_deref() == Some("P-256") => {
                Public::P256([vec![0x04], field("x", self.x)?, field("y", self.y)?].concat())
            }
            _ => return Ok(None),
        };
        Ok(Some(Key {
            kid: self.kid,
            alg: self.alg,
            public,
        }))
    }
}

/// The provider's metadata, the fields the server reads.
#[derive(Deserialize)]
struct Metadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    id_token_signing_alg_values_supported: Vec<String>,
}

/// An OIDC provider, discovered; it checks the ID tokens it signs.
pub struct OidcProvider {
    settings: OidcSettings,
    authorization_endpoint: Url,
    token_endpoint: Url,
    jwks_uri: Url,
    /// The algorithms of the metadata that the server verifies.
    algs: Vec<Alg>,
    keys: RwLock<Arc<Vec<Key>>>,
    /// When the keys were last read, or a read was tried. Held while they are read.
    read_at: Mutex<Instant>,
}

impl OidcProvider {
    /// The provider of `settings`: checks the issuer, reads its metadata and its keys. Blocks.
    pub fn discover(settings: OidcSettings) -> Result<OidcProvider, OidcError> {
        let issuer = &settings.issuer;
        let url = issuer_url(issuer).map_err(OidcError::Issuer)?;
        let metadata_url = format!("{}{METADATA_PATH}", url.as_str().trim_end_matches('/'));
        let metadata: Metadata = fetch("metadata", &metadata_url).map_err(OidcError::Provider)?;
        if metadata.issuer != *issuer {
            return Err(OidcError::Provider(
                "the provider's metadata names another issuer".to_owned(),
            ));
        }
        let endpoint = |name: &str, text: &str| {
            provider_url(text)
                .map_err(|reason| OidcError::Provider(format!("the metadata's {name}: {reason}")))
        };
        let authorization_endpoint =
            endpoint("authorization_endpoint", &metadata.authorization_endpoint)?;
        let token_endpoint = endpoint("token_endpoint", &metadata.token_endpoint)?;
        let jwks_uri = endpoint("jwks_uri", &metadata.jwks_uri)?;
        let mut algs = Vec::new();
        for alg in &metadata.id_token_signing_alg_values_supported {
            algs.extend(Alg::named(alg));
        }
        if algs.is_empty() {
            return Err(OidcError::Provider(
                "the provider's metadata lists neither RS256 nor ES256 for ID tokens".to_owned(),
            ));
        }
        let keys = read_keys(&jwks_uri).map_err(OidcError::Provider)?;
        Ok(OidcProvider {
            settings,
            authorization_endpoint,
            token_endpoint,
            jwks_uri,
            algs,
            keys: RwLock::new(Arc::new(keys)),
            read_at: Mutex::new(Instant::now()),
        })
    }

    /// The authorization request (OpenID Connect Core 1.0, §3.1.2.1) of a sign-in that the provider
    /// sends back to `redirect_uri` with `state`, its ID token with `nonce`, and whose PKCE
    /// `challenge` is S256's: asking for `openid` and `email` only.
    pub(crate) fn authorization_url(
        &self,
        redirect_uri: &str,
        state: &str,
        nonce: &str,
        challenge: &str,
    ) -> Url {
        let mut url = self.authorization_endpoint.clone();
        url.query_pairs_mut()
            .append_pair("client_id", &self.settings.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", SCOPES)
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        url
    }

    /// The ID token the token endpoint gives for `code`, which the provider sent to `redirect_uri`,
    /// with the client's ID and secret and the PKCE `verifier` (OpenID Connect Core 1.0, §3.1.3.1).
    /// Blocks. The access token it also gives is dropped.
    pub(crate) fn exchange(
        &self,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> Result<String, OidcError> {
        #[derive(Deserialize)]
        struct Answer {
            id_token: Option<String>,
            error: Option<String>,
        }
        let failed = |reason: String| {
            OidcError::Provider(format!(
                "the provider's token endpoint at {} {reason}",
                self.token_endpoint
            ))
        };
        let response = agent()
            .post(self.token_endpoint.as_str())
            .header("accept", "application/json")
            .send_form([
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", self.settings.client_id.as_str()),
                ("client_secret", self.settings.client_secret.expose()),
                ("code_verifier", verifier),
            ]);
        let mut response = response.map_err(|err| failed(format!("is unreachable: {err}")))?;
        let status = response.status();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|err| failed(format!("gave an unreadable answer: {err}")))?;
        let answer: Option<Answer> = serde_json::from_str(&body).ok();
        let (id_token, error) =
            answer.map_or((None, None), |answer| (answer.id_token, answer.error));
        match (status.as_u16(), id_token, error) {
            (200, Some(id_token), None) => Ok(id_token),
            (400, _, Some(error)) if error == "invalid_grant" => Err(OidcError::Code),
            (_, _, error) => Err(failed(format!(
                "answered {status} without an ID token: {}",
                printable(error.as_deref().unwrap_or("no error"))
            ))),
        }
    }

    /// The person `id_token` names, if it passes every check for the sign-in of `nonce`. Blocks
    /// when the token names a key the provider has not listed: then reads the keys again, at most
    /// once per `key_refresh`. Keeps nothing of the token.
    pub fn check(&self, id_token: &str, nonce: &str) -> Result<OidcIdentity, OidcError> {
        let refused = |reason: &str| OidcError::Token(reason.to_owned());
        let mut parts = id_token.split('.');
        let (Some(header), Some(payload), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(refused("not a JWS in compact form"));
        };
        let input = &id_token[..header.len() + 1 + payload.len()];
        let header: Header = part(header).ok_or_else(|| refused("a malformed header"))?;
        if header.crit.is_some() {
            return Err(refused("critical header parameters"));
        }
        let alg = Alg::named(&header.alg)
            .filter(|alg| self.algs.contains(alg))
            .ok_or_else(|| refused("an algorithm the server does not take"))?;
        let kid = header.kid.ok_or_else(|| refused("no key ID"))?;
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| refused("a signature that is not base64url"))?;
        let keys = self.keys_with(&kid)?;
        let signed = keys
            .iter()
            .filter(|key| key.kid.as_deref() == Some(kid.as_str()))
            .any(|key| key.verifies(alg, input.as_bytes(), &signature));
        if !signed {
            return Err(refused("not signed by the provider's key"));
        }
        let claims: Claims = part(payload).ok_or_else(|| refused("malformed claims"))?;
        claims
            .check(&self.settings, nonce, now())
            .map_err(refused)?;
        Ok(OidcIdentity {
            subject: claims.sub,
            email: claims.email,
        })
    }

    /// The keys, read again if they lack `kid` and the last read is `key_refresh` ago.
    fn keys_with(&self, kid: &str) -> Result<Arc<Vec<Key>>, OidcError> {
        let has = |keys: &[Key]| keys.iter().any(|key| key.kid.as_deref() == Some(kid));
        let keys = self.keys();
        if has(&keys) {
            return Ok(keys);
        }
        let mut read_at = self.read_at.lock().unwrap_or_else(PoisonError::into_inner);
        let current = self.keys();
        // Read by another check meanwhile, or read too recently.
        if !Arc::ptr_eq(&current, &keys)
            || !due(*read_at, Instant::now(), self.settings.key_refresh)
        {
            return Ok(current);
        }
        *read_at = Instant::now();
        let keys = Arc::new(read_keys(&self.jwks_uri).map_err(OidcError::Provider)?);
        *self.keys.write().unwrap_or_else(PoisonError::into_inner) = keys.clone();
        Ok(keys)
    }

    fn keys(&self) -> Arc<Vec<Key>> {
        self.keys
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Whether the keys, last read at `read_at`, may be read again at `now`.
fn due(read_at: Instant, now: Instant, key_refresh: Duration) -> bool {
    now.saturating_duration_since(read_at) >= key_refresh
}

/// `issuer` as a URL, if it is an issuer: https, or http on a loopback address, with no user, query
/// or fragment.
pub fn issuer_url(issuer: &str) -> Result<Url, String> {
    provider_url(issuer)
        .and_then(|url| {
            let bare = url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none();
            bare.then_some(url)
                .ok_or("an issuer has no user, query or fragment".to_owned())
        })
        .map_err(|reason| format!("the issuer {issuer:?}: {reason}"))
}

/// `text` as a provider's URL: https, or http on a loopback address.
fn provider_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text).map_err(|err| format!("not a URL: {err}"))?;
    let loopback = match url.host() {
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    match url.scheme() {
        "https" => Ok(url),
        "http" if loopback => Ok(url),
        _ => Err("neither https nor http on a loopback address".to_owned()),
    }
}

/// The provider's keys at `jwks_uri`: those for signing with RSA or P-256.
fn read_keys(jwks_uri: &Url) -> Result<Vec<Key>, String> {
    #[derive(Deserialize)]
    struct JwkSet {
        keys: Vec<Jwk>,
    }
    let set: JwkSet = fetch("keys", jwks_uri.as_str())?;
    let mut keys = Vec::new();
    for jwk in set.keys {
        keys.extend(
            jwk.key()
                .map_err(|reason| format!("the provider's keys: {reason}"))?,
        );
    }
    Ok(keys)
}

/// The JSON body of a `200` answer at `url`, the provider's `what`.
fn fetch<T: DeserializeOwned>(what: &str, url: &str) -> Result<T, String> {
    let response = agent().get(url).header("accept", "application/json").call();
    json_body(response).map_err(|reason| format!("the provider's {what} at {url} {reason}"))
}

/// A part of a JWS: base64url of a JSON object.
fn part<T: DeserializeOwned>(encoded: &str) -> Option<T> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).ok()?).ok()
}

/// A JWS's protected header (RFC 7515, §4), the fields the server reads.
#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: Option<String>,
    crit: Option<serde_json::Value>,
}

/// An ID token's claims (OpenID Connect Core 1.0, §2), the ones the server reads.
#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Audience,
    azp: Option<String>,
    exp: f64,
    iat: f64,
    nonce: Option<String>,
    email: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Claims {
    /// Checks the claims for the provider of `settings` and the sign-in of `nonce`, at `now`, in
    /// seconds since the Unix epoch.
    fn check(&self, settings: &OidcSettings, nonce: &str, now: f64) -> Result<(), &'static str> {
        let client_id = settings.client_id.as_str();
        if self.iss != settings.issuer {
            return Err("another issuer");
        }
        if self.sub.is_empty() {
            return Err("an empty subject");
        }
        let audience = match &self.aud {
            Audience::One(aud) => aud == client_id,
            Audience::Many(auds) => auds.len() == 1 && auds[0] == client_id,
        };
        if !audience {
            return Err("an audience other than the client alone");
        }
        if self.azp.as_deref().is_some_and(|azp| azp != client_id) {
            return Err("another authorized party");
        }
        if now >= self.exp + CLOCK_SKEW {
            return Err("expired");
        }
        if self.iat > now + CLOCK_SKEW {
            return Err("issued ahead");
        }
        if nonce.is_empty() || self.nonce.as_deref() != Some(nonce) {
            return Err("another sign-in's nonce");
        }
        Ok(())
    }
}

/// Seconds since the Unix epoch.
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use skald_oidc_stand_in::{Client, Fault, Key as StandInKey, Provider, signed};

    use super::*;

    const CLIENT_ID: &str = "skald-test.apps.example";
    const NONCE: &str = "n-0S6_WzA2Mj";
    const NOW: f64 = 1_800_000_000.0;

    fn settings(issuer: &str) -> OidcSettings {
        OidcSettings {
            issuer: issuer.to_owned(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: ClientSecret::new("secret".to_owned()).expect("a secret"),
            key_refresh: Duration::from_secs(600),
        }
    }

    fn token_claims(issuer: &str) -> Value {
        let now = now() as u64;
        json!({
            "iss": issuer, "aud": CLIENT_ID, "sub": "1", "iat": now, "exp": now + 3600, "nonce": NONCE,
        })
    }

    /// Claims that pass at [`NOW`].
    fn claims() -> Claims {
        Claims {
            iss: "https://idp.example".to_owned(),
            sub: "1".to_owned(),
            aud: Audience::One(CLIENT_ID.to_owned()),
            azp: None,
            exp: NOW + 3600.0,
            iat: NOW,
            nonce: Some(NONCE.to_owned()),
            email: None,
        }
    }

    fn check_at(claims: &Claims, nonce: &str) -> Result<(), &'static str> {
        claims.check(&settings("https://idp.example"), nonce, NOW)
    }

    #[test]
    fn keys_are_due_once_the_interval_has_passed() {
        let read_at = Instant::now();
        let interval = Duration::from_millis(300);
        assert!(due(read_at, read_at + interval, interval));
        assert!(!due(
            read_at,
            read_at + interval - Duration::from_nanos(1),
            interval
        ));
        assert!(due(read_at, read_at, Duration::ZERO));
    }

    #[test]
    fn exp_and_iat_take_the_clock_skew_and_no_more() {
        let mut claims = claims();
        assert_eq!(check_at(&claims, NONCE), Ok(()));
        claims.exp = NOW - CLOCK_SKEW + 0.5;
        assert_eq!(check_at(&claims, NONCE), Ok(()));
        claims.exp = NOW - CLOCK_SKEW;
        assert_eq!(check_at(&claims, NONCE), Err("expired"));

        let mut claims = self::claims();
        claims.iat = NOW + CLOCK_SKEW;
        assert_eq!(check_at(&claims, NONCE), Ok(()));
        claims.iat = NOW + CLOCK_SKEW + 0.5;
        assert_eq!(check_at(&claims, NONCE), Err("issued ahead"));
    }

    #[test]
    fn an_empty_subject_is_refused() {
        let mut claims = claims();
        claims.sub = String::new();
        assert_eq!(check_at(&claims, NONCE), Err("an empty subject"));
    }

    #[test]
    fn an_empty_nonce_is_refused_even_when_the_token_has_one() {
        let mut claims = claims();
        claims.nonce = Some(String::new());
        assert_eq!(check_at(&claims, ""), Err("another sign-in's nonce"));
    }

    #[test]
    fn loopback_addresses_take_http() {
        for url in [
            "https://idp.example",
            "http://127.0.0.2:8080",
            "http://[::1]:8080",
        ] {
            assert!(provider_url(url).is_ok(), "{url}");
        }
        for url in [
            "http://[::2]:8080",
            "http://localhost:8080",
            "http://10.0.0.1",
        ] {
            assert!(provider_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn an_issuer_with_a_user_query_or_fragment_is_refused() {
        for issuer in [
            "https://user@idp.example",
            "https://:password@idp.example",
            "https://idp.example?realm=acme",
            "https://idp.example#top",
        ] {
            match OidcProvider::discover(settings(issuer)) {
                Err(OidcError::Issuer(..)) => {}
                Err(other) => panic!("{issuer}: refused, but not as an issuer: {other:?}"),
                Ok(_) => panic!("{issuer}: taken as an issuer"),
            }
        }
    }

    #[test]
    fn metadata_without_rs256_or_es256_is_refused() {
        let provider = Provider::start(&[StandInKey::ec()]);
        provider.set_algs(&["HS256", "none", "RS384", "PS256"]);
        match OidcProvider::discover(settings(provider.issuer())) {
            Err(OidcError::Provider(..)) => {}
            Err(other) => panic!("refused, but not as the provider's fault: {other:?}"),
            Ok(_) => panic!("a provider whose algorithms the server does not verify is taken"),
        }
    }

    /// The provider serving the JWK Set of `jwks`, the server's provider for it, and a token signed
    /// by `key` under its key ID.
    fn serving(key: &StandInKey, jwks: &[Value]) -> (Provider, OidcProvider, String) {
        let provider = Provider::start(std::slice::from_ref(key));
        provider.set_algs(&["RS256", "ES256"]);
        provider.set_keys_body(&json!({ "keys": jwks }).to_string());
        let oidc = OidcProvider::discover(settings(provider.issuer())).expect("discovered");
        let token = signed(key, json!({}), &token_claims(provider.issuer()));
        (provider, oidc, token)
    }

    #[test]
    fn keys_of_other_kinds_are_left_out() {
        let key = StandInKey::ec();
        // Each is malformed as a key the server takes: only left out, it breaks nothing.
        let others = [
            json!({ "kty": "oct", "kid": "oct", "k": "*" }),
            json!({ "kty": "OKP", "kid": "okp", "crv": "Ed25519", "x": "*" }),
            json!({ "kty": "EC", "kid": "p384", "crv": "P-384", "x": "*", "y": "*" }),
            json!({ "kty": "RSA", "kid": "enc", "use": "enc", "n": "*", "e": "*" }),
            key.jwk(),
        ];
        let (_provider, oidc, token) = serving(&key, &others);
        oidc.check(&token, NONCE).expect("a token by the P-256 key");
    }

    #[test]
    fn a_key_for_encryption_or_another_algorithm_does_not_verify() {
        for (field, value) in [("use", "enc"), ("alg", "ES384")] {
            let key = StandInKey::ec();
            let mut jwk = key.jwk();
            jwk[field] = json!(value);
            let (_provider, oidc, token) = serving(&key, &[jwk]);
            let result = oidc.check(&token, NONCE);
            assert!(
                matches!(result, Err(OidcError::Token(..))),
                "{field} {value}: {result:?}"
            );
        }
    }

    #[test]
    fn a_p256_key_without_a_coordinate_is_malformed() {
        let key = StandInKey::ec();
        let mut jwk = key.jwk();
        jwk.as_object_mut().expect("a JWK object").remove("y");
        let provider = Provider::start(std::slice::from_ref(&key));
        provider.set_keys_body(&json!({ "keys": [jwk] }).to_string());
        provider.set_algs(&["ES256"]);
        assert!(matches!(
            OidcProvider::discover(settings(provider.issuer())),
            Err(OidcError::Provider(..))
        ));
    }

    #[test]
    fn a_token_with_critical_header_parameters_is_refused() {
        let key = StandInKey::ec();
        let (provider, oidc, token) = serving(&key, &[key.jwk()]);
        oidc.check(&token, NONCE).expect("the token without crit");
        let critical = signed(
            &key,
            json!({ "crit": ["exp"], "exp": 1 }),
            &token_claims(provider.issuer()),
        );
        assert!(matches!(
            oidc.check(&critical, NONCE),
            Err(OidcError::Token(..))
        ));
    }

    #[test]
    fn a_refused_code_is_told_from_the_token_endpoints_failures() {
        let provider = Provider::start(&[StandInKey::ec()]);
        let redirect_uri = "http://127.0.0.1:9/callback";
        provider.register(Client {
            client_id: CLIENT_ID.to_owned(),
            client_secret: "secret".to_owned(),
            redirect_uri: redirect_uri.to_owned(),
        });
        let oidc = OidcProvider::discover(settings(provider.issuer())).expect("discovered");
        let verifier = "v".repeat(43);
        let exchange = || oidc.exchange("4/0Aunknown", redirect_uri, &verifier);

        // The stand-in refuses a code it did not issue with `invalid_grant`.
        assert!(matches!(exchange(), Err(OidcError::Code)));
        for status in [400, 401, 500, 503] {
            provider.set_faults(&[Fault::TokenStatus { status }]);
            let result = exchange();
            assert!(
                matches!(result, Err(OidcError::Provider(..))),
                "{status}: {result:?}"
            );
        }
        provider.set_faults(&[]);
        let refused = OidcProvider::discover(OidcSettings {
            client_secret: ClientSecret::new("another".to_owned()).expect("a secret"),
            ..settings(provider.issuer())
        })
        .expect("discovered");
        let result = refused.exchange("4/0Aunknown", redirect_uri, &verifier);
        assert!(
            matches!(result, Err(OidcError::Provider(..))),
            "a refused client secret: {result:?}"
        );
    }

    #[test]
    fn an_error_shows_its_reason() {
        let reason = "the provider's keys at https://idp.example/keys answered 500";
        assert_eq!(OidcError::Provider(reason.to_owned()).to_string(), reason);
        assert_eq!(OidcError::Issuer(reason.to_owned()).to_string(), reason);
        assert_eq!(
            OidcError::Token("expired".to_owned()).to_string(),
            "the ID token: expired"
        );
        assert_eq!(OidcError::Code.to_string(), "the provider refused the code");
    }
}
