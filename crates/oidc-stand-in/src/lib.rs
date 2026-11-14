//! A local stand-in for an OpenID Connect provider, for tests: no test needs a real Google client.
//! Its issuer is `http://127.0.0.1:<port><path>`, and under it it serves
//! - `GET <issuer>/.well-known/openid-configuration`: the provider's metadata (OpenID Connect
//!   Discovery 1.0, §4);
//! - `GET <issuer>/keys`: its keys, the metadata's `jwks_uri`, as a JWK Set (RFC 7517, §5);
//! - `GET <issuer>/authorize`, the metadata's `authorization_endpoint`: the authorization code flow
//!   (OpenID Connect Core 1.0, §3.1.2) for the [`Client`] registered with it, which takes only the
//!   client's redirect URI, matched exactly, as Google does. A person signed in to the stand-in in
//!   their browser goes back to it with `code` and `state`: at once, or by a link on a page that
//!   asks first; without that sign-in, a sign-in page that leads nowhere;
//! - `POST <issuer>/token`, the metadata's `token_endpoint`: gives an access token and an ID token
//!   for a code, once, to the client's ID and secret, in the form or by HTTP Basic, with the PKCE
//!   verifier (RFC 7636) of the code's challenge; the ID token carries the authorization request's
//!   `nonce`;
//! - `GET <issuer>/_stand-in/sign-in?sub=<subject>`: signs the browser in to the stand-in as that
//!   person; with `&ask=1`, authorize then answers a page whose link `#authorize` goes back with the
//!   code and whose link `#cancel` goes back with `error=access_denied`.
//!
//! Tests rotate its keys, break its answers, redirect them, register its client, add its people,
//! set its faults and read the requests it got through [`Provider`], and sign ID tokens, valid and
//! faulty, with [`Key`], [`signed`], [`hmac`] and [`unsigned`].

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::digest::{SHA256, digest};
use ring::hmac as ring_hmac;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{
    ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _, RSA_PKCS1_SHA256, RsaKeyPair,
    RsaPublicKeyComponents,
};
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;

/// The path of the metadata under the issuer.
const METADATA_PATH: &str = "/.well-known/openid-configuration";
/// The path of the keys under the issuer.
const KEYS_PATH: &str = "/keys";
/// The path of the authorization endpoint under the issuer.
const AUTHORIZE_PATH: &str = "/authorize";
/// The path of the token endpoint under the issuer.
const TOKEN_PATH: &str = "/token";
/// The path under the issuer that signs a browser in to the stand-in.
const SIGN_IN_PATH: &str = "/_stand-in/sign-in";
/// The cookie that holds who is signed in to the stand-in in a browser: their subject.
const PERSON_COOKIE: &str = "stand_in_oidc_person";
/// The cookie, `1`, of a browser that authorize asks first.
const ASK_COOKIE: &str = "stand_in_oidc_ask";
/// How long the ID tokens it issues are valid, in seconds.
const ID_TOKEN_LIFETIME: u64 = 3600;

/// `bytes` in unpadded base64url, as JOSE encodes them.
pub fn base64url(bytes: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// `bytes` random bytes in unpadded base64url.
fn random(bytes: usize) -> String {
    let mut out = vec![0; bytes];
    SystemRandom::new()
        .fill(&mut out)
        .expect("random bytes for the stand-in");
    base64url(out)
}

/// A random key ID.
fn random_kid() -> String {
    random(12)
}

/// A signing key of the provider, or of an attacker, and its key ID.
#[derive(Clone)]
pub struct Key {
    kid: String,
    pair: Pair,
}

#[derive(Clone)]
enum Pair {
    Rsa(Arc<RsaKeyPair>),
    Ec(Arc<EcdsaKeyPair>),
}

impl Key {
    /// A new 2048-bit RSA key for RS256, as Google signs with, made by the `openssl` command.
    pub fn rsa() -> Key {
        let made = Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
            ])
            .args(["-outform", "DER"])
            .output()
            .expect("run openssl genpkey");
        assert!(
            made.status.success(),
            "openssl genpkey failed: {}",
            String::from_utf8_lossy(&made.stderr)
        );
        // OpenSSL 3.0 writes the key as PKCS #1, later versions as PKCS #8.
        let pair = RsaKeyPair::from_pkcs8(&made.stdout)
            .or_else(|_| RsaKeyPair::from_der(&made.stdout))
            .expect("read the RSA key openssl made");
        Key {
            kid: random_kid(),
            pair: Pair::Rsa(Arc::new(pair)),
        }
    }

    /// A new P-256 key for ES256.
    pub fn ec() -> Key {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .expect("make a P-256 key");
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .expect("read the P-256 key");
        Key {
            kid: random_kid(),
            pair: Pair::Ec(Arc::new(pair)),
        }
    }

    /// The same key pair under the key ID `kid`.
    pub fn with_kid(&self, kid: &str) -> Key {
        Key {
            kid: kid.to_owned(),
            pair: self.pair.clone(),
        }
    }

    /// Its key ID, the `kid` of its JWK and of the tokens it signs.
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// Its JWS algorithm: `RS256` or `ES256`.
    pub fn alg(&self) -> &'static str {
        match self.pair {
            Pair::Rsa(_) => "RS256",
            Pair::Ec(_) => "ES256",
        }
    }

    /// Its public key as a JWK (RFC 7517, RFC 7518 §6), as the provider lists it.
    pub fn jwk(&self) -> Value {
        match &self.pair {
            Pair::Rsa(pair) => {
                let public = RsaPublicKeyComponents::<Vec<u8>>::from(pair.public());
                json!({
                    "kty": "RSA",
                    "use": "sig",
                    "alg": self.alg(),
                    "kid": self.kid,
                    "n": base64url(&public.n),
                    "e": base64url(&public.e),
                })
            }
            Pair::Ec(pair) => {
                // An uncompressed point: 0x04, then x and y.
                let point = pair.public_key().as_ref();
                json!({
                    "kty": "EC",
                    "use": "sig",
                    "alg": self.alg(),
                    "kid": self.kid,
                    "crv": "P-256",
                    "x": base64url(&point[1..33]),
                    "y": base64url(&point[33..]),
                })
            }
        }
    }

    /// Its public key in each form a verifier may hold it in, with the form's name: DER and PEM of
    /// the SubjectPublicKeyInfo (RFC 5280), for RSA also of the PKCS #1 RSAPublicKey, and the JWK's
    /// JSON. An HMAC keyed with any of them is the algorithm confusion attack.
    pub fn public_forms(&self) -> Vec<(&'static str, Vec<u8>)> {
        let mut forms = Vec::new();
        let (algorithm, key) = match &self.pair {
            Pair::Rsa(pair) => {
                let pkcs1 = pair.public().as_ref().to_vec();
                forms.push(("PKCS #1 DER", pkcs1.clone()));
                forms.push(("PKCS #1 PEM", pem("RSA PUBLIC KEY", &pkcs1)));
                // rsaEncryption, NULL parameters.
                let algorithm = [
                    der(
                        0x06,
                        &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01],
                    ),
                    vec![0x05, 0x00],
                ]
                .concat();
                (algorithm, pkcs1)
            }
            Pair::Ec(pair) => {
                // id-ecPublicKey, prime256v1.
                let algorithm = [
                    der(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
                    der(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]),
                ]
                .concat();
                (algorithm, pair.public_key().as_ref().to_vec())
            }
        };
        let spki = der(
            0x30,
            &[der(0x30, &algorithm), der(0x03, &[&[0][..], &key].concat())].concat(),
        );
        forms.push(("SubjectPublicKeyInfo PEM", pem("PUBLIC KEY", &spki)));
        forms.push(("SubjectPublicKeyInfo DER", spki));
        forms.push(("JWK", self.jwk().to_string().into_bytes()));
        forms
    }

    /// The signature of `input` with its algorithm.
    pub fn signature(&self, input: &[u8]) -> Vec<u8> {
        let rng = SystemRandom::new();
        match &self.pair {
            Pair::Rsa(pair) => {
                let mut signature = vec![0; pair.public().modulus_len()];
                pair.sign(&RSA_PKCS1_SHA256, &rng, input, &mut signature)
                    .expect("sign with the RSA key");
                signature
            }
            Pair::Ec(pair) => pair
                .sign(&rng, input)
                .expect("sign with the P-256 key")
                .as_ref()
                .to_vec(),
        }
    }
}

/// A DER element: `tag`, its definite length, `content`.
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let length = content.len();
    let mut out = vec![tag];
    if length < 0x80 {
        out.push(length as u8);
    } else {
        let bytes: Vec<u8> = length
            .to_be_bytes()
            .into_iter()
            .skip_while(|byte| *byte == 0)
            .collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

/// `der` as PEM with the `label`, 64 characters a line and a final newline.
fn pem(label: &str, der: &[u8]) -> Vec<u8> {
    let body = STANDARD.encode(der);
    let mut text = format!("-----BEGIN {label}-----\n");
    for line in body.as_bytes().chunks(64) {
        text.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        text.push('\n');
    }
    text.push_str(&format!("-----END {label}-----\n"));
    text.into_bytes()
}

/// `base` with the fields of `over` set on it; a `null` in `over` removes the field.
fn merged(mut base: Map<String, Value>, over: Value) -> Value {
    let Value::Object(over) = over else {
        panic!("a JOSE header is a JSON object, not {over}");
    };
    for (name, value) in over {
        if value.is_null() {
            base.remove(&name);
        } else {
            base.insert(name, value);
        }
    }
    Value::Object(base)
}

/// The JWS compact serialization of `claims` under `header`, with the signature `sign` gives for
/// the signing input.
pub fn compact(header: &Value, claims: &Value, sign: impl FnOnce(&[u8]) -> Vec<u8>) -> String {
    let input = format!(
        "{}.{}",
        base64url(header.to_string()),
        base64url(claims.to_string())
    );
    let signature = sign(input.as_bytes());
    format!("{input}.{}", base64url(signature))
}

/// A token of `claims` signed by `key`. Its header is `typ` `JWT` and the key's `alg` and `kid`,
/// with the fields of `header` set over them: `json!({})` for none, `null` to remove one.
pub fn signed(key: &Key, header: Value, claims: &Value) -> String {
    let mut base = Map::new();
    base.insert("alg".to_owned(), json!(key.alg()));
    base.insert("kid".to_owned(), json!(key.kid()));
    base.insert("typ".to_owned(), json!("JWT"));
    compact(&merged(base, header), claims, |input| key.signature(input))
}

/// A token of `claims` with the HMAC algorithm `alg`, `HS256`, `HS384` or `HS512`, keyed with
/// `secret`. Its header is `alg` and `typ` `JWT`, with the fields of `header` set over them.
pub fn hmac(alg: &str, secret: &[u8], header: Value, claims: &Value) -> String {
    let algorithm = match alg {
        "HS256" => ring_hmac::HMAC_SHA256,
        "HS384" => ring_hmac::HMAC_SHA384,
        "HS512" => ring_hmac::HMAC_SHA512,
        _ => panic!("not an HMAC algorithm: {alg}"),
    };
    let mut base = Map::new();
    base.insert("alg".to_owned(), json!(alg));
    base.insert("typ".to_owned(), json!("JWT"));
    let key = ring_hmac::Key::new(algorithm, secret);
    compact(&merged(base, header), claims, |input| {
        ring_hmac::sign(&key, input).as_ref().to_vec()
    })
}

/// A token of `claims` with an empty signature, its header `alg` set to `alg`, such as `none`, and
/// `typ` `JWT`, with the fields of `header` set over them.
pub fn unsigned(alg: &str, header: Value, claims: &Value) -> String {
    let mut base = Map::new();
    base.insert("alg".to_owned(), json!(alg));
    base.insert("typ".to_owned(), json!("JWT"));
    compact(&merged(base, header), claims, |_| Vec::new())
}

/// What the stand-in serves, and what it has been asked.
struct Served {
    issuer: String,
    /// The issuer's path: empty, or such as `/realms/acme`.
    path: String,
    /// `id_token_signing_alg_values_supported` of the default metadata.
    algs: Vec<String>,
    /// The metadata's body instead of the default one.
    metadata: Option<String>,
    keys: Vec<Key>,
    /// The keys' body instead of the JWK Set of `keys`.
    keys_body: Option<String>,
    /// A redirect instead of the metadata: its status and `Location`.
    metadata_redirect: Option<(StatusCode, String)>,
    /// A redirect instead of the keys: its status and `Location`.
    keys_redirect: Option<(StatusCode, String)>,
    metadata_reads: usize,
    key_reads: usize,
    client: Option<Client>,
    people: BTreeMap<String, Person>,
    faults: Vec<Fault>,
    grants: HashMap<String, Grant>,
    codes: Vec<String>,
    issued: Vec<Issued>,
    requests: Vec<Request>,
}

impl Served {
    fn default_metadata(&self) -> Value {
        let issuer = &self.issuer;
        json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}{KEYS_PATH}"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": self.algs,
            "scopes_supported": ["openid", "email"],
            "token_endpoint_auth_methods_supported": ["client_secret_post", "client_secret_basic"],
            "claims_supported": ["aud", "azp", "email", "email_verified", "exp", "iat", "iss", "nonce", "sub"],
            "code_challenge_methods_supported": ["S256"],
        })
    }

    fn key_set(&self) -> String {
        let keys: Vec<Value> = self.keys.iter().map(Key::jwk).collect();
        json!({ "keys": keys }).to_string()
    }
}

type Shared = Arc<Mutex<Served>>;

/// The client registered with the stand-in: the server's, as an OAuth client of a Google Cloud
/// project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub client_id: String,
    pub client_secret: String,
    /// Its only redirect URI: authorize takes only this one, matched exactly, as Google does.
    pub redirect_uri: String,
}

/// A person with an account at the provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    /// `sub`: the person's ID at the provider, which never changes.
    pub subject: String,
    /// `email`, which the person may change.
    pub email: String,
}

impl Person {
    pub fn new(subject: &str, email: &str) -> Person {
        Person {
            subject: subject.to_owned(),
            email: email.to_owned(),
        }
    }
}

/// A fault the stand-in plays until the faults are set again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The person denies the client: authorize goes back with `error=access_denied`.
    AccessDenied,
    /// The token endpoint refuses every code with `invalid_grant`.
    InvalidGrant,
    /// The token endpoint answers with this status and no token.
    TokenStatus { status: u16 },
    /// The token endpoint's ID tokens carry this `nonce` instead of their authorization request's.
    Nonce { nonce: String },
}

/// One of the provider's endpoints of the authorization code flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Authorize,
    Token,
}

/// A request to one of the provider's endpoints of the authorization code flow, as the stand-in
/// received it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub endpoint: Endpoint,
    /// The query parameters and those of a form body.
    pub params: BTreeMap<String, String>,
}

/// What the token endpoint gave for a code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issued {
    pub code: String,
    pub access_token: String,
    pub id_token: String,
}

/// A code authorize issued, and what it was issued for.
struct Grant {
    subject: String,
    redirect_uri: String,
    nonce: Option<String>,
    /// `code_challenge` and `code_challenge_method`, if the request had a challenge.
    challenge: Option<(String, String)>,
    used: bool,
}

/// The running stand-in; it stops when dropped.
pub struct Provider {
    issuer: String,
    address: SocketAddr,
    path: String,
    shared: Shared,
    runtime: Option<Runtime>,
}

/// A runtime for the stand-in's server.
fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_io()
        .build()
        .expect("start the stand-in's runtime")
}

/// The stand-in's endpoints under the issuer's path `path`.
fn router(path: &str, shared: Shared) -> Router {
    Router::new()
        .route(&format!("{path}{AUTHORIZE_PATH}"), get(authorize))
        .route(&format!("{path}{TOKEN_PATH}"), post(token))
        .route(&format!("{path}{SIGN_IN_PATH}"), get(sign_in))
        .fallback(answer)
        .with_state(shared)
}

impl Provider {
    /// Starts a provider whose issuer is `http://127.0.0.1:<port>`, as Google's is a bare origin,
    /// serving `keys`, the first of which [`Provider::sign`] signs with. Its metadata lists RS256
    /// alone, as Google's does.
    pub fn start(keys: &[Key]) -> Provider {
        Provider::start_at("", keys)
    }

    /// As [`Provider::start`], with the issuer `http://127.0.0.1:<port><path>`: `path` is empty or
    /// starts with `/`, such as a corporate IdP's `/realms/acme`.
    pub fn start_at(path: &str, keys: &[Key]) -> Provider {
        assert!(
            path.is_empty() || (path.starts_with('/') && !path.ends_with('/')),
            "an issuer path such as /realms/acme, not {path:?}"
        );
        let runtime = runtime();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind a local port");
        let address = listener.local_addr().expect("the stand-in's address");
        let issuer = format!("http://{address}{path}");
        Provider::serving(runtime, listener, issuer, path, keys)
    }

    /// Serves at `listen` a provider whose issuer is `issuer`, such as `https://oidc.example.test`
    /// in front of a TLS proxy to `listen`, for tests in other processes: the
    /// `skald-oidc-stand-in` binary. Otherwise as [`Provider::start`].
    pub fn serve(listen: SocketAddr, issuer: &str, keys: &[Key]) -> std::io::Result<Provider> {
        let issuer = issuer.trim_end_matches('/');
        let authority = issuer.split_once("://").map_or(issuer, |(_, rest)| rest);
        let path = authority.find('/').map_or("", |slash| &authority[slash..]);
        let runtime = runtime();
        let listener = runtime.block_on(tokio::net::TcpListener::bind(listen))?;
        Ok(Provider::serving(
            runtime,
            listener,
            issuer.to_owned(),
            path,
            keys,
        ))
    }

    /// A provider of `issuer`, its endpoints under `path`, serving on `listener`.
    fn serving(
        runtime: Runtime,
        listener: tokio::net::TcpListener,
        issuer: String,
        path: &str,
        keys: &[Key],
    ) -> Provider {
        let address = listener.local_addr().expect("the stand-in's address");
        let shared = Arc::new(Mutex::new(Served {
            issuer: issuer.clone(),
            path: path.to_owned(),
            algs: vec!["RS256".to_owned()],
            metadata: None,
            keys: keys.to_vec(),
            keys_body: None,
            metadata_redirect: None,
            keys_redirect: None,
            metadata_reads: 0,
            key_reads: 0,
            client: None,
            people: BTreeMap::new(),
            faults: Vec::new(),
            grants: HashMap::new(),
            codes: Vec::new(),
            issued: Vec::new(),
            requests: Vec::new(),
        }));
        let app = router(path, shared.clone());
        runtime.spawn(async move { axum::serve(listener, app).await });
        Provider {
            issuer,
            address,
            path: path.to_owned(),
            shared,
            runtime: Some(runtime),
        }
    }

    fn served(&self) -> MutexGuard<'_, Served> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Its issuer, such as `http://127.0.0.1:41234`, without a trailing slash.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The metadata it serves unless [`Provider::set_metadata`] replaced it.
    pub fn metadata(&self) -> Value {
        self.served().default_metadata()
    }

    /// The algorithms the default metadata lists in `id_token_signing_alg_values_supported`.
    pub fn set_algs(&self, algs: &[&str]) {
        self.served().algs = algs.iter().map(|alg| (*alg).to_owned()).collect();
    }

    /// Serves `metadata` from now on instead of the default.
    pub fn set_metadata(&self, metadata: &Value) {
        self.served().metadata = Some(metadata.to_string());
    }

    /// Serves `body` as the metadata from now on, such as malformed JSON.
    pub fn set_metadata_body(&self, body: &str) {
        self.served().metadata = Some(body.to_owned());
    }

    /// Serves `keys` from now on, the first signing: a key rotation.
    pub fn set_keys(&self, keys: &[Key]) {
        let mut served = self.served();
        served.keys = keys.to_vec();
        served.keys_body = None;
    }

    /// Serves `body` as the keys from now on, such as a malformed JWK Set.
    pub fn set_keys_body(&self, body: &str) {
        self.served().keys_body = Some(body.to_owned());
    }

    /// Answers a request for its metadata from now on with a redirect of `status`, such as `302`,
    /// to `location`.
    pub fn redirect_metadata(&self, status: u16, location: &str) {
        self.served().metadata_redirect = Some((redirect_status(status), location.to_owned()));
    }

    /// Answers a request for its keys from now on with a redirect of `status`, such as `302`, to
    /// `location`.
    pub fn redirect_keys(&self, status: u16, location: &str) {
        self.served().keys_redirect = Some((redirect_status(status), location.to_owned()));
    }

    /// How many times its metadata has been read.
    pub fn metadata_reads(&self) -> usize {
        self.served().metadata_reads
    }

    /// How many times its keys have been read.
    pub fn key_reads(&self) -> usize {
        self.served().key_reads
    }

    /// An ID token of `claims` signed by the first key it serves.
    pub fn sign(&self, claims: &Value) -> String {
        let key = self
            .served()
            .keys
            .first()
            .cloned()
            .expect("the stand-in serves a key");
        signed(&key, json!({}), claims)
    }

    /// Stops serving: from now on its address refuses connections.
    pub fn stop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(5));
        }
    }

    /// After [`Provider::stop`], serves again at the same issuer, with what it served and was
    /// asked before.
    pub fn restart(&mut self) {
        if self.runtime.is_some() {
            return;
        }
        let runtime = runtime();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(self.address))
            .unwrap_or_else(|err| {
                panic!("bind the stand-in's address {} again: {err}", self.address)
            });
        let app = router(&self.path, self.shared.clone());
        runtime.spawn(async move { axum::serve(listener, app).await });
        self.runtime = Some(runtime);
    }

    /// Registers `client`, replacing the one registered before.
    pub fn register(&self, client: Client) {
        self.served().client = Some(client);
    }

    /// Adds `person`, or replaces the person with its subject: an email change.
    pub fn put_person(&self, person: Person) {
        self.served().people.insert(person.subject.clone(), person);
    }

    /// The page that signs a browser in to the stand-in as the person `subject`, as signing in to
    /// Google. Authorize then goes back at once.
    pub fn sign_in_url(&self, subject: &str) -> String {
        format!("{}{SIGN_IN_PATH}?sub={}", self.issuer, encode(subject))
    }

    /// As [`Provider::sign_in_url`], but authorize then asks first: its page's link `#authorize`
    /// goes back with the code, and `#cancel` with `error=access_denied`.
    pub fn asking_sign_in_url(&self, subject: &str) -> String {
        format!("{}&ask=1", self.sign_in_url(subject))
    }

    /// Its authorization endpoint, the metadata's `authorization_endpoint`.
    pub fn authorize_url(&self) -> String {
        format!("{}{AUTHORIZE_PATH}", self.issuer)
    }

    /// Plays `faults` from now on; none for `&[]`.
    pub fn set_faults(&self, faults: &[Fault]) {
        self.served().faults = faults.to_vec();
    }

    /// The requests to `endpoint` so far, in order.
    pub fn requests_to(&self, endpoint: Endpoint) -> Vec<Request> {
        let served = self.served();
        let to = served
            .requests
            .iter()
            .filter(|request| request.endpoint == endpoint);
        to.cloned().collect()
    }

    /// The codes authorize issued so far, in order.
    pub fn codes(&self) -> Vec<String> {
        self.served().codes.clone()
    }

    /// What the token endpoint gave so far, in order.
    pub fn issued(&self) -> Vec<Issued> {
        self.served().issued.clone()
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `status` as a redirect's status.
fn redirect_status(status: u16) -> StatusCode {
    StatusCode::from_u16(status)
        .ok()
        .filter(StatusCode::is_redirection)
        .unwrap_or_else(|| panic!("not a redirect's status: {status}"))
}

/// Every request: the metadata and the keys under the issuer's path, or the redirects set for them,
/// and `404` for the rest.
async fn answer(State(shared): State<Shared>, uri: Uri) -> Response {
    let mut served = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = uri.path();
    let (redirect, body) = if path == format!("{}{METADATA_PATH}", served.path) {
        served.metadata_reads += 1;
        let body = served
            .metadata
            .clone()
            .unwrap_or_else(|| served.default_metadata().to_string());
        (served.metadata_redirect.clone(), body)
    } else if path == format!("{}{KEYS_PATH}", served.path) {
        served.key_reads += 1;
        let body = served.keys_body.clone().unwrap_or_else(|| served.key_set());
        (served.keys_redirect.clone(), body)
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match redirect {
        Some((status, location)) => (status, [(header::LOCATION, location)]).into_response(),
        None => ([(header::CONTENT_TYPE, "application/json")], body).into_response(),
    }
}

/// The parameters of `text`, an `application/x-www-form-urlencoded` query or body; the first of each
/// name.
fn form(text: &str) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    for pair in text.split('&').filter(|pair| !pair.is_empty()) {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        params.entry(decode(name)).or_insert_with(|| decode(value));
    }
    params
}

/// `text` with `+` as a space and its percent-encoded bytes decoded.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' => match bytes.get(at + 1..at + 3).and_then(hex_byte) {
                Some(byte) => {
                    out.push(byte);
                    at += 2;
                }
                None => out.push(b'%'),
            },
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The byte two hex digits give.
fn hex_byte(digits: &[u8]) -> Option<u8> {
    u8::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
}

/// `text` percent-encoded for a query: every byte but the unreserved characters of RFC 3986.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `uri` with `params` and `state`, if any, added to its query.
fn back_url(uri: &str, params: &[(&str, &str)], state: Option<&str>) -> String {
    let mut pairs: Vec<String> = params
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect();
    if let Some(state) = state {
        pairs.push(format!("state={}", encode(state)));
    }
    let separator = if uri.contains('?') { '&' } else { '?' };
    format!("{uri}{separator}{}", pairs.join("&"))
}

/// A redirect to `location`, as the provider goes back to the client.
fn redirect(location: String) -> Response {
    Redirect::to(&location).into_response()
}

/// `url`, a URL [`back_url`] made, as an HTML attribute's value: [`encode`] leaves no `"`, `<` or
/// `>` in its query, and the registered redirect URI has none.
fn attribute(url: &str) -> String {
    url.replace('&', "&amp;")
}

/// The value of the cookie `name` that `headers` carry.
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut pairs = headers
        .get_all(header::COOKIE)
        .into_iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'));
    pairs.find_map(|pair| pair.trim().strip_prefix(name)?.strip_prefix('='))
}

/// The person signed in to the stand-in in the browser that sent `headers`, and whether authorize
/// asks that browser first.
fn signed_in(served: &Served, headers: &HeaderMap) -> Option<(String, bool)> {
    let subject = decode(cookie(headers, PERSON_COOKIE)?);
    let asks = cookie(headers, ASK_COOKIE) == Some("1");
    served
        .people
        .contains_key(&subject)
        .then_some((subject, asks))
}

/// An error page of the provider that goes back nowhere, as Google's for a request it cannot trust.
fn error_page(status: StatusCode, error: &str) -> Response {
    let page = format!(
        "<!doctype html><title>Error</title><h1>Error {}: {error}</h1>",
        status.as_u16()
    );
    (status, Html(page)).into_response()
}

/// `GET <issuer>/authorize`: the authorization request (OpenID Connect Core 1.0, §3.1.2.1).
async fn authorize(
    State(shared): State<Shared>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let params = form(query.as_deref().unwrap_or_default());
    let mut served = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    served.requests.push(Request {
        endpoint: Endpoint::Authorize,
        params: params.clone(),
    });
    let param = |name: &str| params.get(name).map(String::as_str);
    let Some(client) = served.client.clone() else {
        return error_page(StatusCode::UNAUTHORIZED, "invalid_client");
    };
    if param("client_id") != Some(client.client_id.as_str()) {
        return error_page(StatusCode::UNAUTHORIZED, "invalid_client");
    }
    // Only the registered redirect URI, matched exactly; another gets an error page, not a redirect.
    if param("redirect_uri") != Some(client.redirect_uri.as_str()) {
        return error_page(StatusCode::BAD_REQUEST, "redirect_uri_mismatch");
    }
    let redirect_uri = client.redirect_uri;
    let state = param("state");
    if param("response_type") != Some("code") {
        return redirect(back_url(
            &redirect_uri,
            &[("error", "unsupported_response_type")],
            state,
        ));
    }
    if !param("scope").is_some_and(|scope| scope.split(' ').any(|scope| scope == "openid")) {
        return redirect(back_url(
            &redirect_uri,
            &[("error", "invalid_scope")],
            state,
        ));
    }
    let challenge = match (param("code_challenge"), param("code_challenge_method")) {
        (None, None) => None,
        (Some(challenge), method) if matches!(method, None | Some("S256") | Some("plain")) => {
            Some((challenge.to_owned(), method.unwrap_or("plain").to_owned()))
        }
        _ => {
            return redirect(back_url(
                &redirect_uri,
                &[("error", "invalid_request")],
                state,
            ));
        }
    };
    let Some((subject, asks)) = signed_in(&served, &headers) else {
        return Html("<!doctype html><title>Sign in</title><h1>Sign in with Google</h1>")
            .into_response();
    };
    let denied = back_url(&redirect_uri, &[("error", "access_denied")], state);
    if served.faults.contains(&Fault::AccessDenied) {
        return redirect(denied);
    }
    let code = format!("4/0A{}", random(24));
    served.grants.insert(
        code.clone(),
        Grant {
            subject,
            redirect_uri: redirect_uri.clone(),
            nonce: param("nonce").map(str::to_owned),
            challenge,
            used: false,
        },
    );
    served.codes.push(code.clone());
    let authorized = back_url(&redirect_uri, &[("code", &code)], state);
    if asks {
        let page = format!(
            "<!doctype html><title>Choose an account</title>\
             <a id=\"authorize\" href=\"{}\">Continue</a> <a id=\"cancel\" href=\"{}\">Cancel</a>",
            attribute(&authorized),
            attribute(&denied),
        );
        return Html(page).into_response();
    }
    redirect(authorized)
}

/// Whether the request authenticates as `client`: by HTTP Basic, with the ID and secret
/// form-encoded as RFC 6749, §2.3.1, says or as they are, or by `client_id` and `client_secret` in
/// the form `params`.
fn authenticates(client: &Client, headers: &HeaderMap, params: &BTreeMap<String, String>) -> bool {
    let Some(authorization) = headers.get(header::AUTHORIZATION) else {
        let param = |name: &str| params.get(name).map(String::as_str);
        return param("client_id") == Some(client.client_id.as_str())
            && param("client_secret") == Some(client.client_secret.as_str());
    };
    let given = authorization.to_str().unwrap_or_default();
    let (scheme, encoded) = given.split_once(' ').unwrap_or_default();
    let Ok(pair) = STANDARD.decode(encoded.trim()) else {
        return false;
    };
    let plain = format!("{}:{}", client.client_id, client.client_secret);
    let encoded = format!(
        "{}:{}",
        encode(&client.client_id),
        encode(&client.client_secret)
    );
    scheme.eq_ignore_ascii_case("basic") && (pair == plain.as_bytes() || pair == encoded.as_bytes())
}

/// A refusal of the token endpoint (RFC 6749, §5.2).
fn token_error(status: StatusCode, error: &str, description: &str) -> Response {
    let body = json!({ "error": error, "error_description": description });
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// Whether `verifier` is the PKCE verifier of `challenge` under `method` (RFC 7636, §4.6).
fn verifies(verifier: &str, challenge: &str, method: &str) -> bool {
    match method {
        "S256" => base64url(digest(&SHA256, verifier.as_bytes())) == challenge,
        _ => verifier == challenge,
    }
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

/// `POST <issuer>/token`: the token request (OpenID Connect Core 1.0, §3.1.3.1).
async fn token(State(shared): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let params = form(&String::from_utf8_lossy(&body));
    let mut served = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    served.requests.push(Request {
        endpoint: Endpoint::Token,
        params: params.clone(),
    });
    let param = |name: &str| params.get(name).cloned().unwrap_or_default();
    if let Some(Fault::TokenStatus { status }) = served
        .faults
        .iter()
        .find(|fault| matches!(fault, Fault::TokenStatus { .. }))
    {
        let status = StatusCode::from_u16(*status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return token_error(
            status,
            "server_error",
            "The stand-in fails, as a test asked.",
        );
    }
    let Some(client) = served.client.clone() else {
        return token_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "No client is registered.",
        );
    };
    if !authenticates(&client, &headers, &params) {
        return token_error(StatusCode::UNAUTHORIZED, "invalid_client", "Unauthorized");
    }
    if param("grant_type") != "authorization_code" {
        return token_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "Invalid grant_type.",
        );
    }
    let bad_code = || token_error(StatusCode::BAD_REQUEST, "invalid_grant", "Bad Request");
    if served.faults.contains(&Fault::InvalidGrant) {
        return bad_code();
    }
    let code = param("code");
    let Some(grant) = served.grants.get_mut(&code).filter(|grant| !grant.used) else {
        return bad_code();
    };
    if param("redirect_uri") != grant.redirect_uri {
        return token_error(
            StatusCode::BAD_REQUEST,
            "redirect_uri_mismatch",
            "Bad Request",
        );
    }
    if let Some((challenge, method)) = &grant.challenge {
        let verifier = param("code_verifier");
        if verifier.is_empty() || !verifies(&verifier, challenge, method) {
            return token_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "Invalid code verifier.",
            );
        }
    }
    grant.used = true;
    let subject = grant.subject.clone();
    let mut nonce = grant.nonce.clone();
    if let Some(Fault::Nonce { nonce: other }) = served
        .faults
        .iter()
        .find(|fault| matches!(fault, Fault::Nonce { .. }))
    {
        nonce = Some(other.clone());
    }
    let Some(person) = served.people.get(&subject).cloned() else {
        return bad_code();
    };
    let Some(key) = served.keys.first().cloned() else {
        return token_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "The stand-in serves no key.",
        );
    };
    let now = now();
    let mut claims = json!({
        "iss": served.issuer,
        "azp": client.client_id,
        "aud": client.client_id,
        "sub": person.subject,
        "email": person.email,
        "email_verified": true,
        "iat": now,
        "exp": now + ID_TOKEN_LIFETIME,
    });
    if let Some(nonce) = nonce {
        claims["nonce"] = json!(nonce);
    }
    let id_token = signed(&key, json!({}), &claims);
    let access_token = format!("ya29.{}", random(48));
    served.issued.push(Issued {
        code,
        access_token: access_token.clone(),
        id_token: id_token.clone(),
    });
    let body = json!({
        "access_token": access_token,
        "expires_in": 3599,
        "scope": "openid https://www.googleapis.com/auth/userinfo.email",
        "token_type": "Bearer",
        "id_token": id_token,
    });
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// `GET <issuer>/_stand-in/sign-in?sub=<subject>[&ask=1]`: signs the browser in as that person.
async fn sign_in(State(shared): State<Shared>, RawQuery(query): RawQuery) -> Response {
    let params = form(query.as_deref().unwrap_or_default());
    let known = {
        let served = shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        params
            .get("sub")
            .filter(|subject| served.people.contains_key(*subject))
            .cloned()
    };
    let Some(subject) = known else {
        return (StatusCode::NOT_FOUND, Html("<h1>No such person</h1>")).into_response();
    };
    let ask = u8::from(params.get("ask").is_some_and(|ask| ask == "1"));
    let mut response = Html("<h1>Signed in to the provider</h1>").into_response();
    for set in [
        format!("{PERSON_COOKIE}={}", encode(&subject)),
        format!("{ASK_COOKIE}={ask}"),
    ] {
        let set = format!("{set}; Path=/; HttpOnly; SameSite=Lax");
        let value = set
            .parse()
            .expect("a cookie of unreserved characters is a header value");
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}
