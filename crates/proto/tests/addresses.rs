//! Agent addresses: `docs/protocol/README.md`, "Agent address", and `docs/protocol/agents.md`,
//! "Person".

use skald_proto::{
    Address, AddressError, Name, NameError, PERSON, Segment, ServerError, ServerName,
};

fn parse(text: &str) -> Result<Address, AddressError> {
    text.parse()
}

fn name(text: &str) -> Name {
    text.parse().unwrap()
}

/// A server name of `len` characters: labels of up to 63 characters joined by dots.
fn server_of_len(len: usize) -> String {
    let mut server = String::new();
    while server.len() < len {
        if !server.is_empty() {
            server.push('.');
        }
        let label = (len - server.len()).min(63);
        server.push_str(&"a".repeat(label));
    }
    assert_eq!(server.len(), len);
    server
}

#[test]
fn addresses_round_trip() {
    let longest = "a".repeat(63);
    let valid = [
        "acme/alice/claude-api@skald.example".to_owned(),
        "space/alice/claude-api@skald.example".to_owned(),
        "a/b/c@d".to_owned(),
        "acme/ci/deploy-2@localhost".to_owned(),
        "0/9/x--y@xn--bcher-kva.example".to_owned(),
        "acme/alice/claude@123.example".to_owned(),
        "acme/alice/claude@skald.1a".to_owned(),
        format!("{longest}/{longest}/{longest}@{longest}.example"),
        format!("acme/alice/claude@{}", server_of_len(253)),
    ];
    for text in valid {
        let address = parse(&text).unwrap_or_else(|error| panic!("{text}: {error}"));
        assert_eq!(address.to_string(), text);
    }
}

#[test]
fn address_has_its_segments() {
    let address = parse("acme/alice/claude-api@skald.example").unwrap();
    assert_eq!(address.space().as_str(), "acme");
    assert_eq!(address.owner().as_str(), "alice");
    assert_eq!(address.agent().as_str(), "claude-api");
    assert_eq!(address.server().as_str(), "skald.example");
    assert_eq!(
        address,
        Address::new(
            name("acme"),
            name("alice"),
            name("claude-api"),
            "skald.example".parse().unwrap()
        )
    );
}

#[test]
fn user_is_a_person_address() {
    assert_eq!(PERSON, "user");
    let person = parse("acme/alice/user@skald.example").unwrap();
    assert!(person.is_person());
    assert_eq!(
        Address::person(
            name("acme"),
            name("alice"),
            "skald.example".parse().unwrap()
        ),
        person
    );
    assert_eq!(person.to_string(), "acme/alice/user@skald.example");
    for agent in [
        "acme/alice/claude-api@skald.example",
        "acme/user/claude@skald.example",
        "user/alice/claude@skald.example",
        "acme/alice/users@skald.example",
        "acme/alice/claude@user",
    ] {
        assert!(!parse(agent).unwrap().is_person(), "{agent}");
    }
}

#[test]
fn invalid_addresses_are_rejected() {
    use AddressError::{ExtraAt, MissingServer, SegmentCount, Server};
    let segment = AddressError::Segment;
    let label = |error| Server(ServerError::Label(error));
    let long = "a".repeat(64);
    let cases = [
        // Short forms: the wire always carries the full address.
        ("acme/alice/claude-api".to_owned(), MissingServer),
        ("alice/claude-api".to_owned(), MissingServer),
        ("".to_owned(), MissingServer),
        ("acme/alice/claude@skald@example".to_owned(), ExtraAt),
        ("alice/claude-api@skald.example".to_owned(), SegmentCount(2)),
        ("@skald.example".to_owned(), SegmentCount(1)),
        (
            "acme/alice/claude/x@skald.example".to_owned(),
            SegmentCount(4),
        ),
        (
            "/acme/alice/claude@skald.example".to_owned(),
            SegmentCount(4),
        ),
        (
            "acme/alice/claude/@skald.example".to_owned(),
            SegmentCount(4),
        ),
        // Empty segments.
        (
            "/alice/claude@skald.example".to_owned(),
            segment(Segment::Space, NameError::Empty),
        ),
        (
            "acme//claude@skald.example".to_owned(),
            segment(Segment::Owner, NameError::Empty),
        ),
        (
            "acme/alice/@skald.example".to_owned(),
            segment(Segment::Agent, NameError::Empty),
        ),
        ("acme/alice/claude@".to_owned(), Server(ServerError::Empty)),
        // Uppercase letters.
        (
            "Acme/alice/claude@skald.example".to_owned(),
            segment(Segment::Space, NameError::Uppercase('A')),
        ),
        (
            "acme/aLice/claude@skald.example".to_owned(),
            segment(Segment::Owner, NameError::Uppercase('L')),
        ),
        (
            "acme/alice/Claude@skald.example".to_owned(),
            segment(Segment::Agent, NameError::Uppercase('C')),
        ),
        (
            "acme/alice/claude@Skald.example".to_owned(),
            label(NameError::Uppercase('S')),
        ),
        // Other characters.
        (
            "acme/alice/claude_api@skald.example".to_owned(),
            segment(Segment::Agent, NameError::InvalidChar('_')),
        ),
        (
            "acme/alice/claude.api@skald.example".to_owned(),
            segment(Segment::Agent, NameError::InvalidChar('.')),
        ),
        (
            "acme/al ice/claude@skald.example".to_owned(),
            segment(Segment::Owner, NameError::InvalidChar(' ')),
        ),
        (
            "acmé/alice/claude@skald.example".to_owned(),
            segment(Segment::Space, NameError::InvalidChar('é')),
        ),
        (
            "acme/alice/claude@bücher.example".to_owned(),
            label(NameError::InvalidChar('ü')),
        ),
        (
            "acme/alice/claude@skald_example".to_owned(),
            label(NameError::InvalidChar('_')),
        ),
        (
            "acme/alice/claude@[::1]".to_owned(),
            label(NameError::InvalidChar('[')),
        ),
        // A hyphen at either end.
        (
            "-acme/alice/claude@skald.example".to_owned(),
            segment(Segment::Space, NameError::EdgeHyphen),
        ),
        (
            "acme/alice-/claude@skald.example".to_owned(),
            segment(Segment::Owner, NameError::EdgeHyphen),
        ),
        (
            "acme/alice/-@skald.example".to_owned(),
            segment(Segment::Agent, NameError::EdgeHyphen),
        ),
        (
            "acme/alice/claude@skald-.example".to_owned(),
            label(NameError::EdgeHyphen),
        ),
        (
            "acme/alice/claude@skald.-example".to_owned(),
            label(NameError::EdgeHyphen),
        ),
        // Names over 63 characters.
        (
            format!("{long}/alice/claude@skald.example"),
            segment(Segment::Space, NameError::TooLong(64)),
        ),
        (
            format!("acme/{long}/claude@skald.example"),
            segment(Segment::Owner, NameError::TooLong(64)),
        ),
        (
            format!("acme/alice/{long}@skald.example"),
            segment(Segment::Agent, NameError::TooLong(64)),
        ),
        (
            format!("acme/alice/claude@{long}.example"),
            label(NameError::TooLong(64)),
        ),
        // The server name.
        (
            "acme/alice/claude@skald.example:443".to_owned(),
            Server(ServerError::Port),
        ),
        (
            "acme/alice/claude@localhost:8080".to_owned(),
            Server(ServerError::Port),
        ),
        (
            "acme/alice/claude@skald.example:".to_owned(),
            label(NameError::InvalidChar(':')),
        ),
        (
            "acme/alice/claude@skald.example.".to_owned(),
            Server(ServerError::TrailingDot),
        ),
        (
            "acme/alice/claude@.".to_owned(),
            Server(ServerError::TrailingDot),
        ),
        (
            "acme/alice/claude@.skald.example".to_owned(),
            label(NameError::Empty),
        ),
        (
            "acme/alice/claude@skald..example".to_owned(),
            label(NameError::Empty),
        ),
        (
            format!("acme/alice/claude@{}", server_of_len(254)),
            Server(ServerError::TooLong(254)),
        ),
        (
            "acme/alice/claude@10.0.0.1".to_owned(),
            Server(ServerError::NumericLastLabel),
        ),
        (
            "acme/alice/claude@skald.123".to_owned(),
            Server(ServerError::NumericLastLabel),
        ),
        (
            "acme/alice/claude@0".to_owned(),
            Server(ServerError::NumericLastLabel),
        ),
    ];
    for (text, error) in cases {
        assert_eq!(parse(&text), Err(error), "{text:?}");
    }
}

#[test]
fn errors_name_the_problem() {
    let message = |text: &str| parse(text).unwrap_err().to_string();
    let long = "a".repeat(64);
    let cases = [
        (
            "acme/alice/claude".to_owned(),
            "invalid agent address: no '@' and server name; use space/owner/agent@server",
        ),
        (
            "a/b/c@d@e".to_owned(),
            "invalid agent address: more than one '@'",
        ),
        (
            "alice/claude@x".to_owned(),
            "invalid agent address: 2 '/'-separated segments before '@', not 3; use space/owner/agent@server",
        ),
        (
            "/alice/claude@x".to_owned(),
            "invalid agent address: the space name is empty",
        ),
        (
            "acme/Alice/claude@x".to_owned(),
            "invalid agent address: the owner name has the uppercase letter 'A'; use lowercase",
        ),
        (
            "acme/alice/claude_api@x".to_owned(),
            "invalid agent address: the agent name has the character '_'; only a-z, 0-9 and '-' are allowed",
        ),
        (
            "acme/alice/claude-@x".to_owned(),
            "invalid agent address: the agent name starts or ends with '-'",
        ),
        (
            format!("acme/{long}/claude@x"),
            "invalid agent address: the owner name is 64 characters; at most 63 are allowed",
        ),
        (
            "a/b/c@".to_owned(),
            "invalid agent address: the server name is empty",
        ),
        (
            "a/b/c@x:443".to_owned(),
            "invalid agent address: the server name has a port; an address has none",
        ),
        (
            "a/b/c@x.".to_owned(),
            "invalid agent address: the server name ends with '.'",
        ),
        (
            "a/b/c@x..y".to_owned(),
            "invalid agent address: the server name has a label that is empty",
        ),
        (
            format!("a/b/c@{}", server_of_len(300)),
            "invalid agent address: the server name is 300 characters; at most 253 are allowed",
        ),
        (
            "a/b/c@10.0.0.1".to_owned(),
            "invalid agent address: the server name ends with an all-digit label; an IP address is not allowed",
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(message(&text), expected, "{text:?}");
    }
}

#[test]
fn names_and_server_names_parse_alone() {
    assert_eq!(name("claude-api").to_string(), "claude-api");
    assert_eq!("".parse::<Name>(), Err(NameError::Empty));
    assert_eq!("a.b".parse::<Name>(), Err(NameError::InvalidChar('.')));
    let server: ServerName = "skald.example".parse().unwrap();
    assert_eq!(server.to_string(), "skald.example");
    assert_eq!(
        "skald.example.".parse::<ServerName>(),
        Err(ServerError::TrailingDot)
    );
}
