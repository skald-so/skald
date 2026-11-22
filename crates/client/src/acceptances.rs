//! `skald accept-invite`: a person of another account accepts an invite link at a terminal
//! (`docs/protocol/membership.md`, "Invite").
//!
//! - It takes the link only from its person at a terminal, never from an argument, a pipe or the
//!   environment; the CLI refuses it as an agent (`main`). It needs a `client` device with a
//!   certificate, and refuses a malformed link and one of another server, sending nothing.
//! - The person's own process takes the invite by its ID and takes the newcomer's steps: the
//!   revision with the link's hash as its anchor, every later one the server gives accepted as
//!   [`crate::follow`] accepts them; the invite signed by an admin's device per those revisions,
//!   its `key` derived from the link's secret, `expires` not passed, and the account no member.
//!   It shows the invite, asks its person to confirm and for a free handle, and signs the
//!   acceptance with the invite key. The secret stays in that process's memory.
//! - The acceptance and the anchor go to a [`Step`], which the helper runs or this process holding
//!   the local state's lock: it writes both before it sends the acceptance, sends it again while
//!   the server does not answer, then follows the space from the anchor, which the anchors object
//!   names ([`crate::anchors`]).
//! - A person's `skald recv` ([`settle`]) sends an acceptance the server has not taken, and keeps
//!   following the space while the server holds the invite. Once a revision lists the account, the
//!   acceptance is done and the device joins the channels as any member's does
//!   ([`crate::groups`]); when the invite ends first, the device says so and follows the space no
//!   more.
//! - The server's web address, which invite links start with, is the one it names in its answers
//!   to `GET /session` and `GET /devices`, which every sign-in and every run that follows spaces
//!   get ([`record_web`]), or the origin of the server's own address.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_crypto::{InviteLink, check_invite, check_valid_device, sign_acceptance};
use skald_proto::{
    Acceptance, AcceptanceBody, Cbor as _, Cert, DeviceKind, Invite, InviteId, Name, Role, Roster,
    SpaceId, Unknown, VersionRange,
};

use crate::connection::{self, Method, origin, printable};
use crate::devices::{self, date, text};
use crate::follow::{self, Own, SpaceDevices};
use crate::invites::resent;
use crate::person::Person;
use crate::revocations;
use crate::secrets;
use crate::spaces::{self, Follower, Scope};
use crate::state::{self, State};

/// Why a link given as an argument is not taken.
pub const GIVEN: &str = "skald accept-invite takes the invite link only at the terminal, where it \
                         asks for it, never as an argument; nothing was sent";

/// Why a run whose input or output is no terminal takes no link.
pub const NOT_AT_TERMINAL: &str = "skald accept-invite takes the invite link only at a terminal, \
                                   never through a pipe or from a file; nothing was sent";

/// Why a machine without a certified `client` device accepts no invite.
const SIGN_IN_FIRST: &str = "accepting an invite needs this machine signed in, with a device its \
                             account certified: run skald sign-in first, then skald accept-invite";

/// The device's request for an invite, `GET`.
fn invite_path(invite: InviteId) -> String {
    format!("/invites/{invite}")
}

/// `InviteWithSpace` in the API spec, as far as the newcomer reads it; its devices are read as a
/// device list ([`devices::decode`]).
#[derive(Deserialize)]
struct Given {
    invite: String,
    revisions: Vec<String>,
}

/// A step of `skald accept-invite` after its person's answers, which its process hands to the
/// helper or runs here holding the local state's lock.
#[derive(Serialize, Deserialize)]
pub enum Step {
    /// Follows the space of `anchor`, the revision the link names, in CBOR, and stores
    /// `acceptance`, in CBOR.
    Accept {
        anchor: Vec<u8>,
        acceptance: Vec<u8>,
    },
}

/// What the person's process reads the state with: the server of its session, the session token,
/// the account with its pinned master key, and the state.
struct Reader {
    state: State,
    server: String,
    token: String,
    own: Own,
}

impl Reader {
    /// The reader of the machine whose state is in `dir`, its secrets in `storage`; fails unless
    /// it has a session and a certified `client` device.
    fn open(dir: &Path, storage: secrets::Storage) -> Result<Self, String> {
        let (state, server, token) =
            devices::signed_in(dir, storage).map_err(|_| SIGN_IN_FIRST.to_owned())?;
        let Some((_, master, cert)) = spaces::certified(&state)? else {
            return Err(SIGN_IN_FIRST.to_owned());
        };
        if cert.body().kind != DeviceKind::Client {
            return Err(SIGN_IN_FIRST.to_owned());
        }
        Ok(Reader {
            state,
            server,
            token,
            own: Own {
                account: cert.body().account,
                master,
            },
        })
    }

    /// Whether `web`, an invite link's web address, is the server's ([`serves`]).
    fn serves(&self, web: &str) -> Result<bool, String> {
        let recorded = self.state.web(&self.server).map_err(text)?;
        Ok(serves(&self.server, recorded.as_deref(), web))
    }
}

/// Whether `web`, an invite link's web address, is that of the server at `server`: `recorded`, the
/// one the state recorded, or the origin of `server`.
fn serves(server: &str, recorded: Option<&str>, web: &str) -> bool {
    let Some(web) = origin(web) else {
        return false;
    };
    (recorded.map(str::to_owned).into_iter())
        .chain(origin(server))
        .any(|known| known == web)
}

/// Records in `state` `web`, the web address the server at `server` names in an answer, if it is
/// an origin other than the one recorded; warns if it cannot.
pub fn record_web(state: &State, server: &str, web: Option<&str>) {
    let Some(web) = web.filter(|web| origin(web).as_deref() == Some(*web)) else {
        return;
    };
    if state.web(server).ok().flatten().as_deref() == Some(web) {
        return;
    }
    if let Err(err) = state.set_web(server, web) {
        eprintln!("skald: warning: the server's web address is not recorded: {err}");
    }
}

/// The invite, as the newcomer's device checked it: the revision the link names, the latest one
/// it accepts after it, and the handle and device name of the admin whose device signed it.
struct Checked {
    invite: Invite,
    anchor: Roster,
    latest: Roster,
    admin: String,
    device: String,
}

/// `skald accept-invite` by its person at a terminal, on the machine whose state is in `dir`, its
/// secrets in `storage`: refuses a link `given` as an argument; takes the link, checks the invite,
/// asks its person to confirm and for a handle, starting from `handle` or the account's username,
/// then has `step` run the `Accept` step ([`Step`]); gives the exit code.
pub fn run(
    (dir, storage): (&Path, secrets::Storage),
    (speaks, timeout): (VersionRange, Duration),
    (given, handle): (&[String], Option<&str>),
    step: impl FnOnce(Step) -> ExitCode,
) -> ExitCode {
    match accepted((dir, storage), (speaks, timeout), (given, handle)) {
        Ok(Some(accept)) => step(accept),
        Ok(None) => ExitCode::SUCCESS,
        Err(err) => crate::exit(Err(err)),
    }
}

/// As [`run`], up to the `Accept` step: gives it, or `None` once the person declined.
fn accepted(
    (dir, storage): (&Path, secrets::Storage),
    (speaks, timeout): (VersionRange, Duration),
    (given, handle): (&[String], Option<&str>),
) -> Result<Option<Step>, String> {
    if !given.is_empty() {
        return Err(GIVEN.to_owned());
    }
    if !crate::link_code::at_terminal() {
        return Err(NOT_AT_TERMINAL.to_owned());
    }
    let reader = Reader::open(dir, storage)?;
    let entered = Person::Terminal
        .enter("Paste the invite link", "")?
        .ok_or("the terminal's input ended before an invite link; nothing was sent")?;
    let link = InviteLink::parse(entered.trim())
        .map_err(|err| format!("that is no invite link: {err}; nothing was sent"))?;
    drop(entered);
    if !reader.serves(&link.web)? {
        return Err(format!(
            "the invite link is of another server, {}, than this device's, {}; nothing was sent",
            link.web,
            printable(&reader.server)
        ));
    }
    let mut connection = devices::to_session(&reader.server, speaks, timeout)?;
    let body = match connection.get_as(&invite_path(link.invite), &reader.token) {
        Ok(body) => body,
        Err(connection::Error::Refused { status: 404, .. }) => {
            return Err(
                "the invite ended: the server no longer holds it, as it expired, was \
                 revoked or admitted another account; nothing was signed"
                    .to_owned(),
            );
        }
        Err(err) => return Err(text(err)),
    };
    let checked = check(&reader, &link, &body)?;
    let name = checked.latest.body().name.as_str().to_owned();
    if let Some(followed) = (reader.state.spaces().map_err(text)?.into_iter())
        .find(|followed| followed.id == checked.latest.body().space)
    {
        return Err(format!(
            "this device follows the space {} already; nothing was signed",
            followed.name
        ));
    }
    println!("{}", summary(&checked));
    match Person::Terminal.ask("Accept the invite?")? {
        Some(true) => {}
        Some(false) => {
            println!("Declined: nothing was signed.");
            return Ok(None);
        }
        None => return Err("the terminal's input ended; nothing was signed".to_owned()),
    }
    let mut asked = match handle {
        Some(handle) => handle.to_owned(),
        None => spaces::handle(&mut connection, &reader.token, reader.own.account)?.to_string(),
    };
    let handle = loop {
        match free(&checked.latest, &asked) {
            Ok(handle) => break handle,
            Err(why) => {
                println!("{why}");
                asked = Person::Terminal
                    .enter("Another handle", "")?
                    .ok_or("the terminal's input ended; nothing was signed")?;
            }
        }
    };
    let body = AcceptanceBody {
        invite: link.invite,
        account: reader.own.account,
        master: reader.own.master,
        handle,
        unknown: Unknown::default(),
    };
    let acceptance = sign_acceptance(&body, &link.secret.invite_key())
        .map_err(|err| format!("the acceptance fails: {err}"))?;
    drop(link);
    println!("Accepting the invite to the space {name}.");
    Ok(Some(Step::Accept {
        anchor: checked.anchor.to_cbor(),
        acceptance: acceptance.to_cbor(),
    }))
}

/// The newcomer's steps of "Invite" on `body`, the server's answer for the invite of `link`, as
/// `reader`'s device: the invite, its revisions from the one the link names, and its signer.
fn check(reader: &Reader, link: &InviteLink, body: &[u8]) -> Result<Checked, String> {
    let unexpected = |what: &str| format!("unexpected answer from the server: {what}");
    let given: Given =
        serde_json::from_slice(body).map_err(|_| unexpected("an invite without its space"))?;
    let invite = (URL_SAFE_NO_PAD.decode(&given.invite).ok())
        .and_then(|bytes| Invite::from_cbor(&bytes).ok())
        .ok_or_else(|| unexpected("an invite that does not decode"))?;
    let space = invite.body().space;
    if invite.body().invite != link.invite {
        return Err(unexpected("another invite than the link's"));
    }
    let mut devices = devices::decode(body)?;
    devices.revocations = revocations::with_held(&reader.state, devices.revocations)?;
    let revisions: Vec<Roster> = (given.revisions.iter())
        .filter_map(|entry| URL_SAFE_NO_PAD.decode(entry).ok())
        .filter_map(|bytes| Roster::from_cbor(&bytes).ok())
        .collect();
    let anchor = (revisions.iter())
        .find(|revision| revision.hash() == link.revision && revision.body().space == space)
        .ok_or(
            "the server gives no revision of the invite's space with the hash the link names, so \
             the link was altered on its way or the server swapped the revision; nothing was \
             signed",
        )?;
    let latest = latest_from(anchor, &revisions, &devices, &reader.own)?;
    let name = latest.body().name.as_str();
    let cert = signer(&invite, &latest, &devices)?;
    if link.secret.invite_key().public_key() != invite.body().key {
        return Err(
            "the link's secret does not derive the invite's key, so the link was altered on its \
             way; nothing was signed"
                .to_owned(),
        );
    }
    if invite.body().expires <= spaces::now() {
        return Err(format!(
            "the invite expired on {} (UTC); nothing was signed",
            date(invite.body().expires)
        ));
    }
    if latest.body().member(reader.own.account).is_some() {
        return Err(format!(
            "this account is already a member of the space {name}; nothing was signed"
        ));
    }
    let admin = (latest.body().member(cert.body().account))
        .map(|member| member.handle.as_str().to_owned())
        .unwrap_or_default();
    Ok(Checked {
        device: printable(&cert.body().name),
        admin,
        anchor: anchor.clone(),
        latest,
        invite,
    })
}

/// The latest revision of `revisions`, the ones the server gives of a space, accepted after
/// `anchor` with `devices`, its accounts', as `own`'s device; fails unless it accepts every one
/// numbered after `anchor`.
fn latest_from(
    anchor: &Roster,
    revisions: &[Roster],
    devices: &SpaceDevices,
    own: &Own,
) -> Result<Roster, String> {
    let followed = follow::follow(anchor, &BTreeMap::new(), revisions, devices, own);
    let number = anchor.body().revision;
    if let Some(off) = (revisions.iter())
        .filter(|revision| revision.body().revision > number)
        .find(|revision| {
            !(followed.accepted.iter()).any(|accepted| accepted.hash() == revision.hash())
        })
    {
        return Err(format!(
            "revision {} the server gives of the space {} does not follow revision {number}, the \
             one the link names; nothing was signed",
            off.body().revision,
            anchor.body().name.as_str()
        ));
    }
    Ok(followed.accepted.last().unwrap_or(anchor).clone())
}

/// The certificate of the device that signed `invite`, a `client` or `browser` device of an admin
/// in `latest` with a valid certificate among `devices`, chained to the admin's master key, whose
/// key verifies the invite's signature.
fn signer<'d>(
    invite: &Invite,
    latest: &Roster,
    devices: &'d SpaceDevices,
) -> Result<&'d Cert, String> {
    let id = invite.body().signer;
    let name = latest.body().name.as_str();
    let account = (devices.certs.iter())
        .find(|cert| cert.body().device == id)
        .map(|cert| cert.body().account);
    let member = account.and_then(|account| latest.body().member(account));
    let Some(member) = member.filter(|member| spaces::admin_device(latest, member.account, id))
    else {
        return Err(format!(
            "the invite is signed by the device {id}, which is no device of an admin of the space \
             {name}; nothing was signed"
        ));
    };
    let cert = check_valid_device(&member.master, &devices.certs, &devices.revocations, id)
        .map_err(|err| {
            format!(
                "the invite's signer, the device {id} of the admin {}, has no valid certificate \
                 of its account: {err}; nothing was signed",
                member.handle.as_str()
            )
        })?;
    if !matches!(cert.body().kind, DeviceKind::Client | DeviceKind::Browser) {
        return Err(format!(
            "the invite is signed by the device {id}, which is neither a client nor a browser \
             device of an admin of the space {name}; nothing was signed"
        ));
    }
    check_invite(invite, &cert.body().signing).map_err(|_| {
        "the invite's signature does not verify with its signer's key; nothing was signed"
            .to_owned()
    })?;
    Ok(cert)
}

/// What the person sees of `checked` before confirming.
fn summary(checked: &Checked) -> String {
    let body = checked.invite.body();
    let role = match body.role {
        Role::Admin => "admin",
        _ => "member",
    };
    let space = checked.latest.body();
    format!(
        "The invite to the space {} ({}):\n- role: {role}\n- signed by: {} on the device {}\n- \
         channels: {}\n- expires: {} (UTC)",
        space.name.as_str(),
        space.space,
        checked.admin,
        checked.device,
        body.channels.len(),
        date(body.expires)
    )
}

/// `asked` as a handle, if it is a valid name that no member and no service agent's owner holds in
/// `latest`; otherwise why not.
fn free(latest: &Roster, asked: &str) -> Result<Name, String> {
    let handle: Name = asked.parse().map_err(|err| {
        format!(
            "\"{}\" is no valid handle: {err}; give another handle",
            printable(asked)
        )
    })?;
    let body = latest.body();
    let taken = body.members.iter().any(|member| member.handle == handle)
        || body
            .service_agents
            .iter()
            .any(|agent| agent.owner == handle);
    if taken {
        return Err(format!(
            "the handle {} is taken in the space {}; give another handle",
            handle.as_str(),
            body.name.as_str()
        ));
    }
    Ok(handle)
}

/// Runs `step` of `skald accept-invite` on the machine whose state is in `dir`, its secrets in
/// `storage`.
pub fn step(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    step: Step,
) -> Result<(), String> {
    let Step::Accept { anchor, acceptance } = step;
    let malformed = |what: &str| format!("the {what} of the step does not decode");
    let anchor = Roster::from_cbor(&anchor).map_err(|_| malformed("revision"))?;
    let acceptance = Acceptance::from_cbor(&acceptance).map_err(|_| malformed("acceptance"))?;
    let mut follower = spaces::following(dir, storage, speaks, timeout, Scope::All)?;
    let invite = acceptance.body().invite;
    if acceptance.body().account != follower.own.account {
        return Err("the acceptance is of another account than this device's".to_owned());
    }
    let (id, name) = (anchor.body().space, anchor.body().name.as_str());
    let held = (follower.state)
        .add_acceptance((id, name, &anchor.to_cbor()), invite, &acceptance.to_cbor())
        .map_err(text)?;
    match send(&mut follower, invite, &held) {
        Ok(()) => follower.state.acceptance_taken(invite).map_err(text)?,
        Err(Sent::Refused(err)) => {
            follower.state.end_acceptance(invite, id).map_err(text)?;
            return Err(format!(
                "the server refused the acceptance: {}; this device follows the space {name} no \
                 more",
                text(err)
            ));
        }
        Err(Sent::Failed(err)) => {
            return Err(format!(
                "{err}; the acceptance waits for the server to take it: skald recv sends it again"
            ));
        }
    }
    if let Some(space) = followed(&follower, id)?
        && let Err(err) = follower.sync(&space)
    {
        eprintln!("skald: warning: {err}");
    }
    println!(
        "Accepted. This device waits for an admin's device to admit this account to the space \
         {name}; skald recv then joins its channels. Bind a folder to the space with skald \
         add-rule <folder> {name}."
    );
    Ok(())
}

/// Why an acceptance is not on the server.
enum Sent {
    /// The server refused it: it is invalid, of another account than the one another acceptance
    /// is, or the server holds no such invite.
    Refused(connection::Error),
    /// The server did not answer, or failed otherwise, as on an error of its own: it may take it
    /// later.
    Failed(String),
}

/// `PUT /invites/<invite>/acceptance`: sends `acceptance`, in CBOR, again while the server does
/// not answer.
fn send(follower: &mut Follower, invite: InviteId, acceptance: &[u8]) -> Result<(), Sent> {
    let path = format!("/invites/{invite}/acceptance");
    let request = serde_json::json!({ "acceptance": URL_SAFE_NO_PAD.encode(acceptance) });
    let request = request.to_string();
    resent(
        &mut follower.connection,
        &follower.token,
        |connection, token| connection.send_json_as(Method::Put, &path, token, request.as_bytes()),
    )
    .map(drop)
    .map_err(|err| match err {
        connection::Error::Refused {
            status: 400 | 403 | 404 | 409,
            ..
        } => Sent::Refused(err),
        err => Sent::Failed(text(err)),
    })
}

/// The space `id` as the follower's state holds it, if it follows it.
fn followed(follower: &Follower, id: SpaceId) -> Result<Option<state::Space>, String> {
    Ok((follower.state.spaces().map_err(text)?.into_iter()).find(|space| space.id == id))
}

/// In a person's run that connects: for each acceptance the state holds, sends it if the server
/// has not taken it; once the server no longer holds its invite, forgets it when a revision lists
/// the account, and otherwise, the invite having ended, follows its space no more, saying so.
/// Warns of what fails, which a later run tries again.
pub fn settle(follower: &mut Follower) {
    if follower.service.is_some() {
        return;
    }
    let held = match follower.state.acceptances() {
        Ok(held) => held,
        Err(err) => return eprintln!("skald: warning: {}", text(err)),
    };
    for acceptance in held {
        if let Err(err) = settle_one(follower, &acceptance) {
            eprintln!("skald: warning: {err}");
        }
    }
}

/// As [`settle`], for `held`.
fn settle_one(follower: &mut Follower, held: &state::Acceptance) -> Result<(), String> {
    let Some(space) = followed(follower, held.space)? else {
        return follower.state.drop_acceptance(held.invite).map_err(text);
    };
    let name = space.name.clone();
    if held.taken {
        match (follower.connection).get_as(&invite_path(held.invite), &follower.token) {
            Ok(_) => return Ok(()),
            Err(connection::Error::Refused { status: 404, .. }) => {}
            Err(err) => return Err(text(err)),
        }
    } else {
        match send(follower, held.invite, &held.acceptance) {
            Ok(()) => return follower.state.acceptance_taken(held.invite).map_err(text),
            Err(Sent::Refused(_)) => {}
            Err(Sent::Failed(err)) => {
                return Err(format!(
                    "{err}; the acceptance of the invite to the space {name} waits for the server \
                     to take it: a later skald recv sends it again"
                ));
            }
        }
    }
    // The server holds no invite, or refused the acceptance: the space is readable only once a
    // revision lists the account.
    let path = format!("/spaces/{}/revisions", held.space);
    match follower.connection.get_as(&path, &follower.token) {
        Ok(_) => {
            let (latest, _) = follower.sync(&space)?;
            if latest.body().member(follower.own.account).is_some() {
                follower.state.drop_acceptance(held.invite).map_err(text)?;
            }
            Ok(())
        }
        Err(connection::Error::Refused { status: 404, .. }) => {
            (follower.state)
                .end_acceptance(held.invite, held.space)
                .map_err(text)?;
            eprintln!(
                "skald: the invite to the space {name} ended before an admin's device admitted \
                 this account: it expired, was revoked or another account accepted it; this \
                 device follows the space no more"
            );
            Ok(())
        }
        Err(err) => Err(text(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link's web address is the server's when it is the recorded one or the server's origin.
    #[test]
    fn web_address_is_the_recorded_one_or_the_servers_origin() {
        let api = "https://api.skald.test";
        let app = Some("https://app.skald.test");
        assert!(serves(api, app, "https://app.skald.test"));
        assert!(serves(api, app, "https://API.skald.test"));
        assert!(serves(api, app, "https://APP.skald.test:443"));
        assert!(!serves(api, app, "skald.test"));
        assert!(serves(api, None, "https://api.skald.test"));
        assert!(!serves(api, None, "https://app.skald.test"));
        assert!(!serves(api, app, "https://elsewhere.test"));
        assert!(!serves(
            "http://api.skald.test",
            None,
            "https://api.skald.test"
        ));
    }

    /// The state records a web address the server names only if it is an origin.
    #[test]
    fn only_an_origin_is_recorded_as_the_web_address() {
        let dir = crate::test_dir::TestDir::new("acceptances-web");
        let state = State::open(&dir).unwrap();
        let api = "https://api.skald.test";
        for named in [
            None,
            Some("https://app.skald.test/app"),
            Some("javascript:alert(1)"),
            Some("https://APP.skald.test"),
        ] {
            record_web(&state, api, named);
            assert_eq!(state.web(api).unwrap(), None, "{named:?}");
        }
        record_web(&state, api, Some("https://app.skald.test"));
        assert_eq!(
            state.web(api).unwrap().as_deref(),
            Some("https://app.skald.test")
        );
    }
}
