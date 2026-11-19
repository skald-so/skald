//! Linking a device with a link code (`docs/protocol/membership.md`, "Linking a device" and "Link
//! code"), only by its person at a terminal.
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
use std::io::{self, BufRead, IsTerminal as _, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_crypto::{Isk, LinkCode, check_device, check_link, link_session, sign_link};
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

/// Where a run's hello goes, and its later messages.
const HELLO_PATH: &str = "/account/links";
const MESSAGES_PATH: &str = "/account/links/messages";

/// Whether its person runs this at a terminal: both its input and its output are one.
pub fn at_terminal() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// `LinkHello` in the API spec.
#[derive(Deserialize)]
struct Hello {
    hello: Option<String>,
}

/// `LinkMessages` in the API spec.
#[derive(Deserialize)]
struct Messages {
    messages: Vec<String>,
}

/// `POST /account/links/messages` of `message`.
fn send(
    connection: &mut Connection,
    token: &str,
    message: &LinkMessage,
) -> Result<(), connection::Error> {
    let request =
        serde_json::json!({ "message": URL_SAFE_NO_PAD.encode(message.to_cbor()) }).to_string();
    connection
        .send_json_as(Method::Post, MESSAGES_PATH, token, request.as_bytes())
        .map(drop)
}

/// Ends the run `session`, if it is still open.
fn abort(connection: &mut Connection, token: &str, session: [u8; 16]) {
    let _ = send(connection, token, &LinkMessage::Abort { session });
}

/// The later messages of the open run `session` that decode, in the server's order; `None` once
/// the run ended.
fn messages(
    connection: &mut Connection,
    token: &str,
    session: &[u8; 16],
) -> Result<Option<Vec<LinkMessage>>, connection::Error> {
    let path = format!("{MESSAGES_PATH}/{}", URL_SAFE_NO_PAD.encode(session));
    let body = match connection.get_as(&path, token) {
        Ok(body) => body,
        Err(connection::Error::Refused { status: 404, .. }) => return Ok(None),
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

/// The new device, waiting for linking: its connection, local state and session, its account,
/// itself and its keys.
pub struct NewDevice<'a> {
    pub connection: &'a mut Connection,
    pub state: &'a State,
    pub token: &'a str,
    pub account: AccountId,
    pub device: &'a state::Device,
    pub keys: &'a Keys,
}

/// How a run of a code ended at the new device.
enum Run {
    /// It took a grant that passes its checks: the master key, its complete certificate and the
    /// link object, in CBOR.
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
    /// The code expired.
    Expired,
    /// Its person asked for a fresh code.
    Fresh,
}

/// Shows link codes until a run links the device, then stores its certificate ([`store`]); waits
/// for its person to ask for a fresh code once a code ends, and stops once its input ends.
pub fn show(new: &mut NewDevice<'_>) -> Result<(), sign_in::Error> {
    let lifetime = lifetime()?;
    let enters = enters();
    println!(
        "This device is not linked yet. To link it, run skald link at a terminal on a linked \
         device of this account and type this code there:"
    );
    loop {
        while enters.try_recv().is_ok() {}
        let code = LinkCode::generate();
        let ends = Instant::now() + lifetime;
        println!("  {code}");
        println!(
            "It works for {}; press Enter for a fresh code.",
            shown_lifetime(lifetime)
        );
        let mut failed = 0;
        let fresh = loop {
            match new.run(&code, ends, &enters)? {
                Run::Granted {
                    session,
                    master,
                    cert,
                    link,
                } => {
                    let stored = store(new.connection, new.state, new.token, master, cert, link);
                    if stored.is_err() {
                        // The issuing device stops waiting for the certificate.
                        abort(new.connection, new.token, session);
                    }
                    return stored;
                }
                Run::Failed => {
                    failed += 1;
                    println!("A run of the link code failed ({failed} of {ATTEMPTS}).");
                    if failed == ATTEMPTS {
                        println!(
                            "The link code failed {ATTEMPTS} times; if you did not type it each \
                             time, someone else may have tried. Press Enter for a fresh code."
                        );
                        break false;
                    }
                }
                Run::Ended => thread::sleep(POLL),
                Run::Expired => {
                    println!("The link code expired. Press Enter for a fresh code.");
                    break false;
                }
                Run::Fresh => break true,
            }
        };
        if !fresh && enters.recv().is_err() {
            println!(
                "No fresh code asked for: this device waits for linking; run skald sign-in again \
                 for a code."
            );
            return Ok(());
        }
    }
}

impl NewDevice<'_> {
    /// One run of `code`, which ends at `ends`: a hello with a fresh `session`, the first reply of
    /// the run, the request, and the grant, checked; or what ended it first.
    fn run(
        &mut self,
        code: &LinkCode,
        ends: Instant,
        enters: &Receiver<()>,
    ) -> Result<Run, sign_in::Error> {
        let session = link_session();
        let (initiator, share) = code.initiate(self.account, &session);
        let hello = LinkMessage::Hello { session, share }.to_cbor();
        let request = serde_json::json!({
            "device": self.device.id.to_string(),
            "hello": URL_SAFE_NO_PAD.encode(hello),
        })
        .to_string();
        self.connection
            .send_json_as(Method::Post, HELLO_PATH, self.token, request.as_bytes())?;
        let mut initiator = Some(initiator);
        let mut isk: Option<Isk> = None;
        loop {
            match enters.try_recv() {
                Ok(()) => {
                    abort(self.connection, self.token, session);
                    return Ok(Run::Fresh);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            }
            if Instant::now() >= ends {
                abort(self.connection, self.token, session);
                return Ok(Run::Expired);
            }
            let Some(messages) = messages(self.connection, self.token, &session)? else {
                return Ok(if isk.is_some() {
                    Run::Failed
                } else {
                    Run::Ended
                });
            };
            match &isk {
                None => {
                    let reply = messages.iter().find_map(|message| match message {
                        LinkMessage::Reply { share, .. } => Some(*share),
                        _ => None,
                    });
                    if let Some(share) = reply {
                        let initiator = initiator.take().expect("one reply per run");
                        let Ok(taken) = initiator.finish(&share, &[]) else {
                            abort(self.connection, self.token, session);
                            return Ok(Run::Failed);
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
                            self.token,
                            &LinkMessage::Request { session, sealed },
                        );
                        isk = Some(taken);
                        match sent {
                            Ok(()) => {}
                            Err(connection::Error::Refused { status: 404, .. }) => {
                                return Ok(Run::Failed);
                            }
                            Err(err) => return Err(err.into()),
                        }
                    }
                }
                Some(taken) => {
                    let grant = messages.iter().find_map(|message| match message {
                        LinkMessage::Grant { sealed, .. } => Some(sealed),
                        _ => None,
                    });
                    if let Some(sealed) = grant {
                        return Ok(match self.check(taken, sealed) {
                            Ok((master, cert, link)) => Run::Granted {
                                session,
                                master,
                                cert,
                                link,
                            },
                            Err(_) => {
                                abort(self.connection, self.token, session);
                                Run::Failed
                            }
                        });
                    }
                }
            }
            thread::sleep(POLL);
        }
    }

    /// The grant `sealed` of the run keyed by `isk`, if it opens, `cert` holds this device's
    /// account, ID, kind, keys and `name`, `signature` verifies with the certificate of its
    /// `issuer`, which passes checks 1 and 2 with its `master`, and the link object passes its
    /// check ("Link code", the new device's step 3): the master key, the device's complete
    /// certificate, with its `proof`, and the link object, in CBOR.
    fn check(&mut self, isk: &Isk, sealed: &[u8]) -> Result<Granted, String> {
        let opened: Zeroizing<Vec<u8>> = isk.grant_key().open(sealed).map_err(text)?;
        let grant = LinkGrantBody::from_cbor(&opened).map_err(text)?;
        let body = CertBody::from_cbor(&grant.cert).map_err(text)?;
        let keys = self.keys;
        let ours = body.account == self.account
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
        let state = self.state;
        let mut certs = devices::fetch(self.connection, "/devices", self.token, state)?.certs;
        certs.push(cert.clone());
        check_device(&grant.master, &certs, self.device.id).map_err(text)?;
        let issuer = check_device(&grant.master, &certs, grant.link.body().issuer)
            .map_err(text)?
            .body();
        if issuer.kind != DeviceKind::Client || issuer.account != self.account {
            return Err("the link object's issuer is no client device of this account".to_owned());
        }
        check_link(&grant.link, self.device.id, &issuer.signing).map_err(text)?;
        Ok((grant.master, cert.to_cbor(), grant.link.to_cbor()))
    }
}

/// A grant's master key, certificate and link object, in CBOR.
type Granted = ([u8; 32], Vec<u8>, Vec<u8>);

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

/// Each line its person enters, from now on, until its input ends.
fn enters() -> Receiver<()> {
    let (entered, enters) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if line.is_err() || entered.send(()).is_err() {
                return;
            }
        }
    });
    enters
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
    match connection.send_json_as(Method::Put, &path, token, request.as_bytes()) {
        Ok(_) => Ok(()),
        Err(connection::Error::Refused { kind, .. }) if kind == DEVICES_LIMIT => {
            Err(device::Error::DevicesLimit.into())
        }
        Err(err) => Err(err.into()),
    }
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
        "Linked: the server stored this device's certificate, issued by the device {}.",
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
            eprintln!(
                "skald: warning: the server does not give the revision of the space {space} \
                 this device's link starts from, so it does not follow the space"
            );
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
        Err(err) => eprintln!("skald: warning: this device is not linked yet: {err}"),
    }
}

/// What `skald link` says of a run that failed.
const RUN_FAILED: &str = "the run of the code failed: the code typed is not the one the new device \
                          shows, it expired, or someone changed the run; nothing is linked";

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

/// `skald link` at its person's terminal, on the machine whose state is in `dir`: takes the
/// person's answers in this process and has `step` run each step that follows them ([`Step`]),
/// stopping at the first that fails; gives the exit code.
pub fn run(dir: &Path, mut step: impl FnMut(Step) -> ExitCode) -> ExitCode {
    if !at_terminal() {
        return crate::exit(Err(
            "skald link takes a link code only from its person at a terminal; nothing is sent"
                .to_owned(),
        ));
    }
    let checked = step(Step::Check);
    if checked != ExitCode::SUCCESS {
        return checked;
    }
    let mut input = io::stdin().lock();
    let (entry, fresh) = match take_code(dir, &mut input) {
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
    let confirmed = matches!(
        ask(&mut input, "Link it to this account? [yes/no]: "),
        Ok(Some(true))
    );
    step(Step::Answer { run, confirmed })
}

/// The link code its person enters on `input`, on the machine whose state is in `dir`, checked
/// for its form, and whether the person confirmed the new device shows a new code after 3 failed
/// runs in a row.
fn take_code(dir: &Path, input: &mut impl BufRead) -> Result<(String, bool), String> {
    let failed = State::open(dir)
        .and_then(|state| state.failed_link_runs())
        .map_err(text)?;
    let fresh = failed >= ATTEMPTS;
    if fresh {
        println!(
            "The last {ATTEMPTS} runs of skald link failed: get a new code on the new device first."
        );
        let shows = ask(input, "Does the new device show a new code? [yes/no]: ")?;
        if shows != Some(true) {
            return Err("no new code, so no code is run".to_owned());
        }
    }
    println!(
        "Enter only a link code shown on your own device: whoever has a device that shows the \
         code gets that device into this account."
    );
    let entry = read(input, "Link code: ")?.ok_or_else(|| "no code entered".to_owned())?;
    let entry = entry.trim();
    LinkCode::from_entry(entry).map_err(|_| {
        format!(
            "\"{}\" is not a link code: 6 characters of A to Z and 2 to 7; no run starts",
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
            let code = LinkCode::from_entry(&entry)
                .map_err(|_| "the entry is not a link code; no run starts".to_owned())?;
            issuer.open(run, &code)
        }
        Step::Answer { run, confirmed } => {
            let waiting = match RUNS.with(|runs| runs.borrow_mut().take(&run)) {
                Found::Run(waiting) => waiting,
                Found::Replaced => {
                    return Err(
                        "another skald link took the place of this one, so the run of \
                         its code ended; nothing is linked"
                            .to_owned(),
                    );
                }
                Found::Gone => {
                    return Err(
                        "the run of the code is gone, as the helper of the local state exited; \
                         nothing is linked: run skald link again"
                            .to_owned(),
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
            return Err("this device holds no certificate, so it links no device".to_owned());
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
            token,
            master,
            cert,
            keys,
            speaks,
            timeout,
        })
    }

    /// Runs `code` once against the account's latest hello and shows the new device's request;
    /// the run then waits as `run` for its person's answer, and the run that waited, if any, ends.
    fn open(&mut self, run: String, code: &LinkCode) -> Result<(), String> {
        let answer = self
            .connection
            .get_as("/account/links/hello", &self.token)
            .map_err(text)?;
        let hello: Hello = serde_json::from_slice(&answer)
            .map_err(|_| "unexpected answer from the server: no hello".to_owned())?;
        let hello = hello
            .hello
            .and_then(|text| URL_SAFE_NO_PAD.decode(text).ok())
            .and_then(|bytes| LinkMessage::from_cbor(&bytes).ok());
        let Some(LinkMessage::Hello { session, share }) = hello else {
            return Err(
                "no new device of this account shows a link code now; nothing is sent".to_owned(),
            );
        };
        let (isk, request) = self
            .request(code, session, &share)
            .map_err(|ended| self.ended(session, ended))?;
        println!(
            "The new device asks to be linked as {}, a {} device, with the ID {}.",
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
            abort(&mut self.connection, &self.token, replaced.session);
        }
        Ok(())
    }

    /// Answers the hello of `session` with its `share` by `code` and takes the new device's
    /// request, with the run's ISK.
    fn request(
        &mut self,
        code: &LinkCode,
        session: [u8; 16],
        share: &[u8; 32],
    ) -> Result<(Isk, LinkRequestBody), Ended> {
        let failed = |why: &str| Ended::Failed(why.to_owned());
        let (reply, isk) = code
            .respond(self.cert.body().account, &session, share)
            .map_err(|_| failed(RUN_FAILED))?;
        send(
            &mut self.connection,
            &self.token,
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
                "not confirmed: the new device is not linked".to_owned(),
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
        abort(&mut self.connection, &self.token, session);
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
            let messages = messages(&mut self.connection, &self.token, &session)
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
                    "the new device sent no request within {} seconds; nothing is linked",
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
            &self.token,
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
        while messages(&mut self.connection, &self.token, &session)
            .map_err(|err| failed(err.to_string()))?
            .is_some()
        {
            if Instant::now() >= ends {
                return Err(Ended::Declined(format!(
                    "the server holds no certificate of the new device after {} seconds: it is \
                     linked only once the new device has the server store it",
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
                "the run ended without the new device's certificate on the server: the new \
                 device refused the grant or the server its certificate; nothing is linked"
                    .to_owned(),
            ));
        }
        println!(
            "Linked the device {device} ({}) to this account.",
            printable(name)
        );
        Ok(())
    }

    /// Once the new device `device`, of the machine whose state is in `dir`, its secrets in
    /// `storage`, has KeyPackages on the server, within the timeout, adds it to this device's
    /// channels, as a later run would; otherwise leaves that to a later run of this device or of
    /// the new one. Warns of what fails.
    fn add(&mut self, dir: &Path, storage: secrets::Storage, device: DeviceId) {
        let not_added = "skald: warning: the new device is not added to this device's channels yet";
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
                        "The new device has no KeyPackages on the server yet: a later run of this \
                         device or of the new one adds it to this account's channels."
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

/// The line its person enters on `input` after `prompt`, without its end; `None` once the input
/// ends.
fn read(input: &mut impl BufRead, prompt: &str) -> Result<Option<String>, String> {
    let mut output = io::stdout();
    let asked = write!(output, "{prompt}").and_then(|()| output.flush());
    asked.map_err(|err| format!("cannot ask: {err}"))?;
    let mut line = String::new();
    let read = input
        .read_line(&mut line)
        .map_err(|err| format!("cannot read the answer: {err}"))?;
    Ok((read > 0).then(|| line.trim_end_matches(['\r', '\n']).to_owned()))
}

/// Whether its person answers yes to `prompt` on `input`, asked again until yes or no; `None` once
/// the input ends.
fn ask(input: &mut impl BufRead, prompt: &str) -> Result<Option<bool>, String> {
    loop {
        let Some(answer) = read(input, prompt)? else {
            println!();
            return Ok(None);
        };
        match answer.trim().to_lowercase().as_str() {
            "yes" | "y" => return Ok(Some(true)),
            "no" | "n" => return Ok(Some(false)),
            _ => println!("Please answer yes or no."),
        }
    }
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
