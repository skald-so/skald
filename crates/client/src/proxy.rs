//! The proxy of the client's connection to its server: the one `https_proxy` or `HTTPS_PROXY`
//! names, unless `no_proxy` or `NO_PROXY` lists the server's host, each read as curl reads it
//! (`docs/dev/architecture.md`, "Network"). The client reaches the server through it by an HTTP
//! `CONNECT` tunnel.

use std::env;
use std::fmt;
use std::net::IpAddr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use tungstenite::http::Uri;

/// The settings of the proxy, and of the hosts that take none, in the order curl reads them.
const PROXY_SETTINGS: [&str; 2] = ["https_proxy", "HTTPS_PROXY"];
const NO_PROXY_SETTINGS: [&str; 2] = ["no_proxy", "NO_PROXY"];

/// The port of a proxy whose URL has none, as curl takes it.
const DEFAULT_PORT: u16 = 1080;

/// An HTTP proxy that tunnels by `CONNECT`.
#[derive(Clone, PartialEq)]
pub struct Proxy {
    /// Its host as its URL has it, an IPv6 address in brackets, and its port.
    host: String,
    port: u16,
    /// The value of `Proxy-Authorization`, `Basic <credentials>`, when its URL holds credentials.
    authorization: Option<String>,
}

/// `HTTPS_PROXY` holds no URL of an HTTP proxy.
#[derive(Debug)]
pub struct BadUrl;

impl Proxy {
    /// The proxy of the server at `host`, as the server URL has it, from this process's
    /// environment: `None` when none is set or `NO_PROXY` lists `host`.
    pub fn for_host(host: &str) -> Result<Option<Proxy>, BadUrl> {
        let read = |names: [&str; 2]| {
            names.iter().find_map(|name| {
                env::var_os(name)
                    .map(|value| value.to_string_lossy().trim().to_owned())
                    .filter(|value| !value.is_empty())
            })
        };
        choose(
            read(PROXY_SETTINGS).as_deref(),
            read(NO_PROXY_SETTINGS).as_deref(),
            host,
        )
    }

    /// The proxy at `url`, such as `http://user:password@proxy.corp:3128`: `http://` or no scheme,
    /// a host, a port or curl's 1080, and credentials percent-decoded as curl decodes them.
    fn parse(url: &str) -> Result<Proxy, BadUrl> {
        let url = if url.contains("://") {
            url.to_owned()
        } else {
            format!("http://{url}")
        };
        let uri: Uri = url.parse().map_err(|_| BadUrl)?;
        let authority = uri.authority().ok_or(BadUrl)?;
        let scheme = uri.scheme_str().unwrap_or_default();
        let path = uri.path_and_query().map_or("", |path| path.as_str());
        if !scheme.eq_ignore_ascii_case("http") || authority.host().is_empty() || path.len() > 1 {
            return Err(BadUrl);
        }
        let authorization = authority.as_str().rsplit_once('@').map(|(userinfo, _)| {
            let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            let mut credentials = decoded(user);
            credentials.push(b':');
            credentials.extend(decoded(password));
            format!("Basic {}", STANDARD.encode(credentials))
        });
        Ok(Proxy {
            host: authority.host().to_owned(),
            port: authority.port_u16().unwrap_or(DEFAULT_PORT),
            authorization,
        })
    }

    /// Its host and port, such as `proxy.corp:3128`: how messages name it, never with the
    /// credentials of its URL.
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// The request for a tunnel to `authority`, the server's host and port, with the credentials
    /// of its URL as Basic, if any.
    pub fn connect_request(&self, authority: &str) -> String {
        let mut request = format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: skald/{}\r\n",
            env!("CARGO_PKG_VERSION")
        );
        if let Some(authorization) = &self.authorization {
            request.push_str(&format!("Proxy-Authorization: {authorization}\r\n"));
        }
        request.push_str("\r\n");
        request
    }
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Proxy({})", self.address())
    }
}

/// The proxy at `proxy`, if set, of the server at `host`, unless `no_proxy` lists `host`.
fn choose(
    proxy: Option<&str>,
    no_proxy: Option<&str>,
    host: &str,
) -> Result<Option<Proxy>, BadUrl> {
    match proxy {
        Some(_) if no_proxy.is_some_and(|list| bypasses(list, host)) => Ok(None),
        Some(url) => Proxy::parse(url).map(Some),
        None => Ok(None),
    }
}

/// `text` with each `%XX` decoded to its byte, as curl decodes a proxy's credentials.
fn decoded(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = bytes.get(index + 1..index + 3).and_then(|pair| {
            let pair = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(pair, 16).ok()
        });
        match (bytes[index], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                index += 3;
            }
            (byte, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    out
}

/// Whether `no_proxy` lists `host`, as curl reads it: `*` for every host; otherwise entries
/// separated by commas or spaces, each a host name that matches itself and the names under it, in
/// any case and with a leading or trailing dot ignored, or for an address host, an IP address or
/// CIDR block.
fn bypasses(no_proxy: &str, host: &str) -> bool {
    if no_proxy.trim() == "*" {
        return true;
    }
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let host = host.strip_suffix('.').unwrap_or(host);
    let address = host.parse::<IpAddr>().ok();
    no_proxy
        .split([',', ' ', '\t'])
        .filter(|entry| !entry.is_empty())
        .any(|entry| match address {
            Some(address) => in_block(address, entry),
            None => under(host, entry),
        })
}

/// Whether the host name `host` is `entry` or a name under it.
fn under(host: &str, entry: &str) -> bool {
    let entry = entry.strip_suffix('.').unwrap_or(entry);
    let entry = entry.strip_prefix('.').unwrap_or(entry);
    if entry.is_empty() || entry.len() > host.len() {
        return false;
    }
    let (head, tail) = host.split_at(host.len() - entry.len());
    tail.eq_ignore_ascii_case(entry) && (head.is_empty() || head.ends_with('.'))
}

/// Whether `address` is the address `entry` or in its CIDR block, such as `10.0.0.0/8`.
fn in_block(address: IpAddr, entry: &str) -> bool {
    let (block, bits) = match entry.split_once('/') {
        Some((block, bits)) => match bits.parse::<u32>() {
            Ok(bits) => (block, Some(bits)),
            Err(_) => return false,
        },
        None => (entry, None),
    };
    let block = block.trim_start_matches('[').trim_end_matches(']');
    let (address, block, width) = match (address, block.parse::<IpAddr>()) {
        (IpAddr::V4(address), Ok(IpAddr::V4(block))) => (
            u128::from(address.to_bits()),
            u128::from(block.to_bits()),
            32,
        ),
        (IpAddr::V6(address), Ok(IpAddr::V6(block))) => (address.to_bits(), block.to_bits(), 128),
        _ => return false,
    };
    // As curl, a block of 0 bits is the address alone.
    let bits = bits.filter(|&bits| bits != 0).unwrap_or(width);
    if bits > width {
        return false;
    }
    let shift = width - bits;
    address >> shift == block >> shift
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(url: &str) -> Proxy {
        Proxy::parse(url).expect(url)
    }

    #[test]
    fn proxy_url_gives_its_host_port_and_credentials_as_basic() {
        let plain = proxy("http://ada:s3cret@proxy.corp:3128");
        assert_eq!(plain.address(), "proxy.corp:3128");
        assert_eq!(
            plain.authorization.as_deref(),
            Some(format!("Basic {}", STANDARD.encode("ada:s3cret")).as_str())
        );
        let encoded = proxy("HTTP://ada%40corp:p%3Ass%2Fw%zz@[::1]:8080/");
        assert_eq!(encoded.address(), "[::1]:8080");
        assert_eq!(
            encoded.authorization.as_deref(),
            Some(format!("Basic {}", STANDARD.encode("ada@corp:p:ss/w%zz")).as_str())
        );
        let bare = proxy("proxy.corp");
        assert_eq!(
            (bare.address(), bare.authorization),
            ("proxy.corp:1080".to_owned(), None)
        );
        assert_eq!(proxy("ada@10.0.0.1:3128").address(), "10.0.0.1:3128");
        for url in [
            "https://proxy.corp:3128",
            "socks5://proxy.corp:1080",
            "http://",
            "http://proxy.corp:3128/path",
            "http://proxy corp",
        ] {
            assert!(Proxy::parse(url).is_err(), "{url}");
        }
    }

    #[test]
    fn proxy_shows_no_credentials() {
        let proxy = proxy("http://ada:s3cret@proxy.corp:3128");
        assert_eq!(format!("{proxy:?}"), "Proxy(proxy.corp:3128)");
        let request = proxy.connect_request("skald.test:443");
        assert!(request.starts_with("CONNECT skald.test:443 HTTP/1.1\r\nHost: skald.test:443\r\n"));
        assert!(request.ends_with(&format!(
            "Proxy-Authorization: Basic {}\r\n\r\n",
            STANDARD.encode("ada:s3cret")
        )));
        assert!(
            !self::proxy("proxy.corp")
                .connect_request("a:1")
                .contains("Proxy-Authorization")
        );
    }

    #[test]
    fn no_proxy_lists_hosts_as_curl_reads_it() {
        for (list, host) in [
            ("*", "skald.test"),
            (" * ", "10.0.0.1"),
            ("skald.test", "skald.test"),
            ("SKALD.Test", "skald.test"),
            (".skald.test", "skald.test"),
            ("skald.test.", "skald.test."),
            ("test", "api.skald.test"),
            (".test", "skald.test"),
            ("other.example, 127.0.0.1", "127.0.0.1"),
            ("other.example\tskald.test", "skald.test"),
            ("127.0.0.0/8", "127.0.0.1"),
            ("10.1.2.3/32", "10.1.2.3"),
            ("::1", "[::1]"),
            ("[2001:db8::]/32", "[2001:db8::7]"),
            ("localhost", "LocalHost"),
        ] {
            assert!(bypasses(list, host), "{list:?} {host}");
        }
        for (list, host) in [
            ("", "skald.test"),
            ("kald.test", "skald.test"),
            ("skald.test.example", "skald.test"),
            ("skald", "skald.test"),
            ("127.0.0.1", "skald.test"),
            ("*.skald.test", "api.skald.test"),
            ("skald.test", "127.0.0.1"),
            ("127.0.0.2", "127.0.0.1"),
            ("10.0.0.0/8", "11.0.0.1"),
            ("10.0.0.0/33", "10.0.0.1"),
            ("10.0.0.0/x", "10.0.0.1"),
            ("127.0.0.1", "[::1]"),
            ("*, other.example", "skald.test"),
        ] {
            assert!(!bypasses(list, host), "{list:?} {host}");
        }
    }

    #[test]
    fn listed_host_takes_no_proxy() {
        let url = Some("http://proxy.corp:3128");
        let chosen = |no_proxy, host| choose(url, no_proxy, host).expect("a proxy URL");
        assert_eq!(
            chosen(None, "skald.test"),
            Some(proxy("http://proxy.corp:3128"))
        );
        assert_eq!(
            chosen(Some("other.test"), "skald.test"),
            Some(proxy("proxy.corp:3128"))
        );
        assert_eq!(chosen(Some("skald.test"), "skald.test"), None);
        assert_eq!(choose(None, None, "skald.test").ok(), Some(None));
        assert!(choose(Some("ftp://proxy.corp"), None, "skald.test").is_err());
    }
}
