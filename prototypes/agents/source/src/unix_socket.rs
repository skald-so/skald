//! Unix sockets: std's on Unix. Elsewhere, such as on Windows, stand-ins that no call can make,
//! as connecting and binding fail: the code that needs a Unix socket builds there and says why it
//! cannot run.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};

#[cfg(not(unix))]
pub use stand_in::{UnixListener, UnixStream};

#[cfg(not(unix))]
mod stand_in {
    use std::io;
    use std::path::Path;
    use std::time::Duration;

    /// Why a Unix socket call fails off Unix.
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "needs a Unix socket: not on Windows",
        )
    }

    /// A stream no call can make.
    pub enum UnixStream {}

    impl UnixStream {
        pub fn connect(_: impl AsRef<Path>) -> io::Result<Self> {
            Err(unsupported())
        }

        pub fn shutdown(&self, _: std::net::Shutdown) -> io::Result<()> {
            match *self {}
        }

        pub fn set_nonblocking(&self, _: bool) -> io::Result<()> {
            match *self {}
        }

        pub fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
            match *self {}
        }
    }

    impl io::Read for UnixStream {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            match *self {}
        }
    }

    impl io::Write for UnixStream {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            match *self {}
        }

        fn flush(&mut self) -> io::Result<()> {
            match *self {}
        }
    }

    /// A listener no call can make.
    pub enum UnixListener {}

    impl UnixListener {
        pub fn bind(_: impl AsRef<Path>) -> io::Result<Self> {
            Err(unsupported())
        }

        pub fn set_nonblocking(&self, _: bool) -> io::Result<()> {
            match *self {}
        }

        pub fn accept(&self) -> io::Result<(UnixStream, ())> {
            match *self {}
        }
    }
}
