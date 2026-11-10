//! Agent addresses: `docs/protocol/README.md`, "Agent address".

use std::fmt;
use std::str::FromStr;

/// The reserved agent name of a person address: `docs/protocol/agents.md`, "Person".
pub const PERSON: &str = "user";

/// The longest name: 1 to 63 characters.
const NAME_MAX: usize = 63;
/// The longest server name.
const SERVER_MAX: usize = 253;

/// A segment of an address: a space, owner or agent name, or a label of a server name.
/// `a-z` and `0-9`, with `-` inside; 1 to 63 characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

/// The DNS name of a server, in A-label form: [`Name`] labels joined by `.`, at most 253
/// characters, the last label not all digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerName(String);

/// An agent address, `space/owner/agent@server`, such as `acme/alice/claude-api@skald.example`.
/// An address whose agent is [`PERSON`] names a person.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address {
    space: Name,
    owner: Name,
    agent: Name,
    server: ServerName,
}

/// Why text is not a [`Name`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameError {
    /// The name is empty.
    Empty,
    /// The name has an uppercase letter.
    Uppercase(char),
    /// The name has a character other than `a-z`, `0-9` and `-`.
    InvalidChar(char),
    /// The name starts or ends with `-`.
    EdgeHyphen,
    /// The name is longer than 63 characters; holds its length.
    TooLong(usize),
}

/// Why text is not a [`ServerName`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerError {
    /// The server name is empty.
    Empty,
    /// The server name ends with a port, such as `:443`.
    Port,
    /// The server name ends with `.`.
    TrailingDot,
    /// A label of the server name is invalid.
    Label(NameError),
    /// The server name is longer than 253 characters; holds its length.
    TooLong(usize),
    /// The last label is all digits, as in an IP address.
    NumericLastLabel,
}

/// A segment of an [`Address`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    Space,
    Owner,
    Agent,
}

/// Why text is not an [`Address`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressError {
    /// There is no `@` before the server name.
    MissingServer,
    /// There is more than one `@`.
    ExtraAt,
    /// The part before `@` does not have exactly three segments; holds how many it has.
    SegmentCount(usize),
    /// A segment is invalid.
    Segment(Segment, NameError),
    /// The server name is invalid.
    Server(ServerError),
}

impl Name {
    /// The name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Name {
    type Err = NameError;

    fn from_str(text: &str) -> Result<Self, NameError> {
        if text.is_empty() {
            return Err(NameError::Empty);
        }
        if let Some(char) = text.chars().find(|&char| !is_name_char(char)) {
            return Err(if char.is_ascii_uppercase() {
                NameError::Uppercase(char)
            } else {
                NameError::InvalidChar(char)
            });
        }
        if text.starts_with('-') || text.ends_with('-') {
            return Err(NameError::EdgeHyphen);
        }
        if text.len() > NAME_MAX {
            return Err(NameError::TooLong(text.len()));
        }
        Ok(Self(text.to_owned()))
    }
}

fn is_name_char(char: char) -> bool {
    char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-'
}

impl ServerName {
    /// The server name as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ServerName {
    type Err = ServerError;

    fn from_str(text: &str) -> Result<Self, ServerError> {
        if text.is_empty() {
            return Err(ServerError::Empty);
        }
        if text
            .rsplit_once(':')
            .is_some_and(|(_, port)| !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(ServerError::Port);
        }
        if text.ends_with('.') {
            return Err(ServerError::TrailingDot);
        }
        let mut last = "";
        for label in text.split('.') {
            label.parse::<Name>().map_err(ServerError::Label)?;
            last = label;
        }
        if text.len() > SERVER_MAX {
            return Err(ServerError::TooLong(text.len()));
        }
        if last.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ServerError::NumericLastLabel);
        }
        Ok(Self(text.to_owned()))
    }
}

impl Address {
    /// The address of `agent`, owned by `owner` in `space` on `server`.
    pub fn new(space: Name, owner: Name, agent: Name, server: ServerName) -> Self {
        Self {
            space,
            owner,
            agent,
            server,
        }
    }

    /// The address of the person whose handle in `space` is `owner`: its agent is [`PERSON`].
    pub fn person(space: Name, owner: Name, server: ServerName) -> Self {
        Self::new(space, owner, Name(PERSON.to_owned()), server)
    }

    /// The space: unique on its server.
    pub fn space(&self) -> &Name {
        &self.space
    }

    /// The owner: a member's handle or a group of service agents, unique in the space.
    pub fn owner(&self) -> &Name {
        &self.owner
    }

    /// The agent: unique within its owner; [`PERSON`] in a person address.
    pub fn agent(&self) -> &Name {
        &self.agent
    }

    /// The server hosting the space.
    pub fn server(&self) -> &ServerName {
        &self.server
    }

    /// Whether this is a person address: one whose agent is [`PERSON`].
    pub fn is_person(&self) -> bool {
        self.agent.as_str() == PERSON
    }
}

impl FromStr for Address {
    type Err = AddressError;

    /// Parses the full address; a short form such as `alice/claude-api` is invalid.
    fn from_str(text: &str) -> Result<Self, AddressError> {
        let (path, server) = text.split_once('@').ok_or(AddressError::MissingServer)?;
        if server.contains('@') {
            return Err(AddressError::ExtraAt);
        }
        let segments: Vec<&str> = path.split('/').collect();
        let [space, owner, agent] = segments[..] else {
            return Err(AddressError::SegmentCount(segments.len()));
        };
        let segment = |segment, text: &str| {
            text.parse::<Name>()
                .map_err(|error| AddressError::Segment(segment, error))
        };
        Ok(Self::new(
            segment(Segment::Space, space)?,
            segment(Segment::Owner, owner)?,
            segment(Segment::Agent, agent)?,
            server.parse().map_err(AddressError::Server)?,
        ))
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for ServerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for Address {
    /// The full address, as the wire carries it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}@{}",
            self.space, self.owner, self.agent, self.server
        )
    }
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("is empty"),
            Self::Uppercase(char) => write!(f, "has the uppercase letter '{char}'; use lowercase"),
            Self::InvalidChar(char) => write!(
                f,
                "has the character {char:?}; only a-z, 0-9 and '-' are allowed"
            ),
            Self::EdgeHyphen => f.write_str("starts or ends with '-'"),
            Self::TooLong(len) => write!(f, "is {len} characters; at most {NAME_MAX} are allowed"),
        }
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the server name ")?;
        match self {
            Self::Empty => f.write_str("is empty"),
            Self::Port => f.write_str("has a port; an address has none"),
            Self::TrailingDot => f.write_str("ends with '.'"),
            Self::Label(error) => write!(f, "has a label that {error}"),
            Self::TooLong(len) => {
                write!(f, "is {len} characters; at most {SERVER_MAX} are allowed")
            }
            Self::NumericLastLabel => {
                f.write_str("ends with an all-digit label; an IP address is not allowed")
            }
        }
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Space => "space",
            Self::Owner => "owner",
            Self::Agent => "agent",
        })
    }
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid agent address: ")?;
        match self {
            Self::MissingServer => {
                f.write_str("no '@' and server name; use space/owner/agent@server")
            }
            Self::ExtraAt => f.write_str("more than one '@'"),
            Self::SegmentCount(count) => write!(
                f,
                "{count} '/'-separated segments before '@', not 3; use space/owner/agent@server"
            ),
            Self::Segment(segment, error) => write!(f, "the {segment} name {error}"),
            Self::Server(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for NameError {}
impl std::error::Error for ServerError {}
impl std::error::Error for AddressError {}
