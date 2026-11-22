//! Linking a device with a link code (`docs/protocol/membership.md`, "Linking a device" and "Link
//! code"), only by its person: at a terminal, or on the local page `--browser` opens
//! ([`crate::page`]).
//!
//! - The new device, waiting for linking, shows a code and runs it through the server's relay: a
//!   hello per run, at most 3 runs that take a reply, until the code expires or its person asks for
//!   a fresh one. From a grant that passes its checks it keeps the master key, its certificate and
//!   the link object in the local state, then has the server store the certificate; a later run
//!   stores one the server did not take. Stored, it pins the master key, starts each space from the
//!   revision the link object names and pins the personal space.
//! - `skald link`, on a device with a valid certificate, takes a code its person types, answers the
//!   account's latest hello, shows the new device's `name` and kind and, once its person confirms,
//!   issues the certificate and a link object of the revisions it accepted. After 3 failed runs in
//!   a row it runs no code until its person confirms the new device shows a new one. It takes the
//!   person's answers in its own process and hands each step after them to the helper ([`Step`]);
//!   once the new device has KeyPackages, it adds it to the device's channels.

use std::cell::RefCell;
use std::env;
use std::io::{self, IsTerminal as _};
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_crypto::{
    CpaceInitiator, Isk, LinkCode, LinkEntry, LinkPairing, check_device, check_link, link_session,
    sign_link,
};
use skald_proto::{
    AccountId, CERT_LABEL, Cbor as _, Cert, CertBody, DeviceId, DeviceKind, Link, LinkBody,
    LinkGrantBody, LinkMessage, LinkRequestBody, PROOF_LABEL, Unknown, VersionRange,
};
use zeroize::Zeroizing;

use crate::connection::{self, Connection, Method, printable};
use crate::device::{self, DEVICES_LIMIT, Keys};
use crate::devices::{self, signed_in_with_secrets, text, to_session};
use crate::follow;
use crate::groups;
use crate::key_packages;
use crate::person::{Person, Pressed, Presses};
use crate::revocations;
use crate::secrets;
use crate::sign_in;
use crate::spaces::{self, Scope, now};
use crate::state::{self, Link as Linked, State};

/// The setting of how long a code lives, in seconds, for tests.
const LIFETIME_SETTING: &str = "SKALD_LINK_CODE_LIFETIME";
/// How long a code lives without the setting.
const LIFETIME: Duration = Duration::from_secs(300);
/// The runs of a code that may fail at the new device, and the failed runs in a row after which
/// the issuing device runs no code until its person confirms a new one.
const ATTEMPTS: u32 = 3;
/// How long a side waits between its reads of a run's messages.
const POLL: Duration = Duration::from_millis(200);

/// Where a run's hello goes, and its later messages: with a session; without one, where the run
/// opens, and its messages, its hello first.
const HELLO_PATH: &str = "/account/links";
const MESSAGES_PATH: &str = "/account/links/messages";
const SESSIONLESS_PATH: &str = "/links";
const SESSIONLESS_MESSAGES_PATH: &str = "/links/messages";
/// Where the issuing device takes the hello of a pairing its person entered.
const PAIRING_PATH: &str = "/account/links/pairing";

/// Whether its person runs this at a terminal: both its input and its output are one.
pub fn at_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// `LinkHello` in the API spec.
#[derive(Deserialize)]
struct Hello {
    hello: Option<String>,
}

/// `PairedLinkHello` in the API spec.
#[derive(Deserialize)]
struct PairedHello {
    hello: String,
}

/// `SessionlessLinkRun` in the API spec.
#[derive(Deserialize)]
struct SessionlessRun {
    pairing: String,
    token: String,
}

/// `LinkMessages` in the API spec.
#[derive(Deserialize)]
struct Messages {
    messages: Vec<String>,
}

/// How a side reaches a run: by its session's account, or by the run's token without a session.
#[derive(Clone)]
enum Way {
    Session(String),
    Token(String),
}

impl Way {
    /// The bearer token of its requests.
    fn token(&self) -> &str {
        match self {
            Way::Session(token) | Way::Token(token) => token,
        }
    }
}

/// A later message of the run reached by `way`: `POST /account/links/messages`, or `POST
/// /links/messages` with the run's token.
fn send(
    connection: &mut Connection,
    way: &Way,
    message: &LinkMessage,
) -> Result<(), connection::Error> {
    let request =
        serde_json::json!({ "message": URL_SAFE_NO_PAD.encode(message.to_cbor()) }).to_string();
    let path = match way {
        Way::Session(_) => MESSAGES_PATH,
        Way::Token(_) => SESSIONLESS_MESSAGES_PATH,
    };
    connection
        .send_json_as(Method::Post, path, way.token(), request.as_bytes())
        .map(drop)
}

/// Ends the run `session`, reached by `way`, if it is still open.
fn abort(connection: &mut Connection, way: &Way, session: [u8; 16]) {
    let _ = send(connection, way, &LinkMessage::Abort { session });
}

/// The later messages of the open run `session`, reached by `way`, that decode, in the server's
/// order; `None` once the run ended.
fn messages(
    connection: &mut Connection,
    way: &Way,
    session: &[u8; 16],
) -> Result<Option<Vec<LinkMessage>>, connection::Error> {
    let (path, over) = match way {
        Way::Session(_) => (
            format!("{MESSAGES_PATH}/{}", URL_SAFE_NO_PAD.encode(session)),
            404,
        ),
        Way::Token(_) => (SESSIONLESS_MESSAGES_PATH.to_owned(), 401),
    };
    let body = match connection.get_as(&path, way.token()) {
        Ok(body) => body,
        Err(connection::Error::Refused { status, .. }) if status == over => return Ok(None),
        Err(err) => return Err(err),
    };
    let listed: Messages = serde_json::from_slice(&body).map_err(|_| {
        connection::Error::Unexpected("a link run's messages without their list".to_owned())
    })?;
    Ok(Some(
        listed
            .messages
            .iter()
            .filter_map(|text| URL_SAFE_NO_PAD.decode(text).ok())
            .filter_map(|bytes| LinkMessage::from_cbor(&bytes).ok())
            .filter(|message| message.session() == session)
            .collect(),
    ))
}

/// The new device, waiting for linking: its connection and local state, its session and account
/// once it has them, itself and its keys, and whether its runs go without a session, each with a
/// pairing.
pub struct NewDevice<'a> {
    pub connection: &'a mut Connection,
    pub state: &'a State,
    pub token: Option<String>,
    pub account: Option<AccountId>,
    pub device: &'a state::Device,
    pub keys: &'a Keys,
    pub sessionless: bool,
}

/// How a run of a code ended at the new device.
enum Run {
    /// It took a grant that passes its checks: the master key, its complete certificate and the
    /// link object, in CBOR. Without a session, the issuer's certificate is checked only once the
    /// device has one ([`check_granted`]).
    Granted {
        session: [u8; 16],
        master: [u8; 32],
        cert: Vec<u8>,
        link: Vec<u8>,
    },
    /// It failed once the device took a reply: it spent an attempt.
    Failed,
    /// It ended before a reply: it spent none.
    Ended,
}

/// An open run of a code at the new device: its `session` and way, its side of CPace, and the
/// code as it shows it, with the run's pairing if it has one.
struct OpenRun {
    session: [u8; 16],
    way: Way,
    initiator: Option<CpaceInitiator>,
    isk: Option<Isk>,
    shown: String,
}

/// How showing link codes ended.
pub enum Shown {
    /// A run linked the device with its session: the server stored its certificate.
    Linked,
    /// A run granted the device, which had no session, its certificate, kept as unchecked: the
    /// device then got the token of a device session by its key, which stored the certificate.
    ByKey(String),
    /// `tick` stopped it, or its person's input ended with no code left.
    Stopped,
}

/// After `intro`, shows `person` link codes until a run links the device, then stores its
/// certificate: with its session ([`store`]), or without one by its key. A fresh code comes once
/// `presses` says its person asks for one, which it waits for once a code ends. Between its reads,
/// `tick` runs, and stops it on `true`; it stops too once its person's input ends with no code
/// left.
pub fn show(
    new: &mut NewDevice<'_>,
    (person, presses): (Person<'_>, &Presses),
    intro: &str,
    mut tick: impl FnMut(&mut NewDevice<'_>) -> Result<bool, sign_in::Error>,
) -> Result<Shown, sign_in::Error> {
    let lifetime = lifetime()?;
    println!("{intro}");
    loop {
        presses.drain();
        let code = LinkCode::generate();
        let ends = Instant::now() + lifetime;
        let mut run = new.open(&code)?;
        person
            .show(Some(&format!("  {}", run.shown)))
            .map_err(sign_in::Error::Io)?;
        println!("{}", works_for(lifetime, person));
        let mut failed = 0;
        let mut code_left = true;
        loop {
            if tick(new)? {
                if code_left {
                    new.abort(&run);
                }
                return Ok(Shown::Stopped);
            }
            match presses.look() {
                Pressed::Once => break,
                Pressed::Ended if !code_left => {
                    println!("{NO_FRESH_CODE}");
                    return Ok(Shown::Stopped);
                }
                Pressed::Not | Pressed::Ended => {}
            }
            if !code_left {
                thread::sleep(POLL);
                continue;
            }
            if Instant::now() >= ends {
                new.abort(&run);
                println!("{EXPIRED} {}", person.fresh_code());
                code_left = false;
                continue;
            }
            match new.step(&mut run)? {
                None => thread::sleep(POLL),
                Some(Run::Granted {
                    session,
                    master,
                    cert,
                    link,
                }) => {
                    let linked = new.granted(master, cert, link);
                    if linked.is_err() {
                        // The issuing device stops waiting for the certificate.
                        abort(new.connection, &run.way, session);
                    }
                    return linked;
                }
                Some(Run::Failed) => {
                    failed += 1;
                    println!("{}", failed_line(failed));
                    if failed == ATTEMPTS {
                        println!("{THREE_TIMES} {}", person.fresh_code());
                        code_left = false;
                        continue;
                    }
                    run = reopen(new, person, &code, &run)?;
                }
                Some(Run::Ended) => {
                    thread::sleep(POLL);
                    run = reopen(new, person, &code, &run)?;
                }
            }
        }
        if code_left {
            new.abort(&run);
        }
    }
}

/// Opens the next run of `code` after `ended`, showing `person` the code again if it changed: a run
/// without a session has a pairing of its own.
fn reopen(
    new: &mut NewDevice<'_>,
    person: Person<'_>,
    code: &LinkCode,
    ended: &OpenRun,
) -> Result<OpenRun, sign_in::Error> {
    let run = new.open(code)?;
    if run.shown != ended.shown {
        println!("{NEW_CODE}");
        person
            .show(Some(&format!("  {}", run.shown)))
            .map_err(sign_in::Error::Io)?;
    }
    Ok(run)
}

/// What the new device says while it waits for linking.
pub const NOT_LINKED: &str =
    "Not linked yet. Run skald link on a linked device and type this code there:";
/// What it says once a code expired, and after its third failed run.
const EXPIRED: &str = "The link code expired.";
/// What it says before the code of a run without a session that replaced an ended one.
const NEW_CODE: &str = "The link code is now:";
const THREE_TIMES: &str = "The code failed 3 times; if not all were you, someone else tried.";
/// What it says once its person's input ended with no code left.
const NO_FRESH_CODE: &str = "No fresh code asked for. Run skald sign-in again for one.";

/// How long a code shown to `person` works, and how to get a fresh one.
fn works_for(lifetime: Duration, person: Person<'_>) -> String {
    format!(
        "It works for {}. {}",
        shown_lifetime(lifetime),
        person.fresh_code()
    )
}

/// What the new device says of its `failed`th failed run.
fn failed_line(failed: u32) -> String {
    format!("A run of the link code failed ({failed} of {ATTEMPTS}).")
}

impl NewDevice<'_> {
    /// Opens a run of `code` with a fresh `session`: with a session, by its hello as its account's;
    /// without one, by asking the server for the run, whose pairing it then sends its hello with.
    fn open(&mut self, code: &LinkCode) -> Result<OpenRun, sign_in::Error> {
        let session = link_session();
        let device = self.device.id.to_string();
        if !self.sessionless {
            let token = self.token.clone().unwrap_or_default();
            let account = self.account.ok_or_else(|| {
                sign_in::Error::Unexpected("a run with a session of no account".to_owned())
            })?;
            let (initiator, share) = code.initiate(account, &session);
            let hello = LinkMessage::Hello { session, share }.to_cbor();
            let request = serde_json::json!({
                "device": device,
                "hello": URL_SAFE_NO_PAD.encode(hello),
            })
            .to_string();
            self.connection
                .send_json_as(Method::Post, HELLO_PATH, &token, request.as_bytes())?;
            return Ok(OpenRun {
                session,
                way: Way::Session(token),
                initiator: Some(initiator),
                isk: None,
                shown: code.to_string(),
            });
        }
        let request = serde_json::json!({ "device": device }).to_string();
        let opened = (self.connection).post_json(SESSIONLESS_PATH, request.as_bytes())?;
        let run: Option<SessionlessRun> = serde_json::from_slice(&opened).ok();
        let (pairing, token) = run
            .and_then(|run| Some((LinkPairing::from_entry(&run.pairing).ok()?, run.token)))
            .ok_or_else(|| {
                sign_in::Error::Unexpected("a link run without its pairing and token".to_owned())
            })?;
        let (initiator, share) = code.initiate_paired(&pairing, &session);
        let way = Way::Token(token);
        send(
            self.connection,
            &way,
            &LinkMessage::Hello { session, share },
        )?;
        Ok(OpenRun {
            session,
            way,
            initiator: Some(initiator),
            isk: None,
            shown: code.shown_with(&pairing),
        })
    }

    /// Takes the checked grant of `master`, `cert` and `link`, in CBOR: with a session, stores it
    /// ([`store`]); without one, keeps it as unchecked and gets a device session by its key, which
    /// has the server store the certificate.
    fn granted(
        &mut self,
        master: [u8; 32],
        cert: Vec<u8>,
        link: Vec<u8>,
    ) -> Result<Shown, sign_in::Error> {
        if let Some(token) = self.token.clone() {
            store(self.connection, self.state, &token, master, cert, link)?;
            return Ok(Shown::Linked);
        }
        self.state.set_link(&Linked::Unchecked {
            master,
            cert: cert.clone(),
            link,
        })?;
        by_key(self.connection, self.keys, &cert).map(Shown::ByKey)
    }

    /// Ends `run`, if it is still open.
    fn abort(&mut self, run: &OpenRun) {
        abort(self.connection, &run.way, run.session);
    }

    /// One read of `run`'s messages: once the first reply comes, the request; once the grant
    /// comes, the grant, checked. `None` while the run goes on.
    fn step(&mut self, run: &mut OpenRun) -> Result<Option<Run>, sign_in::Error> {
        let session = run.session;
        let Some(messages) = messages(self.connection, &run.way, &session)? else {
            return Ok(Some(if run.isk.is_some() {
                Run::Failed
            } else {
                Run::Ended
            }));
        };
        match &run.isk {
            None => {
                let reply = messages.iter().find_map(|message| match message {
                    LinkMessage::Reply { share, .. } => Some(*share),
                    _ => None,
                });
                let Some(share) = reply else {
                    return Ok(None);
                };
                let initiator = run.initiator.take().expect("one reply per run");
                let Ok(taken) = initiator.finish(&share, &[]) else {
                    self.abort(run);
                    return Ok(Some(Run::Failed));
                };
                let body = LinkRequestBody {
                    kind: DeviceKind::Client,
                    signing: self.keys.signing().public_key(),
                    hpke: self.keys.hpke().public_key(),
                    name: self.device.name.clone(),
                    unknown: Unknown::default(),
                };
                let sealed = taken.request_key().seal(&body.to_cbor());
                let sent = send(
                    self.connection,
                    &run.way,
                    &LinkMessage::Request { session, sealed },
                );
                run.isk = Some(taken);
                match sent {
                    Ok(()) => Ok(None),
                    Err(connection::Error::Refused {
                        status: 401 | 404, ..
                    }) => Ok(Some(Run::Failed)),
                    Err(err) => Err(err.into()),
                }
            }
            Some(taken) => {
                let grant = messages.iter().find_map(|message| match message {
                    LinkMessage::Grant { sealed, .. } => Some(sealed),
                    _ => None,
                });
                let Some(sealed) = grant else {
                    return Ok(None);
                };
                Ok(Some(match self.check(taken, sealed) {
                    Ok((master, cert, link)) => Run::Granted {
                        session,
                        master,
                        cert,
                        link,
                    },
                    Err(_) => {
                        self.abort(run);
                        Run::Failed
                    }
                }))
            }
        }
    }

    /// The grant `sealed` of the run keyed by `isk`, if it opens and `cert` holds this device's
    /// account, ID, kind, keys and `name`; with a session, also if `signature` verifies with the
    /// certificate of its `issuer`, which passes checks 1 and 2 with its `master`, and the link
    /// object passes its check ("Link code", the new device's step 3): the master key, the
    /// device's complete certificate, with its `proof`, and the link object, in CBOR.
    fn check(&mut self, isk: &Isk, sealed: &[u8]) -> Result<Checked, String> {
        let opened: Zeroizing<Vec<u8>> = isk.grant_key().open(sealed).map_err(text)?;
        let grant = LinkGrantBody::from_cbor(&opened).map_err(text)?;
        let body = CertBody::from_cbor(&grant.cert).map_err(text)?;
        let keys = self.keys;
        let ours = self.account.is_none_or(|account| body.account == account)
            && body.device == self.device.id
            && body.kind == DeviceKind::Client
            && body.signing == keys.signing().public_key()
            && body.hpke == keys.hpke().public_key()
            && body.name == self.device.name
            && body.issuer.is_some();
        if !ours {
            return Err("the certificate is not this device's".to_owned());
        }
        let proof = keys.signing().sign(PROOF_LABEL, &[&grant.cert]);
        let cert = Cert::from_parts(grant.cert.clone(), grant.signature, proof).map_err(text)?;
        let (cert, link) = (cert.to_cbor(), grant.link.to_cbor());
        if let Some(token) = &self.token {
            check_granted(
                self.connection,
                self.state,
                token,
                keys,
                (&grant.master, &cert, &link),
            )?;
        }
        Ok((grant.master, cert, link))
    }
}

/// The token of a device session of the device of `keys` by its key and its complete certificate
/// `cert`, in CBOR, which the server stores if it holds none.
pub fn by_key(
    connection: &mut Connection,
    keys: &Keys,
    cert: &[u8],
) -> Result<String, sign_in::Error> {
    connection
        .device_key_session(keys.id(), keys.signing(), cert)
        .map_err(limited)
}

/// `err`, a refusal of a certificate, as the devices limit if it says so.
fn limited(err: connection::Error) -> sign_in::Error {
    match err {
        connection::Error::Refused { kind, .. } if kind == DEVICES_LIMIT => {
            device::Error::DevicesLimit.into()
        }
        err => err.into(),
    }
}

/// A checked grant's master key, certificate and link object, in CBOR.
type Checked = ([u8; 32], Vec<u8>, Vec<u8>);

/// Checks, with the session `token` of the device of `keys`, that the issuer of its granted
/// certificate `cert`, in CBOR, passes checks 1 and 2 with `master` and is a `client` device of
/// its account, and that the link object `link` passes its check ("Link code", the new device's
/// step 3).
pub fn check_granted(
    connection: &mut Connection,
    state: &State,
    token: &str,
    keys: &Keys,
    grant: (&[u8; 32], &[u8], &[u8]),
) -> Result<(), String> {
    let certs = devices::fetch(connection, "/devices", token, state)?.certs;
    check_listed(certs, keys, grant)
}

/// [`check_granted`] against `certs`, the certificates the account's device list gives.
pub fn check_listed(
    mut certs: Vec<Cert>,
    keys: &Keys,
    (master, cert, link): (&[u8; 32], &[u8], &[u8]),
) -> Result<(), String> {
    let cert = Cert::from_cbor(cert).map_err(text)?;
    let link = Link::from_cbor(link).map_err(text)?;
    let account = cert.body().account;
    certs.push(cert);
    check_device(master, &certs, keys.id()).map_err(text)?;
    let issuer = check_device(master, &certs, link.body().issuer)
        .map_err(text)?
        .body();
    if issuer.kind != DeviceKind::Client || issuer.account != account {
        return Err("the link object's issuer is no client device of this account".to_owned());
    }
    check_link(&link, keys.id(), &issuer.signing).map_err(text)
}

/// How long a code lives: [`LIFETIME`], or as [`LIFETIME_SETTING`] says.
fn lifetime() -> Result<Duration, sign_in::Error> {
    match env::var(LIFETIME_SETTING) {
        Ok(seconds) => seconds
            .parse::<u64>()
            .ok()
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .ok_or_else(|| {
                sign_in::Error::Io(format!(
                    "{LIFETIME_SETTING} is not a number of seconds above 0"
                ))
            }),
        Err(_) => Ok(LIFETIME),
    }
}

/// `lifetime` as a person reads it.
fn shown_lifetime(lifetime: Duration) -> String {
    match lifetime.as_secs() {
        seconds if seconds % 60 == 0 => format!("{} minutes", seconds / 60),
        seconds => format!("{seconds} seconds"),
    }
}

/// `PUT /devices/{device}/cert`: has the server store `cert`, in CBOR, the certificate of the
/// device of `keys`; the same certificate again is taken too.
pub fn upload(
    connection: &mut Connection,
    token: &str,
    device: DeviceId,
    cert: &[u8],
) -> Result<(), sign_in::Error> {
    let request = serde_json::json!({ "cert": URL_SAFE_NO_PAD.encode(cert) }).to_string();
    let path = format!("/devices/{device}/cert");
    connection
        .send_json_as(Method::Put, &path, token, request.as_bytes())
        .map(drop)
        .map_err(limited)
}

/// Keeps the grant of `master`, `cert` and `link`, in CBOR, in `state`, then has the server store
/// the certificate and, once it did, pins the master key and starts from the link object
/// ([`linked`]). A grant the server does not take waits in the state for a later run.
pub fn store(
    connection: &mut Connection,
    state: &State,
    token: &str,
    master: [u8; 32],
    cert: Vec<u8>,
    link: Vec<u8>,
) -> Result<(), sign_in::Error> {
    state.set_link(&Linked::Granted {
        master,
        cert: cert.clone(),
        link: link.clone(),
    })?;
    let parsed = Cert::from_cbor(&cert)
        .map_err(|err| state::Error::Malformed(format!("the granted certificate: {err}")))?;
    let body = parsed.body();
    upload(connection, token, body.device, &cert)?;
    linked(connection, state, token, master, cert, &link)?;
    println!(
        "Linked by the device {}.",
        body.issuer
            .map_or_else(String::new, |issuer| issuer.to_string())
    );
    Ok(())
}

/// Once the server stored the certificate `cert` of a grant: starts each space of the link object
/// `link` from the revision it names, pins its personal space, then pins `master` and holds the
/// certificate. A space whose revision the server does not give is warned of and skipped.
fn linked(
    connection: &mut Connection,
    state: &State,
    token: &str,
    master: [u8; 32],
    cert: Vec<u8>,
    link: &[u8],
) -> Result<(), sign_in::Error> {
    let link = Link::from_cbor(link)
        .map_err(|err| state::Error::Malformed(format!("the granted link object: {err}")))?;
    let body = link.body();
    for (space, hash) in &body.anchors {
        let given = follow::given(connection, token, *space).map_err(sign_in::Error::Io)?;
        let Some(revision) = given
            .iter()
            .find(|revision| revision.hash() == *hash && revision.body().space == *space)
        else {
            eprintln!("skald: warning: no start revision of the space {space}: not following it");
            continue;
        };
        state.add_linked_space(
            *space,
            revision.body().name.as_str(),
            &revision.to_cbor(),
            body.personal == Some(*space),
        )?;
    }
    state.set_link(&Linked::Certified { master, cert })?;
    Ok(())
}

/// At a later run of the machine whose state is in `dir`, its secrets in `storage`: has the server
/// store the certificate of a grant its device keeps, saying so, and then keeps its KeyPackages
/// and, unless `person` is false, its anchors object; warns of what fails.
pub fn finish(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    person: bool,
) {
    if !dir.join(state::FILE).is_file() {
        return;
    }
    let Ok(state) = State::open(dir) else {
        return;
    };
    let Ok(Some(state::Device {
        link: Linked::Granted { master, cert, link },
        ..
    })) = state.device()
    else {
        return;
    };
    let finished = (|| -> Result<(), String> {
        let (state, server, token, stored, secrets) = signed_in_with_secrets(dir, storage)?;
        let mut connection = to_session(&server, speaks, timeout)?;
        store(&mut connection, &state, &token, master, cert, link).map_err(text)?;
        let device = state.device().map_err(text)?;
        let keys = device::known_keys(&stored, device.as_ref())
            .map_err(text)?
            .ok_or_else(|| device::Error::MissingKeys.to_string())?;
        if let Err(err) =
            key_packages::keep(&mut connection, &token, (&state, &secrets), &keys, false)
        {
            key_packages::warn(&err);
        }
        Ok(())
    })();
    match finished {
        Ok(()) if person => spaces::take_personal(dir, storage, speaks, timeout),
        Ok(()) => {}
        Err(err) => eprintln!("skald: warning: not linked yet: {err}"),
    }
}

/// What `skald link` says of a run that failed, and when no run has its code.
const RUN_FAILED: &str = "the code failed: wrong, expired or tampered with; nothing is linked";
const NO_RUN: &str = "no new device shows this code now; nothing is sent";

/// A step of `skald link` after its person's answers, which its process hands to the helper or
/// runs here holding the local state's lock, as other commands do (`docs/dev/architecture.md`,
/// "Local state"). A run between `Open` and `Answer` waits in the process that ran `Open`.
#[derive(Serialize, Deserialize)]
pub enum Step {
    /// Checks that the device holds a valid certificate that is not revoked.
    Check,
    /// Zeroes the count of failed runs if `fresh`, runs `entry`, a link code, once against the
    /// account's latest hello and shows the new device's request; the run waits as `run`.
    Open {
        run: String,
        entry: String,
        fresh: bool,
    },
    /// Grants the new device of the run `run` its certificate if `confirmed`, waits for the server
    /// to store it and adds the new device to this device's channels ([`Issuer::add`]); otherwise
    /// ends the run.
    Answer { run: String, confirmed: bool },
}

/// A run of `skald link` between its steps `Open` and `Answer`: its session and ISK, and the new
/// device's request.
struct Waiting {
    session: [u8; 16],
    isk: Isk,
    request: LinkRequestBody,
}

/// The runs of `skald link` in this process: the one that waits for its person's answer, by its
/// name, and the names of those a later run took the place of.
struct Runs<T> {
    waiting: Option<(String, T)>,
    replaced: Vec<String>,
}

/// What an answer finds of the run it names.
#[derive(Debug, PartialEq)]
enum Found<T> {
    /// The run, which no longer waits.
    Run(T),
    /// A later run took its place.
    Replaced,
    /// None.
    Gone,
}

impl<T> Runs<T> {
    const fn new() -> Self {
        Runs {
            waiting: None,
            replaced: Vec::new(),
        }
    }

    /// Has the run `run`, holding `held`, wait; gives what the run it takes the place of held.
    fn wait(&mut self, run: String, held: T) -> Option<T> {
        let (earlier, held) = self.waiting.replace((run, held))?;
        self.replaced.push(earlier);
        Some(held)
    }

    /// What the answer naming `run` finds; a run it does not name waits on.
    fn take(&mut self, run: &str) -> Found<T> {
        if self.waiting.as_ref().is_some_and(|(name, _)| name == run) {
            return self
                .waiting
                .take()
                .map_or(Found::Gone, |(_, held)| Found::Run(held));
        }
        match self.replaced.iter().position(|name| name == run) {
            Some(at) => {
                self.replaced.swap_remove(at);
                Found::Replaced
            }
            None => Found::Gone,
        }
    }
}

thread_local! {
    /// The runs of `skald link` in this process.
    static RUNS: RefCell<Runs<Waiting>> = const { RefCell::new(Runs::new()) };
}

/// `skald link` on the machine whose state is in `dir`, its person at a terminal or on the page:
/// takes the person's answers in this process and has `step` run each step that follows them
/// ([`Step`]), stopping at the first that fails; gives the exit code.
pub fn run(dir: &Path, person: Person<'_>, mut step: impl FnMut(Step) -> ExitCode) -> ExitCode {
    if matches!(person, Person::Terminal) && !at_terminal() {
        return crate::exit(Err(
            "skald link needs a terminal, or --browser for a local page".to_owned(),
        ));
    }
    let checked = step(Step::Check);
    if checked != ExitCode::SUCCESS {
        return checked;
    }
    let (entry, fresh) = match take_code(dir, person) {
        Ok(taken) => taken,
        Err(err) => return crate::exit(Err(err)),
    };
    let run = URL_SAFE_NO_PAD.encode(link_session());
    let opened = step(Step::Open {
        run: run.clone(),
        entry,
        fresh,
    });
    if opened != ExitCode::SUCCESS {
        return opened;
    }
    // An answer that cannot be read is no yes.
    let confirmed = matches!(person.ask("Link it to this account?"), Ok(Some(true)));
    step(Step::Answer { run, confirmed })
}

/// The link code `person` enters, on the machine whose state is in `dir`, checked for its form,
/// and whether the person confirmed the new device shows a new code after 3 failed runs in a row.
fn take_code(dir: &Path, person: Person<'_>) -> Result<(String, bool), String> {
    let failed = State::open(dir)
        .and_then(|state| state.failed_link_runs())
        .map_err(text)?;
    let fresh = failed >= ATTEMPTS;
    if fresh {
        println!("The last {ATTEMPTS} runs failed: get a new code on the new device first.");
        let shows = person.ask("Does the new device show a new code?")?;
        if shows != Some(true) {
            return Err("no new code, so no code is run".to_owned());
        }
    }
    println!("Enter only a code from your own device: it joins this account.");
    let entry = (person.enter("Link code", "Link")?).ok_or_else(|| "no code entered".to_owned())?;
    let entry = entry.trim();
    LinkEntry::from_entry(entry).map_err(|_| {
        format!(
            "\"{}\" is not a link code: 6 or 12 characters of A-Z and 2-7",
            printable(entry)
        )
    })?;
    Ok((entry.to_owned(), fresh))
}

/// Runs `step` of `skald link` on the machine whose state is in `dir`, its secrets in `storage`.
pub fn step(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    step: Step,
) -> Result<(), String> {
    let mut issuer = Issuer::of(dir, storage, speaks, timeout)?;
    match step {
        Step::Check => Ok(()),
        Step::Open { run, entry, fresh } => {
            if fresh {
                issuer.state.set_failed_link_runs(0).map_err(text)?;
            }
            let entry = LinkEntry::from_entry(&entry)
                .map_err(|_| "the entry is not a link code; no run starts".to_owned())?;
            issuer.open(run, &entry)
        }
        Step::Answer { run, confirmed } => {
            let waiting = match RUNS.with(|runs| runs.borrow_mut().take(&run)) {
                Found::Run(waiting) => waiting,
                Found::Replaced => {
                    return Err(
                        "another skald link replaced this one; nothing is linked".to_owned()
                    );
                }
                Found::Gone => {
                    return Err(
                        "the helper exited, so the run is gone; nothing is linked".to_owned()
                    );
                }
            };
            issuer.answer(&waiting, confirmed)?;
            issuer.add(
                dir,
                storage,
                DeviceId::from_signing_key(&waiting.request.signing),
            );
            Ok(())
        }
    }
}

/// The issuing device: its local state, session and connection, its pinned master key, its
/// certificate and its keys, and how long it waits for the new device.
struct Issuer {
    connection: Connection,
    state: State,
    token: String,
    /// How it reaches its runs: by its session.
    way: Way,
    master: [u8; 32],
    cert: Cert,
    keys: Keys,
    speaks: VersionRange,
    timeout: Duration,
}

impl Issuer {
    /// The issuing device of the machine whose state is in `dir`, its secrets in `storage`, once
    /// it holds a valid certificate that is not revoked.
    fn of(
        dir: &Path,
        storage: secrets::Storage,
        speaks: VersionRange,
        timeout: Duration,
    ) -> Result<Self, String> {
        let (state, server, token, stored, _) = signed_in_with_secrets(dir, storage)?;
        let device = state.device().map_err(text)?;
        let Some(
            this @ state::Device {
                link: Linked::Certified { master, cert },
                ..
            },
        ) = &device
        else {
            return Err("this device is not linked, so it links no device".to_owned());
        };
        let cert = Cert::from_cbor(cert)
            .map_err(|err| state::Error::Malformed(format!("the device's certificate: {err}")))
            .map_err(text)?;
        let keys = device::known_keys(&stored, Some(this))
            .map_err(text)?
            .ok_or_else(|| device::Error::MissingKeys.to_string())?;
        let master = *master;
        let mut connection = to_session(&server, speaks, timeout)?;
        revocations::refuse_if_revoked(&mut connection, &token, &state, (&master, &cert))?;
        Ok(Issuer {
            connection,
            state,
            way: Way::Session(token.clone()),
            token,
            master,
            cert,
            keys,
            speaks,
            timeout,
        })
    }

    /// Runs `entry` once against the account's latest hello, or that of its pairing, and shows
    /// the new device's request; the run then waits as `run` for its person's answer, and the run
    /// that waited, if any, ends.
    fn open(&mut self, run: String, entry: &LinkEntry) -> Result<(), String> {
        let Some(LinkMessage::Hello { session, share }) = self.hello(entry)? else {
            return Err(NO_RUN.to_owned());
        };
        let (isk, request) = self
            .request(entry, session, &share)
            .map_err(|ended| self.ended(session, ended))?;
        println!(
            "Link {} ({} device, {})?",
            printable(&request.name),
            devices::kind_name(request.kind),
            DeviceId::from_signing_key(&request.signing)
        );
        let waiting = Waiting {
            session,
            isk,
            request,
        };
        if let Some(replaced) = RUNS.with(|runs| runs.borrow_mut().wait(run, waiting)) {
            abort(&mut self.connection, &self.way, replaced.session);
        }
        Ok(())
    }

    /// The hello of the run `entry` names, if one is open: the account's latest, or that of the
    /// entry's pairing.
    fn hello(&mut self, entry: &LinkEntry) -> Result<Option<LinkMessage>, String> {
        let text = match entry {
            LinkEntry::Code(_) => {
                let answer = (self.connection)
                    .get_as("/account/links/hello", &self.token)
                    .map_err(text)?;
                let hello: Hello = serde_json::from_slice(&answer)
                    .map_err(|_| "unexpected answer from the server: no hello".to_owned())?;
                hello.hello
            }
            LinkEntry::Paired { pairing, .. } => {
                let request = serde_json::json!({ "pairing": pairing.as_str() }).to_string();
                let answer = self.connection.send_json_as(
                    Method::Post,
                    PAIRING_PATH,
                    &self.token,
                    request.as_bytes(),
                );
                let answer = match answer {
                    Ok(answer) => answer,
                    Err(connection::Error::Refused { status: 404, .. }) => return Ok(None),
                    Err(err) => return Err(err.to_string()),
                };
                let hello: PairedHello = serde_json::from_slice(&answer)
                    .map_err(|_| "unexpected answer from the server: no hello".to_owned())?;
                Some(hello.hello)
            }
        };
        Ok(text
            .and_then(|text| URL_SAFE_NO_PAD.decode(text).ok())
            .and_then(|bytes| LinkMessage::from_cbor(&bytes).ok()))
    }

    /// Answers the hello of `session` with its `share` by `entry` and takes the new device's
    /// request, with the run's ISK.
    fn request(
        &mut self,
        entry: &LinkEntry,
        session: [u8; 16],
        share: &[u8; 32],
    ) -> Result<(Isk, LinkRequestBody), Ended> {
        let failed = |why: &str| Ended::Failed(why.to_owned());
        let responded = match entry {
            LinkEntry::Code(code) => code.respond(self.cert.body().account, &session, share),
            LinkEntry::Paired { pairing, code } => code.respond_paired(pairing, &session, share),
        };
        let (reply, isk) = responded.map_err(|_| failed(RUN_FAILED))?;
        send(
            &mut self.connection,
            &self.way,
            &LinkMessage::Reply {
                session,
                share: reply,
            },
        )
        .map_err(|err| Ended::Failed(err.to_string()))?;
        let sealed = self.request_of(session)?;
        let opened = isk
            .request_key()
            .open(&sealed)
            .map_err(|_| failed(RUN_FAILED))?;
        let request = LinkRequestBody::from_cbor(&opened).map_err(|_| failed(RUN_FAILED))?;
        Ok((isk, request))
    }

    /// Grants the new device of `waiting` its certificate if `confirmed` and waits for the server
    /// to store it; otherwise ends the run.
    fn answer(&mut self, waiting: &Waiting, confirmed: bool) -> Result<(), String> {
        let Waiting {
            session, request, ..
        } = waiting;
        let device = DeviceId::from_signing_key(&request.signing);
        let outcome = if confirmed {
            self.grant(&waiting.isk, *session, device, request)
                .and_then(|()| self.wait_for(*session, device, &request.name))
        } else {
            Err(Ended::Declined(
                "not confirmed: nothing is linked".to_owned(),
            ))
        };
        match outcome {
            Ok(()) => self.state.set_failed_link_runs(0).map_err(text),
            Err(ended) => Err(self.ended(*session, ended)),
        }
    }

    /// Ends the run `session` as `ended` says, counting it among the failed runs in a row if it
    /// failed; gives why it ended.
    fn ended(&mut self, session: [u8; 16], ended: Ended) -> String {
        abort(&mut self.connection, &self.way, session);
        match ended {
            Ended::Failed(why) => {
                let counted = (self.state.failed_link_runs())
                    .and_then(|failed| self.state.set_failed_link_runs(failed + 1));
                match counted {
                    Ok(()) => why,
                    Err(err) => format!("{why}; the failed run is not counted: {err}"),
                }
            }
            Ended::Declined(why) => why,
        }
    }

    /// The sealed request of the run `session`, once the new device sent it within the timeout.
    fn request_of(&mut self, session: [u8; 16]) -> Result<Vec<u8>, Ended> {
        let ends = Instant::now() + self.timeout;
        loop {
            let messages = messages(&mut self.connection, &self.way, &session)
                .map_err(|err| Ended::Failed(err.to_string()))?
                .ok_or_else(|| Ended::Failed(RUN_FAILED.to_owned()))?;
            let request = messages.into_iter().find_map(|message| match message {
                LinkMessage::Request { sealed, .. } => Some(sealed),
                _ => None,
            });
            if let Some(sealed) = request {
                return Ok(sealed);
            }
            if Instant::now() >= ends {
                return Err(Ended::Declined(format!(
                    "no request from the new device in {} seconds; nothing is linked",
                    self.timeout.as_secs()
                )));
            }
            thread::sleep(POLL);
        }
    }

    /// Issues the certificate of `device` for `request`, signs the link object of the revisions
    /// this device accepted and its personal space, and sends both with the master key, sealed by
    /// the grant key of `isk`.
    fn grant(
        &mut self,
        isk: &Isk,
        session: [u8; 16],
        device: DeviceId,
        request: &LinkRequestBody,
    ) -> Result<(), Ended> {
        let failed = |why: String| Ended::Failed(why);
        let this = self.cert.body().device;
        let body = CertBody {
            account: self.cert.body().account,
            device,
            kind: request.kind,
            signing: request.signing,
            hpke: request.hpke,
            issuer: Some(this),
            issued: now(),
            ssh: None,
            github: None,
            name: request.name.clone(),
            unknown: Unknown::default(),
        };
        let cert = body.to_cbor();
        let signature = self.keys.signing().sign(CERT_LABEL, &[&cert]);
        let (anchors, personal) = crate::anchors::held(&self.state).map_err(failed)?;
        let link = sign_link(
            &LinkBody {
                device,
                issuer: this,
                anchors,
                personal,
                unknown: Unknown::default(),
            },
            self.keys.signing(),
        )
        .map_err(|err| failed(err.to_string()))?;
        let grant = LinkGrantBody {
            master: self.master,
            cert,
            signature,
            link,
            unknown: Unknown::default(),
        };
        let sealed = isk.grant_key().seal(&grant.to_cbor());
        send(
            &mut self.connection,
            &self.way,
            &LinkMessage::Grant { session, sealed },
        )
        .map_err(|err| match err {
            connection::Error::Refused { status: 404, .. } => failed(RUN_FAILED.to_owned()),
            err => failed(err.to_string()),
        })
    }

    /// Waits, within the timeout, for the run `session` to end, then for the server to hold the
    /// certificate of `device`, named `name`.
    fn wait_for(&mut self, session: [u8; 16], device: DeviceId, name: &str) -> Result<(), Ended> {
        let failed = |err: String| Ended::Failed(err);
        let ends = Instant::now() + self.timeout;
        while messages(&mut self.connection, &self.way, &session)
            .map_err(|err| failed(err.to_string()))?
            .is_some()
        {
            if Instant::now() >= ends {
                return Err(Ended::Declined(format!(
                    "the new device stored no certificate in {} seconds: it is not linked yet",
                    self.timeout.as_secs()
                )));
            }
            thread::sleep(POLL);
        }
        let held = devices::fetch(&mut self.connection, "/devices", &self.token, &self.state)
            .map_err(failed)?;
        let stored = check_device(&self.master, &held.certs, device).is_ok();
        if !stored {
            return Err(failed(
                "the run ended with no certificate stored; nothing is linked".to_owned(),
            ));
        }
        println!("Linked the device {} ({device}).", printable(name));
        Ok(())
    }

    /// Once the new device `device`, of the machine whose state is in `dir`, its secrets in
    /// `storage`, has KeyPackages on the server, within the timeout, adds it to this device's
    /// channels, as a later run would; otherwise leaves that to a later run of this device or of
    /// the new one. Warns of what fails.
    fn add(&mut self, dir: &Path, storage: secrets::Storage, device: DeviceId) {
        let not_added = "skald: warning: the new device is not in the channels yet";
        let follower = spaces::following(dir, storage, self.speaks, self.timeout, Scope::All);
        let mut follower = match follower {
            Ok(follower) => follower,
            Err(err) => {
                eprintln!("{not_added}: {err}");
                return;
            }
        };
        let ends = Instant::now() + self.timeout;
        loop {
            match key_packages::held(&mut self.connection, &self.token, device) {
                Ok(true) => break,
                Ok(false) if Instant::now() < ends => thread::sleep(POLL),
                Ok(false) => {
                    println!(
                        "The new device has no KeyPackages yet: it joins the channels at a later \
                         run."
                    );
                    return;
                }
                Err(err) => {
                    eprintln!("{not_added}: {err}");
                    return;
                }
            }
        }
        let spaces = match follower.state.spaces() {
            Ok(spaces) => spaces,
            Err(err) => {
                eprintln!("skald: warning: {err}");
                return;
            }
        };
        for space in spaces {
            if let Err(err) = follower.sync(&space) {
                eprintln!("skald: warning: {err}");
            }
        }
        groups::align_accepted(&mut follower);
    }
}

/// How a step of `skald link` failed: `Failed` counts among the failed runs in a row; `Declined`,
/// by its person or for want of an answer from the new device, does not.
enum Ended {
    Failed(String),
    Declined(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer that names another run finds nothing, so nothing is granted, and the waiting run
    /// waits for its own answer.
    #[test]
    fn an_answer_takes_only_the_run_it_names() {
        let mut runs = Runs::new();
        assert_eq!(runs.take("first"), Found::Gone);
        assert!(runs.wait("first".to_owned(), 1).is_none());
        assert_eq!(runs.take("other"), Found::Gone);
        assert_eq!(runs.take("first"), Found::Run(1));
        assert_eq!(runs.take("first"), Found::Gone);
    }

    /// A run that takes the place of a waiting one gives it back to be ended; the answer of the
    /// replaced run finds that it was replaced, once.
    #[test]
    fn a_later_run_takes_the_place_of_the_waiting_one() {
        let mut runs = Runs::new();
        assert!(runs.wait("first".to_owned(), 1).is_none());
        assert_eq!(runs.wait("second".to_owned(), 2), Some(1));
        assert_eq!(runs.take("first"), Found::Replaced);
        assert_eq!(runs.take("first"), Found::Gone);
        assert_eq!(runs.take("second"), Found::Run(2));
    }
}
