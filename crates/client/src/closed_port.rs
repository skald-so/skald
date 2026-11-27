//! A local address nothing listens on, for the tests of a server or a proxy that is down. The unit
//! tests use it, and the integration tests of the client and the server through `#[path]`.

use std::net::{Ipv4Addr, SocketAddr};

use socket2::{Domain, Socket, Type};

/// A local address nothing listens on, its port below the ephemeral ranges of Linux, macOS and
/// Windows: no test's server or connection gets it as its own, so a connection there is refused
/// and cannot connect to itself, as one to a closed ephemeral port can. A port is tried by binding
/// it without listening, so that a connection of another test to a port being tried is refused
/// too.
pub fn closed_port() -> SocketAddr {
    let start = 20_000 + std::process::id() % 10_000;
    (start..30_000)
        .chain(20_000..start)
        .find_map(|port| {
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, u16::try_from(port).ok()?));
            let socket = Socket::new(Domain::IPV4, Type::STREAM, None).ok()?;
            socket.bind(&address.into()).ok()?;
            Some(address)
        })
        .expect("a free local port below the ephemeral ranges")
}
