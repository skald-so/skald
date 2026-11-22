//! The DNS root of trust of a space (`docs/protocol/membership.md`, "Domain"): `skald members`
//! checks the members of a space whose policy names a domain against the TXT records of the
//! domain's `_skald` name.
//!
//! - It looks them up with DNS over HTTPS [RFC 8484] at the resolver the setting
//!   `SKALD_DNS_RESOLVER` names, Cloudflare's by default, reached as the server is
//!   ([`connection::agent_to`]), and keeps them, by that resolver, for their TTL. Results from one
//!   resolver are never shown with another's, so a run with another one, as an agent could make,
//!   changes nothing later runs show.
//! - A member they list with its fingerprint shows as listed. One they list with another, and an
//!   admin they do not list, are key changes: it warns of each once and keeps it by that resolver
//!   ([`key_changes::keep_from`]). A resolver that fails leaves
//!   the members not checked and fails no command.

use std::collections::BTreeSet;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_crypto::account_fingerprint;
use skald_proto::{AccountId, Role, Roster};

use crate::connection::{self, printable};
use crate::devices::text;
use crate::follow::{Own, master_of};
use crate::key_changes;
use crate::state::State;

/// The resolver by default: Cloudflare's.
pub const RESOLVER: &str = "https://cloudflare-dns.com/dns-query";

/// The setting that names the resolver.
pub const RESOLVER_SETTING: &str = "SKALD_DNS_RESOLVER";

/// The label before a domain of the name its records are at.
const LABEL: &str = "_skald";

/// The longest a domain name can be with [`LABEL`] before it: 253 characters in all.
const LONGEST: usize = 253 - LABEL.len() - 1;

/// The longest a lookup may take.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest DNS message a resolver gives.
const LARGEST: u64 = 65_535;

/// The DNS type of TXT and SOA records, and the class IN.
const TXT: u16 = 16;
const SOA: u16 = 6;
const IN: u16 = 1;

/// How long records are kept without a TTL of their own, as for a name without records and
/// without an SOA record; and the longest they are kept, whatever their TTL.
const NO_TTL: u32 = 300;
const MOST_TTL: u32 = 86_400;

/// `given` if it is a domain name a policy names (`docs/protocol/membership.md`, "Domain"): two
/// labels or more joined by `.`, each of 1 to 63 lowercase ASCII letters, digits and `-`, none
/// starting or ending with `-`, at most [`LONGEST`] characters in all, without a trailing dot.
pub fn name(given: &str) -> Result<String, String> {
    let label = |label: &str| {
        (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && (label.bytes())
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    if given.len() <= LONGEST && given.contains('.') && given.split('.').all(label) {
        Ok(given.to_owned())
    } else {
        Err(format!(
            "\"{}\" is no domain name, such as example.com: two labels or more of lowercase \
             ASCII letters, digits and '-', without a trailing dot",
            printable(given)
        ))
    }
}

/// What a space's domain shows of its members.
#[derive(Debug, PartialEq)]
pub struct Checked {
    /// The domain, as the policy names it.
    pub domain: String,
    /// The members its records list with their fingerprints; or why they were not checked.
    pub listed: Result<BTreeSet<AccountId>, String>,
    /// The key changes its records show, each the first time.
    pub warnings: Vec<String>,
}

/// Checks the members of `roster`, a space's last accepted revision, against the records of the
/// domain its policy names, if any, looked up at `resolver` or kept from it in `state`, with
/// `timeout` for the lookup at most; a member's fingerprint takes the master key [`master_of`]
/// gives.
pub fn check(
    state: &State,
    (resolver, timeout): (&str, Duration),
    roster: &Roster,
    own: &Own,
) -> Result<Option<Checked>, String> {
    let body = roster.body();
    let Some(given) = &body.policy.domain else {
        return Ok(None);
    };
    let not_checked = |why: String| Checked {
        domain: printable(given),
        listed: Err(why),
        warnings: Vec::new(),
    };
    let Ok(domain) = name(given) else {
        return Ok(Some(not_checked("it is no domain name".to_owned())));
    };
    let records = match records(state, (resolver, timeout.min(LOOKUP_TIMEOUT)), &domain)? {
        Ok(records) => records,
        Err(why) => return Ok(Some(not_checked(why))),
    };
    let key = (resolver, domain.as_str());
    let mut listed = BTreeSet::new();
    let mut warnings = Vec::new();
    for member in body.members.iter() {
        let Some(master) = master_of(roster, own, member.account) else {
            continue;
        };
        let fingerprint = account_fingerprint(member.account, &master);
        let mut theirs: Vec<&str> = (records.iter())
            .filter(|(account, _)| *account == member.account)
            .map(|(_, listed)| listed.as_str())
            .collect();
        theirs.sort_unstable();
        theirs.dedup();
        let handle = member.handle.as_str();
        let space = body.name.as_str();
        let warning = if theirs.contains(&fingerprint.as_str()) {
            listed.insert(member.account);
            state
                .clear_domain_warnings(key, member.account)
                .map_err(text)?;
            continue;
        } else if !theirs.is_empty() {
            let others = theirs.join(", ");
            (
                others.clone(),
                format!(
                    "{domain} lists {handle} of the space {space} with the fingerprint {others}, \
                     not {fingerprint}, the one its roster gives: its key may have been changed"
                ),
            )
        } else if member.role == Role::Admin {
            (
                String::new(),
                format!(
                    "{domain} does not list {handle}, an admin of the space {space}: its key may \
                     have been changed"
                ),
            )
        } else {
            continue;
        };
        if (state.warn_of_domain(key, member.account, &master, &warning.0)).map_err(text)? {
            key_changes::keep_from(state, &key_changes::dns(resolver), &warning.1)?;
            warnings.push(warning.1);
        }
    }
    Ok(Some(Checked {
        domain,
        listed: Ok(listed),
        warnings,
    }))
}

/// The accounts and fingerprints the records of `domain` list, kept in `state` from `resolver` or,
/// once those expire, looked up there with `timeout` and kept; or why they could not be.
fn records(
    state: &State,
    (resolver, timeout): (&str, Duration),
    domain: &str,
) -> Result<Result<Vec<(AccountId, String)>, String>, String> {
    let now = crate::spaces::now();
    let kept = state.domain_records(resolver, domain).map_err(text)?;
    let texts = match kept {
        Some((texts, expires)) if now < expires => texts,
        _ => match look_up(resolver, timeout, &format!("{LABEL}.{domain}")) {
            Ok((texts, ttl)) => {
                let texts: Vec<String> = texts
                    .into_iter()
                    .filter(|text| record(text).is_some())
                    .collect();
                let expires = now.saturating_add(u64::from(ttl.min(MOST_TTL)));
                (state.set_domain_records((resolver, domain), &texts, expires)).map_err(text)?;
                texts
            }
            Err(why) => return Ok(Err(why)),
        },
    };
    Ok(Ok(texts.iter().filter_map(|text| record(text)).collect()))
}

/// The account and fingerprint the record `text` lists: the account ID as text, one space and the
/// fingerprint, eight groups of five digits joined by one space; `None` for a text of another
/// form.
fn record(text: &str) -> Option<(AccountId, String)> {
    let (account, fingerprint) = text.split_once(' ')?;
    let groups: Vec<&str> = fingerprint.split(' ').collect();
    let digits = |group: &&str| group.len() == 5 && group.bytes().all(|byte| byte.is_ascii_digit());
    (groups.len() == 8 && groups.iter().all(digits))
        .then_some(())
        .and(account.parse().ok())
        .map(|account| (account, fingerprint.to_owned()))
}

/// The texts of the TXT records of `name` that the resolver at `resolver` gives over DNS over
/// HTTPS, by `GET`, within `timeout`, and how long to keep them, in seconds; or why it gave none.
fn look_up(resolver: &str, timeout: Duration, name: &str) -> Result<(Vec<String>, u32), String> {
    let failed = |why: String| format!("the resolver at {} failed: {why}", printable(resolver));
    let agent = connection::agent_to(resolver, timeout).map_err(|err| failed(err.to_string()))?;
    let joiner = if resolver.contains('?') { '&' } else { '?' };
    let url = format!(
        "{resolver}{joiner}dns={}",
        URL_SAFE_NO_PAD.encode(query(name))
    );
    let mut response = (agent.get(&url))
        .header("accept", "application/dns-message")
        .call()
        .map_err(|err| failed(printable(&err.to_string())))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(failed(format!("it answered with the HTTP status {status}")));
    }
    let message = (response.body_mut().with_config().limit(LARGEST))
        .read_to_vec()
        .map_err(|err| failed(printable(&err.to_string())))?;
    answer(&message).map_err(failed)
}

/// The DNS query for the TXT records of `name`, with the ID 0 [RFC 8484] and recursion desired.
fn query(name: &str) -> Vec<u8> {
    let mut out = vec![0, 0, 0x01, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        out.push(u8::try_from(label.len()).expect("a label of 63 characters at most"));
        out.extend(label.as_bytes());
    }
    out.push(0);
    out.extend(TXT.to_be_bytes());
    out.extend(IN.to_be_bytes());
    out
}

/// The texts of the TXT records in `message`, the answer to [`query`], each its strings joined,
/// and how long to keep them: their least TTL; for a name without records, the SOA record's
/// negative TTL [RFC 2308], or [`NO_TTL`] without one. A name that does not exist has no records.
fn answer(message: &[u8]) -> Result<(Vec<String>, u32), String> {
    let malformed = || "its answer is no DNS answer to the query".to_owned();
    let header = message.get(..12).ok_or_else(malformed)?;
    let count = |at: usize| usize::from(u16::from_be_bytes([header[at], header[at + 1]]));
    if header[..2] != [0, 0] || header[2] & 0x80 == 0 || header[2] & 0x78 != 0 {
        return Err(malformed());
    }
    if header[2] & 0x02 != 0 {
        return Err("its answer is truncated".to_owned());
    }
    match header[3] & 0x0f {
        0 | 3 => {}
        code => return Err(format!("it answered with the DNS error {code}")),
    }
    let mut at = 12;
    for _ in 0..count(4) {
        at = skip_name(message, at).ok_or_else(malformed)? + 4;
    }
    let (mut texts, mut least, mut negative) = (Vec::new(), None::<u32>, None::<u32>);
    for section in 0..2 {
        for _ in 0..count(6 + 2 * section) {
            at = skip_name(message, at).ok_or_else(malformed)?;
            let fixed = message.get(at..at + 10).ok_or_else(malformed)?;
            let kind = u16::from_be_bytes([fixed[0], fixed[1]]);
            let class = u16::from_be_bytes([fixed[2], fixed[3]]);
            let ttl = u32::from_be_bytes([fixed[4], fixed[5], fixed[6], fixed[7]]);
            let length = usize::from(u16::from_be_bytes([fixed[8], fixed[9]]));
            let data = message
                .get(at + 10..at + 10 + length)
                .ok_or_else(malformed)?;
            at += 10 + length;
            match (section, kind, class) {
                (0, TXT, IN) => {
                    texts.push(strings(data).ok_or_else(malformed)?);
                    least = Some(least.map_or(ttl, |least| least.min(ttl)));
                }
                (1, SOA, IN) => {
                    let minimum = data.len().checked_sub(4).map(|end| &data[end..]);
                    let minimum = minimum.ok_or_else(malformed)?;
                    let minimum = u32::from_be_bytes(minimum.try_into().expect("4 bytes"));
                    negative = Some(ttl.min(minimum));
                }
                _ => {}
            }
        }
    }
    let ttl = least.or(negative.filter(|_| texts.is_empty()));
    Ok((texts, ttl.unwrap_or(NO_TTL)))
}

/// Where the name at `at` in `message` ends.
fn skip_name(message: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let length = *message.get(at)?;
        match length {
            0 => return Some(at + 1),
            // A pointer to a name before it.
            _ if length & 0xc0 == 0xc0 => return message.get(at + 1).map(|_| at + 2),
            _ if length & 0xc0 == 0 => at += 1 + usize::from(length),
            _ => return None,
        }
    }
}

/// The character strings of `data`, a TXT record's data, joined; `None` unless they fill it, or
/// if they are no UTF-8.
fn strings(data: &[u8]) -> Option<String> {
    let mut joined = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let length = usize::from(data[at]);
        joined.extend(data.get(at + 1..at + 1 + length)?);
        at += 1 + length;
    }
    String::from_utf8(joined).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spaces::tests::{first, member};
    use skald_crypto::{SigningKey, sign_roster};

    /// A resource record of the class IN: its name, as labels or a pointer, `kind`, `ttl` and
    /// `data`.
    fn resource(name: &[u8], kind: u16, ttl: u32, data: &[u8]) -> Vec<u8> {
        let length = u16::try_from(data.len()).unwrap();
        [
            name,
            &kind.to_be_bytes(),
            &IN.to_be_bytes(),
            &ttl.to_be_bytes(),
            &length.to_be_bytes(),
            data,
        ]
        .concat()
    }

    /// A DNS answer to [`query`] for `name` with `rcode`, the TXT records `txt`, each its strings
    /// and TTL, and in the authority section an SOA record with `soa`, its TTL and minimum.
    fn reply(name: &str, rcode: u8, txt: &[(&[&str], u32)], soa: Option<(u32, u32)>) -> Vec<u8> {
        let counts = [1, txt.len(), usize::from(soa.is_some()), 0];
        let mut out = vec![0, 0, 0x81, 0x80 | rcode];
        for count in counts {
            out.extend(u16::try_from(count).unwrap().to_be_bytes());
        }
        out.extend(&query(name)[12..]);
        // The question's name.
        let pointer = [0xc0, 0x0c];
        for (strings, ttl) in txt {
            let data: Vec<u8> = (strings.iter())
                .flat_map(|string| {
                    let length = u8::try_from(string.len()).unwrap();
                    std::iter::once(length).chain(string.bytes())
                })
                .collect();
            out.extend(resource(&pointer, TXT, *ttl, &data));
        }
        if let Some((ttl, minimum)) = soa {
            // Its servers' names as pointers, then five numbers, the minimum last.
            let data = [&pointer[..], &pointer, &[0; 16], &minimum.to_be_bytes()].concat();
            out.extend(resource(b"\x07example\x00", SOA, ttl, &data));
        }
        out
    }

    #[test]
    fn a_domain_name_is_lowercase_ascii_labels() {
        let longest = format!("{}.example", vec!["a".repeat(63); 4].join("."));
        let label = format!("{}.example", "a".repeat(63));
        for given in [
            "corp.example",
            "corp-1.example",
            "a.b.c.example",
            "1.example",
            &label,
            &longest[longest.len() - LONGEST..],
        ] {
            assert_eq!(name(given).as_deref(), Ok(given), "{given}");
        }
        let too_long = format!("a{}", &longest[longest.len() - LONGEST..]);
        for given in [
            "example",
            "",
            ".example",
            "corp..example",
            "corp.example.",
            "Corp.Example",
            "-corp.example",
            "corp-.example",
            "corp_x.example",
            "corp example",
            "пример.example",
            &format!("{}.example", "a".repeat(64)),
            &too_long,
        ] {
            assert!(name(given).is_err(), "{given}");
        }
    }

    #[test]
    fn a_query_asks_for_the_txt_records_of_the_name() {
        assert_eq!(
            query("_skald.corp.example"),
            [
                &[0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 6][..],
                b"_skald",
                &[4],
                b"corp",
                &[7],
                b"example",
                &[0, 0, 16, 0, 1],
            ]
            .concat()
        );
    }

    #[test]
    fn an_answer_gives_its_texts_and_their_least_ttl() {
        let name = "_skald.corp.example";
        let txt: [(&[&str], u32); 2] = [(&["one ", "text"], 600), (&["two"], 60)];
        assert_eq!(
            answer(&reply(name, 0, &txt, None)),
            Ok((vec!["one text".to_owned(), "two".to_owned()], 60))
        );
    }

    #[test]
    fn a_name_without_records_is_kept_for_its_negative_ttl() {
        let name = "_skald.corp.example";
        for rcode in [0, 3] {
            assert_eq!(
                answer(&reply(name, rcode, &[], Some((900, 120)))),
                Ok((Vec::new(), 120))
            );
            assert_eq!(
                answer(&reply(name, rcode, &[], Some((90, 120)))),
                Ok((Vec::new(), 90))
            );
            assert_eq!(
                answer(&reply(name, rcode, &[], None)),
                Ok((Vec::new(), NO_TTL))
            );
        }
    }

    #[test]
    fn an_error_a_truncated_answer_or_no_answer_gives_no_records() {
        let name = "_skald.corp.example";
        let good = reply(name, 0, &[(&["text"], 60)], None);
        let mut truncated = good.clone();
        truncated[2] |= 0x02;
        let mut query_back = good.clone();
        query_back[2] &= 0x7f;
        let mut other_id = good.clone();
        other_id[1] = 1;
        for (what, message) in [
            ("SERVFAIL", reply(name, 2, &[], None)),
            ("REFUSED", reply(name, 5, &[], None)),
            ("truncated", truncated),
            ("a query", query_back),
            ("another ID", other_id),
            ("cut short", good[..good.len() - 1].to_vec()),
            ("empty", Vec::new()),
        ] {
            assert!(answer(&message).is_err(), "{what}");
        }
    }

    #[test]
    fn a_record_is_an_account_id_and_a_fingerprint_each_after_one_space() {
        let account = AccountId::generate();
        let fingerprint = "01234 56789 01234 56789 01234 56789 01234 56789";
        assert_eq!(
            record(&format!("{account} {fingerprint}")),
            Some((account, fingerprint.to_owned()))
        );
        let upper = account.to_string().to_uppercase();
        for given in [
            format!("{account}"),
            format!(" {account} {fingerprint}"),
            format!("{account} {fingerprint} "),
            format!("{account}  {fingerprint}"),
            format!("{account}\t{fingerprint}"),
            format!("{account} {}", fingerprint.replace(' ', "  ")),
            format!("{upper} {fingerprint}"),
            format!("{account} {}", &fingerprint[..41]),
            format!("{account} {fingerprint} 01234"),
            format!("{account} {}", fingerprint.replace('0', "a")),
            format!("{account} {}", fingerprint.replace(' ', "")),
            format!("not-an-id {fingerprint}"),
            "v=spf1 -all".to_owned(),
        ] {
            assert_eq!(record(&given), None, "{given}");
        }
    }

    /// Records kept from a resolver: a member they list with its fingerprint is listed; one they
    /// list with another, and an admin they do not list, warn once, and again once the records
    /// have listed it with its fingerprint between; a member that is no admin they do not list
    /// shows nothing. A key change another resolver showed first still warns once with this one.
    /// A policy's domain of another form is not checked.
    #[test]
    fn kept_records_list_members_and_warn_of_each_key_change_once() {
        let state = State::in_memory().unwrap();
        // Ada, the admin, with the master key `first` gives; Bob, a member, this device's account.
        let (ada, bob, bob_master) = (AccountId::generate(), AccountId::generate(), [5; 32]);
        let mut body = first(ada).body().clone();
        body.members.push(member(bob, "bob", bob_master));
        body.policy.domain = Some("corp.example".to_owned());
        let roster =
            |body: &skald_proto::RosterBody| sign_roster(body, &SigningKey::generate()).unwrap();
        let own = Own {
            account: bob,
            master: bob_master,
        };
        // Ports nothing answers on: every check below takes the records kept.
        let resolver = ("http://127.0.0.1:9/dns-query", Duration::from_secs(1));
        let other = ("http://127.0.0.1:7/dns-query", Duration::from_secs(1));
        let fingerprint = |account, master: &[u8; 32]| account_fingerprint(account, master);
        let line = |account, fingerprint: String| format!("{account} {fingerprint}");
        let [ada_listed, ada_elsewhere, bob_listed] = [
            line(ada, fingerprint(ada, &[3; 32])),
            line(ada, fingerprint(ada, &[4; 32])),
            line(bob, fingerprint(bob, &bob_master)),
        ];
        let check_at = |resolver: (&str, Duration), records: &[&String]| {
            let records: Vec<String> = records.iter().map(|text| (*text).clone()).collect();
            let expires = crate::spaces::now() + 3_600;
            (state.set_domain_records((resolver.0, "corp.example"), &records, expires)).unwrap();
            let checked = check(&state, resolver, &roster(&body), &own)
                .unwrap()
                .unwrap();
            assert_eq!(checked.domain, "corp.example");
            let listed = checked.listed.expect("checked");
            let naming = |handle: &str| {
                (checked.warnings.iter())
                    .filter(|warning| {
                        let mut words = warning
                            .split(|char: char| !(char.is_ascii_alphanumeric() || char == '-'));
                        words.any(|word| word == handle)
                    })
                    .count()
            };
            assert_eq!(naming("bob"), 0, "{:?}", checked.warnings);
            (listed, naming("ada"), checked.warnings.len())
        };
        let check_with = |records: &[&String]| check_at(resolver, records);
        let both = BTreeSet::from([ada, bob]);
        let only_bob = BTreeSet::from([bob]);

        assert_eq!(
            check_at(other, &[&bob_listed, &ada_elsewhere]),
            (only_bob.clone(), 1, 1)
        );

        assert_eq!(
            check_with(&[&bob_listed, &ada_elsewhere]),
            (only_bob.clone(), 1, 1)
        );
        assert_eq!(
            check_with(&[&bob_listed, &ada_elsewhere]),
            (only_bob.clone(), 0, 0)
        );
        assert_eq!(
            check_with(&[&bob_listed, &ada_listed, &ada_elsewhere]),
            (both.clone(), 0, 0)
        );
        assert_eq!(
            check_with(&[&bob_listed, &ada_elsewhere]),
            (only_bob.clone(), 1, 1)
        );
        assert_eq!(check_with(&[&bob_listed]), (only_bob.clone(), 1, 1));
        assert_eq!(check_with(&[&bob_listed]), (only_bob, 0, 0));
        assert_eq!(check_with(&[&ada_listed]), (BTreeSet::from([ada]), 0, 0));

        for given in ["not a domain", "Corp.Example", "corp.example."] {
            body.policy.domain = Some(given.to_owned());
            let checked = check(&state, resolver, &roster(&body), &own)
                .unwrap()
                .unwrap();
            assert!(
                checked.listed.is_err() && checked.warnings.is_empty(),
                "{checked:?}"
            );
        }
        body.policy.domain = None;
        assert_eq!(check(&state, resolver, &roster(&body), &own), Ok(None));
    }
}
