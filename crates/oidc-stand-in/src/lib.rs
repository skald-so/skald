//! A local stand-in for an OpenID Connect provider, for tests: no test needs a real Google client.
//! Its issuer is `http://127.0.0.1:<port><path>`, and under it it serves
//! - `GET <issuer>/.well-known/openid-configuration`: the provider's metadata (OpenID Connect
//!   Discovery 1.0, §4);
//! - `GET <issuer>/keys`: its keys, the metadata's `jwks_uri`, as a JWK Set (RFC 7517, §5).
//!
//! Tests rotate its keys and break its answers through [`Provider`], and sign ID tokens, valid and
//! faulty, with [`Key`], [`signed`], [`hmac`] and [`unsigned`].

use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
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

/// `bytes` in unpadded base64url, as JOSE encodes them.
pub fn base64url(bytes: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A random key ID.
fn random_kid() -> String {
    let mut bytes = [0; 12];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("random bytes for a key ID");
    base64url(bytes)
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
    metadata_reads: usize,
    key_reads: usize,
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

/// The running stand-in; it stops when dropped.
pub struct Provider {
    issuer: String,
    shared: Shared,
    runtime: Option<Runtime>,
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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .build()
            .expect("start the stand-in's runtime");
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind a local port");
        let address = listener.local_addr().expect("the stand-in's address");
        let issuer = format!("http://{address}{path}");
        let shared = Arc::new(Mutex::new(Served {
            issuer: issuer.clone(),
            path: path.to_owned(),
            algs: vec!["RS256".to_owned()],
            metadata: None,
            keys: keys.to_vec(),
            keys_body: None,
            metadata_reads: 0,
            key_reads: 0,
        }));
        let app = Router::new().fallback(answer).with_state(shared.clone());
        runtime.spawn(async move { axum::serve(listener, app).await });
        Provider {
            issuer,
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
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Every request: the metadata and the keys under the issuer's path, and `404` for the rest.
async fn answer(State(shared): State<Shared>, uri: Uri) -> Response {
    let mut served = shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = uri.path();
    let body = if path == format!("{}{METADATA_PATH}", served.path) {
        served.metadata_reads += 1;
        served
            .metadata
            .clone()
            .unwrap_or_else(|| served.default_metadata().to_string())
    } else if path == format!("{}{KEYS_PATH}", served.path) {
        served.key_reads += 1;
        served.keys_body.clone().unwrap_or_else(|| served.key_set())
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}
