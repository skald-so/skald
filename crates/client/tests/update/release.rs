//! A stand-in for the GitHub Releases of `skald-so/skald`, served over HTTPS as GitHub serves them,
//! and an HTTPS proxy of the kind `HTTPS_PROXY` names.
//!
//! The stand-in answers, under the base URL of GitHub's REST API that `SKALD_GITHUB_API_URL` sets:
//! - `GET /repos/skald-so/skald/releases/latest`: the newest release that is not a prerelease, or
//!   404 when there is none;
//! - `GET /repos/skald-so/skald/releases`: every release, newest first;
//! - `GET /repos/skald-so/skald/releases/tags/<tag>`: the release of `<tag>`;
//! - `GET /skald-so/skald/releases/download/<tag>/<name>`: a release file's `browser_download_url`,
//!   302 to the file itself, which it serves as `application/octet-stream`.
//!
//! As GitHub's, the API refuses a request without a `User-Agent` with 403. Its release answers are
//! GitHub's, without the assets' `digest`: a release file's checksum is its `<archive>.sha256`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The repository of the releases.
const REPO: &str = "skald-so/skald";

/// Every target a release has an archive of.
const TARGETS: [&str; 6] = [
    "aarch64-apple-darwin",
    "aarch64-pc-windows-msvc",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
];

/// The target of this machine's archive.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const TARGET: &str = "aarch64-apple-darwin";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const TARGET: &str = "x86_64-apple-darwin";
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const TARGET: &str = "aarch64-unknown-linux-gnu";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const TARGET: &str = "x86_64-unknown-linux-gnu";
#[cfg(all(windows, target_arch = "aarch64"))]
pub const TARGET: &str = "aarch64-pc-windows-msvc";
#[cfg(all(windows, target_arch = "x86_64"))]
pub const TARGET: &str = "x86_64-pc-windows-msvc";

/// The name of the release's archive of `target`, as dist names it.
pub fn archive(target: &str) -> String {
    if target.ends_with("-windows-msvc") {
        format!("skald-client-{target}.zip")
    } else {
        format!("skald-client-{target}.tar.gz")
    }
}

/// The name of the checksum of the release file `name`.
pub fn checksum_of(name: &str) -> String {
    format!("{name}.sha256")
}

/// A checksum file as dist writes it: `<hex> *<name>` and a blank line.
pub fn checksum(name: &str, bytes: &[u8]) -> Vec<u8> {
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{hex} *{name}\n\n").into_bytes()
}

/// A GitHub Release of skald: its tag and its files.
pub struct Release {
    tag: String,
    prerelease: bool,
    files: BTreeMap<String, Vec<u8>>,
    cut: Option<String>,
}

impl Release {
    /// The release of `tag` as the release workflow publishes it, a prerelease if `tag` is a beta's:
    /// an archive of each target with its checksum, `install.sh` and `install.ps1`. This machine's
    /// archive is laid out as dist lays it out, and its skald prints `skald <version>`, the version
    /// of `tag`; any other target's archive is no archive of this machine. `work` keeps the files
    /// made for it.
    pub fn new(work: &Path, tag: &str) -> Self {
        let version = tag.strip_prefix('v').expect("a release tag starts with v");
        let mut files = BTreeMap::new();
        for target in TARGETS {
            let name = archive(target);
            let bytes = if target == TARGET {
                archive_of(work, tag, &skald_printing(version))
            } else {
                format!("the {target} archive of {tag}, not this machine's\n").into_bytes()
            };
            files.insert(checksum_of(&name), checksum(&name, &bytes));
            files.insert(name, bytes);
        }
        for installer in ["install.sh", "install.ps1"] {
            files.insert(
                installer.to_owned(),
                format!("# {installer} of {tag}\n").into_bytes(),
            );
        }
        Release {
            tag: tag.to_owned(),
            prerelease: version.contains('-'),
            files,
            cut: None,
        }
    }

    /// The release without its file `name`.
    pub fn without(mut self, name: &str) -> Self {
        assert!(
            self.files.remove(name).is_some(),
            "the release has no {name}"
        );
        self
    }

    /// The release with `bytes` for its file `name`.
    pub fn with(mut self, name: &str, bytes: Vec<u8>) -> Self {
        assert!(
            self.files.insert(name.to_owned(), bytes).is_some(),
            "the release has no {name}"
        );
        self
    }

    /// The release whose file `name` is cut off halfway: the connection closes after half of it.
    pub fn cut(mut self, name: &str) -> Self {
        assert!(self.files.contains_key(name), "the release has no {name}");
        self.cut = Some(name.to_owned());
        self
    }
}

/// An archive of `skald` for this machine, as dist lays it out: `skald.exe` at its root in a zip on
/// Windows, `skald-client-<target>/skald` in a gzipped tar elsewhere; a README beside it.
fn archive_of(work: &Path, tag: &str, skald: &Path) -> Vec<u8> {
    let stage = work.join(format!("stage-{tag}"));
    let out = work.join(format!("{tag}-{}", archive(TARGET)));
    let _ = fs::remove_dir_all(&stage);
    let mut tar;
    if cfg!(windows) {
        fs::create_dir_all(&stage).expect("create the archive's directory");
        fs::copy(skald, stage.join("skald.exe")).expect("copy skald into the archive");
        fs::write(stage.join("README.md"), "# skald\n").expect("write the README");
        // The bsdtar of Windows, not one of a Unix-like toolset on PATH: it writes zip files.
        let system = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        tar = Command::new(Path::new(&system).join(r"System32\tar.exe"));
        tar.arg("-a").arg("-cf").arg(&out).arg("-C").arg(&stage);
        tar.args(["skald.exe", "README.md"]);
    } else {
        let top = format!("skald-client-{TARGET}");
        fs::create_dir_all(stage.join(&top)).expect("create the archive's directory");
        fs::copy(skald, stage.join(&top).join("skald")).expect("copy skald into the archive");
        fs::write(stage.join(&top).join("README.md"), "# skald\n").expect("write the README");
        tar = Command::new("tar");
        // No AppleDouble files of macOS's extended attributes.
        tar.env("COPYFILE_DISABLE", "1");
        tar.arg("-czf").arg(&out).arg("-C").arg(&stage).arg(&top);
    }
    let status = tar.status().expect("run tar");
    assert!(status.success(), "tar failed: {status}");
    fs::read(&out).expect("read the archive")
}

/// A program for this machine that prints `skald <version>`, built once with the toolchain that
/// builds these tests.
fn skald_printing(version: &str) -> PathBuf {
    static BUILDS: Mutex<()> = Mutex::new(());
    let _one_at_a_time = BUILDS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("update-stand-ins")
        .join(version);
    let exe = dir.join(format!("skald{}", std::env::consts::EXE_SUFFIX));
    if exe.is_file() {
        return exe;
    }
    fs::create_dir_all(&dir).expect("create the stand-in's directory");
    let source = dir.join("main.rs");
    fs::write(
        &source,
        format!("fn main() {{\n    println!(\"skald {version}\");\n}}\n"),
    )
    .expect("write the stand-in's source");
    let built = dir.join(format!(
        "skald-{}{}",
        process::id(),
        std::env::consts::EXE_SUFFIX
    ));
    let rustc =
        Path::new(env!("CARGO")).with_file_name(format!("rustc{}", std::env::consts::EXE_SUFFIX));
    let rustc = if rustc.is_file() {
        rustc
    } else {
        PathBuf::from("rustc")
    };
    let status = Command::new(rustc)
        .args(["--edition", "2021", "-C", "debuginfo=0", "-o"])
        .arg(&built)
        .arg(&source)
        .status()
        .expect("run rustc");
    assert!(status.success(), "rustc failed on the stand-in: {status}");
    fs::rename(&built, &exe).expect("move the stand-in into place");
    exe
}

/// A test CA, and a server certificate it issued for 127.0.0.1 and for `releases.skald.test`, a name
/// no resolver knows, made with openssl.
pub struct Certificates {
    /// The CA's certificate, PEM.
    pub ca: PathBuf,
    config: Arc<ServerConfig>,
}

/// The certificates' config: the CA's name, the server's name and the extensions of each.
const OPENSSL_CONFIG: &str = "\
[req]
prompt = no
distinguished_name = ca_name

[ca_name]
CN = skald update test CA

[server_name]
CN = releases.skald.test

[ca]
basicConstraints = critical, CA:true
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash

[server]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = DNS:releases.skald.test, IP:127.0.0.1
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid
";

/// Makes the certificates in `dir`.
pub fn certificates(dir: &Path) -> Certificates {
    fs::create_dir_all(dir).expect("create the certificates' directory");
    fs::write(dir.join("openssl.cnf"), OPENSSL_CONFIG).expect("write the openssl config");
    // The server's request names the server, with the same config.
    fs::write(
        dir.join("server.cnf"),
        OPENSSL_CONFIG.replace(
            "distinguished_name = ca_name",
            "distinguished_name = server_name",
        ),
    )
    .expect("write the openssl config");
    let path = |name: &str| dir.join(name).to_str().expect("a UTF-8 path").to_owned();
    let (config, server_config) = (path("openssl.cnf"), path("server.cnf"));
    openssl(&[
        "req",
        "-x509",
        "-config",
        &config,
        "-extensions",
        "ca",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-days",
        "2",
        "-keyout",
        &path("ca.key"),
        "-out",
        &path("ca.pem"),
    ]);
    openssl(&[
        "req",
        "-new",
        "-config",
        &server_config,
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        &path("server.key"),
        "-out",
        &path("server.csr"),
    ]);
    openssl(&[
        "x509",
        "-req",
        "-in",
        &path("server.csr"),
        "-CA",
        &path("ca.pem"),
        "-CAkey",
        &path("ca.key"),
        "-set_serial",
        "1",
        "-days",
        "2",
        "-extfile",
        &config,
        "-extensions",
        "server",
        "-out",
        &path("server.pem"),
    ]);
    let chain = CertificateDer::pem_file_iter(path("server.pem"))
        .expect("read the server's certificate")
        .collect::<Result<Vec<_>, _>>()
        .expect("parse the server's certificate");
    let key = PrivateKeyDer::from_pem_file(path("server.key")).expect("read the server's key");
    let tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("TLS versions")
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .expect("a TLS config of the server's certificate");
    Certificates {
        ca: dir.join("ca.pem"),
        config: Arc::new(tls),
    }
}

fn openssl(args: &[&str]) {
    let output = Command::new("openssl")
        .args(args)
        .output()
        .expect("run openssl; script/test needs it");
    assert!(
        output.status.success(),
        "openssl {} failed: {}",
        args[0],
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A request the stand-in got.
#[derive(Clone, Debug)]
pub struct Request {
    /// Its target, such as `/repos/skald-so/skald/releases/latest`.
    pub path: String,
    /// Its `Host` header.
    pub host: String,
}

/// The stand-in, serving on a free local port until the tests end.
pub struct StandIn {
    /// The base URL of its API, for `SKALD_GITHUB_API_URL`.
    pub url: String,
    /// The local address it listens on.
    pub addr: SocketAddr,
    connections: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<Request>>>,
}

struct Shared {
    url: String,
    releases: Vec<Release>,
    tls: Arc<ServerConfig>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl StandIn {
    /// Serves `releases`, newest first, over HTTPS with the certificate for `host`: 127.0.0.1, or
    /// `releases.skald.test`, which only a proxy reaches.
    pub fn start(certificates: &Certificates, host: &str, releases: Vec<Release>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let addr = listener.local_addr().expect("the stand-in's address");
        let url = format!("https://{host}:{}", addr.port());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let accepted = Arc::clone(&connections);
        let shared = Arc::new(Shared {
            url: url.clone(),
            releases,
            tls: Arc::clone(&certificates.config),
            requests: Arc::clone(&requests),
        });
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                accepted.fetch_add(1, Ordering::Relaxed);
                let shared = Arc::clone(&shared);
                thread::spawn(move || shared.serve(stream));
            }
        });
        StandIn {
            url,
            addr,
            connections,
            requests,
        }
    }

    /// The connections it accepted so far, whether or not a request came over them.
    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::Relaxed)
    }

    /// The requests so far.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    /// The archives downloaded so far, each as `<tag>/<name>`, in order.
    pub fn archives(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter_map(|request| {
                let file = request
                    .path
                    .strip_prefix(&format!("/{REPO}/releases/download/"))?
                    .to_owned();
                (file.ends_with(".tar.gz") || file.ends_with(".zip")).then_some(file)
            })
            .collect()
    }

    /// Whether a request so far asked the API for the releases.
    pub fn looked_up(&self) -> bool {
        self.requests()
            .iter()
            .any(|request| request.path.starts_with(&format!("/repos/{REPO}/releases")))
    }
}

/// What the stand-in sends: the whole response, or a response cut off after its head and half of
/// its body.
enum Answer {
    Whole(Vec<u8>),
    Cut(Vec<u8>),
}

impl Shared {
    fn serve(&self, stream: TcpStream) {
        let Ok(connection) = ServerConnection::new(Arc::clone(&self.tls)) else {
            return;
        };
        let mut tls = StreamOwned::new(connection, stream);
        let Some(head) = read_head(&mut tls) else {
            return;
        };
        let mut lines = head.split("\r\n");
        let mut start = lines.next().unwrap_or_default().split(' ');
        let method = start.next().unwrap_or_default().to_owned();
        let path = start.next().unwrap_or_default().to_owned();
        let header = |wanted: &str| {
            head.split("\r\n").skip(1).find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case(wanted)
                    .then(|| value.trim().to_owned())
            })
        };
        let host = header("host").unwrap_or_default();
        let agent = header("user-agent").is_some_and(|agent| !agent.is_empty());
        self.requests.lock().unwrap().push(Request {
            path: path.clone(),
            host,
        });
        match self.answer(&method, &path, agent) {
            Answer::Whole(bytes) => {
                let _ = tls.write_all(&bytes);
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
            Answer::Cut(bytes) => {
                let _ = tls.write_all(&bytes);
                let _ = tls.flush();
                let _ = tls.sock.shutdown(Shutdown::Both);
            }
        }
    }

    fn answer(&self, method: &str, target: &str, agent: bool) -> Answer {
        if method != "GET" && method != "HEAD" {
            return Answer::Whole(response("405 Method Not Allowed", &[], b""));
        }
        let head_only = method == "HEAD";
        let path = target.split('?').next().unwrap_or_default();
        if let Some(rest) = path.strip_prefix(&format!("/repos/{REPO}/")) {
            if !agent {
                return json_response(
                    "403 Forbidden",
                    &json!({ "message": "Request forbidden by administrative rules. Please make sure your request has a User-Agent header" }),
                    head_only,
                );
            }
            let found = match rest.trim_end_matches('/') {
                "releases/latest" => self.json_of(|release| !release.prerelease),
                "releases" => Some(Value::Array(
                    (0..self.releases.len()).map(|i| self.json(i)).collect(),
                )),
                rest => rest
                    .strip_prefix("releases/tags/")
                    .and_then(|tag| self.json_of(|release| release.tag == tag)),
            };
            return match found {
                Some(value) => json_response("200 OK", &value, head_only),
                None => json_response(
                    "404 Not Found",
                    &json!({ "message": "Not Found" }),
                    head_only,
                ),
            };
        }
        if let Some(file) = path.strip_prefix(&format!("/{REPO}/releases/download/")) {
            if self.file(file).is_none() {
                return Answer::Whole(response("404 Not Found", &[], b"Not Found"));
            }
            let location = format!("{}/release-assets/{file}", self.url);
            return Answer::Whole(response("302 Found", &[("location", &location)], b""));
        }
        if let Some((release, name, bytes)) = path
            .strip_prefix("/release-assets/")
            .and_then(|file| self.file(file))
        {
            let mut answer = response(
                "200 OK",
                &[("content-type", "application/octet-stream")],
                if head_only { b"" } else { bytes },
            );
            if head_only {
                return Answer::Whole(answer);
            }
            if release.cut.as_deref() == Some(name) {
                answer.truncate(answer.len() - bytes.len() / 2);
                return Answer::Cut(answer);
            }
            return Answer::Whole(answer);
        }
        Answer::Whole(response("404 Not Found", &[], b"Not Found"))
    }

    /// The release and the bytes of `file`, `<tag>/<name>`.
    fn file<'a>(&'a self, file: &'a str) -> Option<(&'a Release, &'a str, &'a [u8])> {
        let (tag, name) = file.split_once('/')?;
        let release = self.releases.iter().find(|release| release.tag == tag)?;
        let bytes = release.files.get(name)?;
        Some((release, name, bytes))
    }

    /// The API's answer of the newest release that `wanted` picks.
    fn json_of(&self, wanted: impl Fn(&Release) -> bool) -> Option<Value> {
        let index = self.releases.iter().position(wanted)?;
        Some(self.json(index))
    }

    /// The API's answer of the release at `index`, as GitHub gives it.
    fn json(&self, index: usize) -> Value {
        let release = &self.releases[index];
        let id = 1000 - index;
        let tag = &release.tag;
        // Newest first: each older release a day earlier.
        let date = format!("2026-11-{:02}T12:00:00Z", 28 - index.min(27));
        let assets: Vec<Value> = release
            .files
            .iter()
            .enumerate()
            .map(|(n, (name, bytes))| {
                json!({
                    "url": format!("{}/repos/{REPO}/releases/assets/{}", self.url, id * 100 + n),
                    "id": id * 100 + n,
                    "name": name,
                    "label": "",
                    "content_type": "application/octet-stream",
                    "state": "uploaded",
                    "size": bytes.len(),
                    "download_count": 0,
                    "created_at": date,
                    "updated_at": date,
                    "browser_download_url": format!("{}/{REPO}/releases/download/{tag}/{name}", self.url),
                })
            })
            .collect();
        json!({
            "url": format!("{}/repos/{REPO}/releases/{id}", self.url),
            "html_url": format!("{}/{REPO}/releases/tag/{tag}", self.url),
            "assets_url": format!("{}/repos/{REPO}/releases/{id}/assets", self.url),
            "id": id,
            "tag_name": tag,
            "target_commitish": "dev",
            "name": tag,
            "body": "",
            "draft": false,
            "prerelease": release.prerelease,
            "immutable": true,
            "created_at": date,
            "published_at": date,
            "assets": assets,
            "tarball_url": format!("{}/repos/{REPO}/tarball/{tag}", self.url),
            "zipball_url": format!("{}/repos/{REPO}/zipball/{tag}", self.url),
        })
    }
}

/// A response with `status`, `headers` and `body`, closing the connection.
fn response(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut head = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn json_response(status: &str, value: &Value, head_only: bool) -> Answer {
    let body = serde_json::to_vec(value).expect("JSON");
    let mut answer = response(
        status,
        &[("content-type", "application/json; charset=utf-8")],
        &body,
    );
    if head_only {
        answer.truncate(answer.len() - body.len());
    }
    Answer::Whole(answer)
}

/// The head of a request on `stream`, up to its blank line, read a byte at a time so that nothing
/// after it is consumed; `None` if the stream ends or fails first.
fn read_head(stream: &mut impl Read) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 64 * 1024 || stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    String::from_utf8(head).ok()
}

/// An HTTP proxy that tunnels `CONNECT` to one authority, `<host>:<port>`, to a local address, and
/// refuses any other request with 403.
pub struct Proxy {
    /// Its URL, for `HTTPS_PROXY`.
    pub url: String,
    connects: Arc<Mutex<Vec<String>>>,
}

impl Proxy {
    pub fn start(authority: String, to: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let url = format!(
            "http://{}",
            listener.local_addr().expect("the proxy's address")
        );
        let connects = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&connects);
        let authority = Arc::new(authority);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let recorded = Arc::clone(&recorded);
                let authority = Arc::clone(&authority);
                thread::spawn(move || tunnel(client, &authority, to, &recorded));
            }
        });
        Proxy { url, connects }
    }

    /// The request lines the proxy got, such as `CONNECT releases.skald.test:443 HTTP/1.1`.
    pub fn requests(&self) -> Vec<String> {
        self.connects.lock().unwrap().clone()
    }
}

fn tunnel(mut client: TcpStream, authority: &str, to: SocketAddr, recorded: &Mutex<Vec<String>>) {
    let Some(head) = read_head(&mut client) else {
        return;
    };
    let line = head.split("\r\n").next().unwrap_or_default().to_owned();
    recorded.lock().unwrap().push(line.clone());
    let upstream = (line == format!("CONNECT {authority} HTTP/1.1"))
        .then(|| TcpStream::connect(to).ok())
        .flatten();
    let Some(mut upstream) = upstream else {
        let _ = client
            .write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        return;
    };
    if client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .is_err()
    {
        return;
    }
    let (Ok(mut client_out), Ok(mut upstream_in)) = (client.try_clone(), upstream.try_clone())
    else {
        return;
    };
    thread::spawn(move || {
        let _ = io::copy(&mut client_out, &mut upstream_in);
        let _ = upstream_in.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut upstream, &mut client);
    let _ = client.shutdown(Shutdown::Write);
}
