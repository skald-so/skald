//! The server's attachment storage: a directory, or a bucket on an S3-compatible server. At start
//! the server names the storage it uses, never the bucket's secret key, then writes an object of
//! its own there, reads it back and deletes it; when a step fails, it exits with an error naming the
//! storage and the step, and never listens. A storage setting it cannot take stops it too, naming
//! the setting. Without either storage it starts with none, as every other test's server does.
//!
//! The settings, each an argument or its environment variable:
//! - `--attachments-dir <DIR>`, `SKALD_ATTACHMENTS_DIR`: the directory;
//! - `--attachments-bucket <NAME>`, `SKALD_ATTACHMENTS_BUCKET`: the bucket, which replaces the
//!   directory;
//! - `--attachments-bucket-endpoint <URL>`, `SKALD_ATTACHMENTS_BUCKET_ENDPOINT`: the S3 endpoint,
//!   https, or http on a loopback address; the bucket is addressed in the path;
//! - `--attachments-bucket-region <REGION>`, `SKALD_ATTACHMENTS_BUCKET_REGION`;
//! - `--attachments-bucket-access-key-id <ID>`, `SKALD_ATTACHMENTS_BUCKET_ACCESS_KEY_ID`;
//! - the secret key only from the environment: `SKALD_ATTACHMENTS_BUCKET_SECRET_KEY`.
//!
//! The bucket's server is the local S3-compatible server `script/test` starts, at
//! `SKALD_TEST_S3_URL` with `SKALD_TEST_S3_REGION`, `SKALD_TEST_S3_ACCESS_KEY_ID` and
//! `SKALD_TEST_S3_SECRET_KEY`. The tests reach it through a proxy of their own, which keeps the
//! requests the server sends it.

mod common;
// The client's test CA and its TLS endpoint before a plain HTTP server.
#[allow(dead_code)]
#[path = "../../client/tests/https/front.rs"]
mod front;
#[allow(dead_code)]
#[path = "../../client/tests/common/tls.rs"]
mod tls;

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, atomic};
use std::time::{Duration, Instant};
use std::{env, fs, process, thread};

use common::{SERVER, TestDb, collect, get_url};
use front::Front;
use tls::Ca;

const DIR: &str = "--attachments-dir";
const DIR_ENV: &str = "SKALD_ATTACHMENTS_DIR";
const BUCKET: &str = "--attachments-bucket";
const BUCKET_ENV: &str = "SKALD_ATTACHMENTS_BUCKET";
const ENDPOINT: &str = "--attachments-bucket-endpoint";
const ENDPOINT_ENV: &str = "SKALD_ATTACHMENTS_BUCKET_ENDPOINT";
const REGION: &str = "--attachments-bucket-region";
const REGION_ENV: &str = "SKALD_ATTACHMENTS_BUCKET_REGION";
const ACCESS_KEY_ID: &str = "--attachments-bucket-access-key-id";
const ACCESS_KEY_ID_ENV: &str = "SKALD_ATTACHMENTS_BUCKET_ACCESS_KEY_ID";
const SECRET_KEY_ENV: &str = "SKALD_ATTACHMENTS_BUCKET_SECRET_KEY";

/// The line the server prints once it listens.
const LISTENING: &str = "skald-server: listening on ";

/// How long a server may take to start or to give up.
const START_TIMEOUT: Duration = Duration::from_secs(90);

// --- A directory ---

/// With a writable directory, as an argument or from the environment, the server starts, names
/// the directory and leaves it as it was: its object is gone, what was there stays.
#[test]
fn a_writable_directory_is_named_and_left_as_it_was() {
    for from_env in [false, true] {
        let dir = TempDir::new("writable");
        fs::write(dir.path().join("kept"), b"an attachment").expect("write a file");
        let before = entries(dir.path());
        let path = dir.text();

        let (args, envs) = if from_env {
            (vec![], vec![(DIR_ENV, path.as_str())])
        } else {
            (vec![DIR, path.as_str()], vec![])
        };
        let mut server = start(&args, &envs);

        server.assert_healthy();
        assert_eq!(
            entries(dir.path()),
            before,
            "from the environment: {from_env}"
        );
        let output = server.stop();
        assert!(
            output.contains(&path),
            "the directory is not named: {output}"
        );
        assert_eq!(
            fs::read(dir.path().join("kept")).expect("read the file"),
            b"an attachment"
        );
    }
}

/// A directory that does not exist stops the server, naming it and the step or the setting; the
/// server does not create it.
#[test]
fn a_missing_directory_stops_the_server() {
    let dir = TempDir::new("missing");
    let missing = dir.path().join("attachments");
    let missing_text = missing.to_str().expect("a UTF-8 path").to_owned();

    let output = stopped(&[DIR, &missing_text], &[]);

    assert!(output.contains(&missing_text), "{output}");
    assert!(
        names_step(&output, "writ") || names_setting(&output, DIR, DIR_ENV),
        "neither the step nor the setting is named: {output}"
    );
    assert!(!missing.exists(), "the server created the directory");
}

/// A directory the server cannot write in stops it, naming the directory and the write.
#[cfg(unix)]
#[test]
fn a_read_only_directory_stops_the_server() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("read-only");
    let attachments = dir.path().join("attachments");
    fs::create_dir(&attachments).expect("create the directory");
    fs::set_permissions(&attachments, fs::Permissions::from_mode(0o500))
        .expect("make the directory read-only");
    let path = attachments.to_str().expect("a UTF-8 path").to_owned();

    let output = stopped(&[DIR, &path], &[]);

    fs::set_permissions(&attachments, fs::Permissions::from_mode(0o700))
        .expect("make the directory writable again");
    assert!(output.contains(&path), "{output}");
    assert!(
        names_step(&output, "writ"),
        "the write is not named: {output}"
    );
    assert_eq!(entries(&attachments), Vec::<String>::new());
}

// --- A bucket ---

/// With a bucket it can use, given as arguments or from the environment, the server starts and
/// names the bucket and its endpoint; the bucket's server sees it write an object of its own, read
/// it back and delete it, in path-style requests, and the bucket is left empty. Its output never
/// shows the secret key.
#[test]
fn a_bucket_is_named_written_read_and_deleted_and_left_empty() {
    let s3 = S3::from_env();
    for from_env in [false, true] {
        let Stand {
            bucket,
            proxy,
            settings,
        } = s3.stand();
        let (args, envs) = settings.split(from_env);
        let mut server = start(&args, &envs);

        server.assert_healthy();
        assert_wrote_read_deleted(&proxy.requests(), &bucket.name);
        assert_eq!(bucket.keys(), Vec::<String>::new(), "objects left behind");
        let output = server.stop();
        assert!(
            output.contains(&bucket.name),
            "the bucket is not named: {output}"
        );
        assert!(
            output.contains(&proxy.address()),
            "the endpoint is not named: {output}"
        );
        assert_shows_no_secret(&output, &[&s3.secret_key]);
    }
}

/// A bucket replaces the directory: with both, the server uses the bucket, leaves the directory
/// as it was and names only the bucket; a directory that does not exist does not stop it.
#[test]
fn a_bucket_replaces_the_directory() {
    let s3 = S3::from_env();
    let dir = TempDir::new("replaced");
    fs::write(dir.path().join("kept"), b"an attachment").expect("write a file");
    let missing = dir.path().join("missing");
    for path in [
        dir.text(),
        missing.to_str().expect("a UTF-8 path").to_owned(),
    ] {
        let Stand {
            bucket,
            proxy,
            settings,
        } = s3.stand();
        let (mut args, envs) = settings.split(false);
        args.extend([DIR, path.as_str()]);
        let mut server = start(&args, &envs);

        server.assert_healthy();
        assert_wrote_read_deleted(&proxy.requests(), &bucket.name);
        let output = server.stop();
        assert!(output.contains(&bucket.name), "{output}");
        assert!(!output.contains(&path), "the directory is named: {output}");
        assert_eq!(entries(dir.path()), vec!["kept".to_owned()]);
    }
}

/// A secret key the bucket's server refuses stops the server, naming the bucket and the write;
/// its output shows neither that key nor the right one.
#[test]
fn a_wrong_secret_key_stops_the_server() {
    let s3 = S3::from_env();
    let Stand {
        bucket,
        mut settings,
        ..
    } = s3.stand();
    let wrong = "wrong-secret-key-3c9e51d07b2a4f86";
    settings.secret_key = Some(wrong.to_owned());
    let (args, envs) = settings.split(false);

    let output = stopped(&args, &envs);

    assert_refused_at_write(&output, &bucket.name, &[wrong, &s3.secret_key]);
    assert_eq!(bucket.keys(), Vec::<String>::new());
}

/// A bucket that does not exist stops the server, naming it and the write.
#[test]
fn a_missing_bucket_stops_the_server() {
    let s3 = S3::from_env();
    let name = unique_bucket_name();
    let proxy = Recorder::start(s3.address());
    let settings = s3.settings(&name, &proxy.url());
    let (args, envs) = settings.split(false);

    let output = stopped(&args, &envs);

    assert_refused_at_write(&output, &name, &[&s3.secret_key]);
}

/// A bucket's server that does not answer stops the server, naming the bucket and the write.
#[test]
fn a_stopped_bucket_server_stops_the_server() {
    let s3 = S3::from_env();
    let bucket = s3.bucket();
    let closed = format!("http://127.0.0.1:{}", closed_port());
    let settings = s3.settings(&bucket.name, &closed);
    let (args, envs) = settings.split(false);

    let output = stopped(&args, &envs);

    assert_refused_at_write(&output, &bucket.name, &[&s3.secret_key]);
}

/// An http endpoint off a loopback address stops the server, naming the setting, before it sends
/// the bucket's server anything; a name is not known to be loopback, though `localhost` reaches
/// it.
#[test]
fn an_http_endpoint_off_loopback_stops_the_server() {
    let s3 = S3::from_env();
    let bucket = s3.bucket();
    let proxy = Recorder::start(s3.address());
    let localhost = format!("http://localhost:{}", proxy.port);
    for endpoint in ["http://s3.example", "http://192.0.2.10:9000", &localhost] {
        for from_env in [false, true] {
            let settings = s3.settings(&bucket.name, endpoint);
            let (args, envs) = settings.split(from_env);

            let output = stopped(&args, &envs);

            assert!(
                names_setting(&output, ENDPOINT, ENDPOINT_ENV),
                "{endpoint}: the setting is not named: {output}"
            );
            assert_shows_no_secret(&output, &[&s3.secret_key]);
        }
    }
    assert_eq!(proxy.requests(), Vec::<(String, String)>::new());
}

/// A bucket without its secret key in the environment stops the server, naming the setting,
/// before it sends the bucket's server anything.
#[test]
fn a_bucket_without_its_secret_key_stops_the_server() {
    let s3 = S3::from_env();
    let Stand {
        proxy,
        mut settings,
        ..
    } = s3.stand();
    settings.secret_key = None;
    let (args, envs) = settings.split(false);

    let output = stopped(&args, &envs);

    assert!(output.contains(SECRET_KEY_ENV), "{output}");
    assert_eq!(proxy.requests(), Vec::<(String, String)>::new());
}

/// An https endpoint is checked against the platform's root certificates, which `SSL_CERT_FILE`
/// replaces: with the endpoint's CA there, the server uses the bucket; without it, the server
/// stops, naming the bucket and the write.
#[test]
fn an_https_endpoint_is_checked_against_the_platform_roots() {
    let s3 = S3::from_env();
    let dir = TempDir::new("ca");
    let ca = Ca::new(dir.path());
    let front = Front::start(&s3.address(), vec![ca.issue("IP:127.0.0.1")]);
    let endpoint = front.url("127.0.0.1");
    let ca_file = ca.cert.to_str().expect("a UTF-8 path").to_owned();

    let bucket = s3.bucket();
    let settings = s3.settings(&bucket.name, &endpoint);
    let (args, mut envs) = settings.split(false);
    envs.push(("SSL_CERT_FILE", ca_file.as_str()));
    let mut server = start(&args, &envs);
    server.assert_healthy();
    assert_wrote_read_deleted(&requests_in(&front.plain()), &bucket.name);
    assert_eq!(bucket.keys(), Vec::<String>::new());
    assert_shows_no_secret(&server.stop(), &[&s3.secret_key]);

    let untrusted = s3.bucket();
    let settings = s3.settings(&untrusted.name, &endpoint);
    let (args, envs) = settings.split(false);
    let output = stopped(&args, &envs);
    assert_refused_at_write(&output, &untrusted.name, &[&s3.secret_key]);
    assert_eq!(untrusted.keys(), Vec::<String>::new());
}

// --- Helpers ---

/// Whether `output` names the step `stem`, such as `writ` for write and writing.
fn names_step(output: &str, stem: &str) -> bool {
    output.to_lowercase().contains(stem)
}

/// Whether `output` names a setting, by its argument or its environment variable.
fn names_setting(output: &str, arg: &str, env: &str) -> bool {
    output.contains(arg) || output.contains(env)
}

/// Checks that `output`, a server's that stopped, names `bucket` and the write and shows none of
/// `secrets`.
#[track_caller]
fn assert_refused_at_write(output: &str, bucket: &str, secrets: &[&str]) {
    assert!(output.contains(bucket), "the bucket is not named: {output}");
    assert!(
        names_step(output, "writ"),
        "the write is not named: {output}"
    );
    assert_shows_no_secret(output, secrets);
}

#[track_caller]
fn assert_shows_no_secret(output: &str, secrets: &[&str]) {
    for secret in secrets {
        assert!(
            !output.contains(secret),
            "the output shows {secret:?}: {output}"
        );
    }
}

/// Checks that `requests` hold a write of an object in `bucket`, addressed in the path, a read of
/// the same object after it and its delete after that.
#[track_caller]
fn assert_wrote_read_deleted(requests: &[(String, String)], bucket: &str) {
    let prefix = format!("/{bucket}/");
    let at = |method: &str, path: &str, from: usize| {
        requests[from..]
            .iter()
            .position(|(m, p)| m == method && p == path)
            .map(|found| from + found)
    };
    let done = requests.iter().enumerate().any(|(put, (method, path))| {
        method == "put"
            && path.len() > prefix.len()
            && path.starts_with(&prefix)
            && at("get", path, put + 1)
                .and_then(|get| at("delete", path, get + 1))
                .is_some()
    });
    assert!(
        done,
        "no write, read and delete of one object in {bucket}: {requests:?}"
    );
}

/// The requests in `sent`, what clients sent: each request's method and path without its query, in
/// lowercase. A request line ends a line of `sent`; it follows the body of the request before it
/// on the connection without a line break.
fn requests_in(sent: &str) -> Vec<(String, String)> {
    let methods = ["delete", "head", "post", "put", "get"];
    sent.to_lowercase()
        .split("\r\n")
        .filter_map(|line| {
            let line = line
                .strip_suffix(" http/1.1")
                .or_else(|| line.strip_suffix(" http/1.0"))?;
            let (before, target) = line.rsplit_once(' ')?;
            let method = methods.iter().find(|method| before.ends_with(**method))?;
            let path = target.split('?').next()?;
            path.starts_with('/')
                .then(|| ((*method).to_owned(), path.to_owned()))
        })
        .collect()
}

/// The names in `dir`, sorted.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("read the directory")
        .map(|entry| {
            entry
                .expect("a directory entry")
                .file_name()
                .into_string()
                .expect("a UTF-8 name")
        })
        .collect();
    names.sort();
    names
}

/// A loopback port nothing listens on.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    listener.local_addr().expect("local address").port()
}

/// A new directory under the system's temporary directory; removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: atomic::AtomicUsize = atomic::AtomicUsize::new(0);
        let dir = env::temp_dir().join(format!(
            "skald-attachments-{name}-{}-{}",
            process::id(),
            NEXT.fetch_add(1, atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create a temporary directory");
        // The real path: on macOS the temporary directory is behind a symlink.
        TempDir(dir.canonicalize().expect("the directory's real path"))
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn text(&self) -> String {
        self.0.to_str().expect("a UTF-8 path").to_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The S3-compatible server `script/test` starts.
struct S3 {
    url: String,
    region: String,
    access_key_id: String,
    secret_key: String,
}

/// The bucket settings of a server, `secret_key` its environment variable's value or unset.
struct BucketSettings {
    bucket: String,
    endpoint: String,
    region: String,
    access_key_id: String,
    secret_key: Option<String>,
}

impl BucketSettings {
    /// The settings as arguments, or as environment variables if `from_env`; the secret key always
    /// in the environment.
    fn split(&self, from_env: bool) -> (Vec<&str>, Vec<(&str, &str)>) {
        let values = [
            (BUCKET, BUCKET_ENV, self.bucket.as_str()),
            (ENDPOINT, ENDPOINT_ENV, self.endpoint.as_str()),
            (REGION, REGION_ENV, self.region.as_str()),
            (
                ACCESS_KEY_ID,
                ACCESS_KEY_ID_ENV,
                self.access_key_id.as_str(),
            ),
        ];
        let (mut args, mut envs) = (Vec::new(), Vec::new());
        for (arg, env, value) in values {
            if from_env {
                envs.push((env, value));
            } else {
                args.extend([arg, value]);
            }
        }
        if let Some(secret) = &self.secret_key {
            envs.push((SECRET_KEY_ENV, secret.as_str()));
        }
        (args, envs)
    }
}

impl S3 {
    fn from_env() -> Self {
        let var = |name: &str| {
            env::var(name)
                .unwrap_or_else(|_| panic!("{name} is not set: run the tests with script/test"))
        };
        S3 {
            url: var("SKALD_TEST_S3_URL"),
            region: var("SKALD_TEST_S3_REGION"),
            access_key_id: var("SKALD_TEST_S3_ACCESS_KEY_ID"),
            secret_key: var("SKALD_TEST_S3_SECRET_KEY"),
        }
    }

    /// Its address, such as `127.0.0.1:41234`.
    fn address(&self) -> String {
        self.url
            .strip_prefix("http://")
            .expect("an http URL")
            .trim_end_matches('/')
            .to_owned()
    }

    /// The settings of `bucket` at `endpoint` with its region and credentials.
    fn settings(&self, bucket: &str, endpoint: &str) -> BucketSettings {
        BucketSettings {
            bucket: bucket.to_owned(),
            endpoint: endpoint.to_owned(),
            region: self.region.clone(),
            access_key_id: self.access_key_id.clone(),
            secret_key: Some(self.secret_key.clone()),
        }
    }

    /// A new empty bucket and a proxy before the S3 server, with the settings of the bucket at the
    /// proxy.
    fn stand(&self) -> Stand<'_> {
        let bucket = self.bucket();
        let proxy = Recorder::start(self.address());
        let settings = self.settings(&bucket.name, &proxy.url());
        Stand {
            bucket,
            proxy,
            settings,
        }
    }

    /// A new empty bucket; removed on drop.
    fn bucket(&self) -> Bucket<'_> {
        let name = unique_bucket_name();
        let config = format!(
            "<CreateBucketConfiguration><LocationConstraint>{}</LocationConstraint>\
             </CreateBucketConfiguration>",
            self.region
        );
        let (status, body) = self.request("PUT", &format!("/{name}"), Some(&config));
        assert_eq!(status, 200, "create the bucket {name}: {body}");
        Bucket { s3: self, name }
    }

    /// A request signed with its credentials, by curl; gives the status and the body.
    fn request(&self, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
        let mut command = Command::new("curl");
        command
            .args(["--silent", "--show-error", "--request", method])
            .args(["--aws-sigv4", &format!("aws:amz:{}:s3", self.region)])
            .args([
                "--user",
                &format!("{}:{}", self.access_key_id, self.secret_key),
            ])
            .args(["--write-out", "\n%{http_code}"]);
        if let Some(body) = body {
            command.args(["--data-binary", body]);
        }
        let output = command
            .arg(format!("{}{path}", self.url.trim_end_matches('/')))
            .output()
            .expect("run curl; script/test needs it");
        assert!(
            output.status.success(),
            "curl {method} {path}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let (body, status) = text.rsplit_once('\n').expect("curl's status line");
        (status.parse().expect("an HTTP status"), body.to_owned())
    }
}

fn unique_bucket_name() -> String {
    static NEXT: atomic::AtomicUsize = atomic::AtomicUsize::new(0);
    format!(
        "skald-test-{}-{}",
        process::id(),
        NEXT.fetch_add(1, atomic::Ordering::Relaxed)
    )
}

/// A bucket, a proxy before its server and the settings of the bucket at the proxy.
struct Stand<'a> {
    bucket: Bucket<'a>,
    proxy: Recorder,
    settings: BucketSettings,
}

/// A bucket on the S3-compatible server.
struct Bucket<'a> {
    s3: &'a S3,
    name: String,
}

impl Bucket<'_> {
    /// The keys of the objects in it.
    fn keys(&self) -> Vec<String> {
        let (status, body) = self
            .s3
            .request("GET", &format!("/{}?list-type=2", self.name), None);
        assert_eq!(status, 200, "list {}: {body}", self.name);
        body.split("<Key>")
            .skip(1)
            .filter_map(|rest| rest.split_once("</Key>").map(|(key, _)| key.to_owned()))
            .collect()
    }
}

impl Drop for Bucket<'_> {
    fn drop(&mut self) {
        if let Ok(keys) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.keys())) {
            for key in keys {
                let _ = self
                    .s3
                    .request("DELETE", &format!("/{}/{key}", self.name), None);
            }
        }
        let _ = self.s3.request("DELETE", &format!("/{}", self.name), None);
    }
}

/// A TCP proxy on a free loopback port before a server, which keeps everything its clients send.
struct Recorder {
    port: u16,
    sent: Arc<Mutex<Vec<u8>>>,
}

impl Recorder {
    /// Forwards each connection to `target`, such as `127.0.0.1:41234`, both ways.
    fn start(target: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let port = listener.local_addr().expect("the proxy's address").port();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&sent);
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let (target, kept) = (target.clone(), Arc::clone(&kept));
                thread::spawn(move || forward(client, &target, &kept));
            }
        });
        Recorder { port, sent }
    }

    /// Its address, such as `127.0.0.1:41235`.
    fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    fn url(&self) -> String {
        format!("http://{}", self.address())
    }

    /// The requests its clients sent so far, as [`requests_in`] gives them.
    fn requests(&self) -> Vec<(String, String)> {
        requests_in(&String::from_utf8_lossy(&self.sent.lock().unwrap()))
    }
}

/// Forwards `client` to `target` both ways, keeping what the client sends in `sent`.
fn forward(mut client: TcpStream, target: &str, sent: &Mutex<Vec<u8>>) {
    let Ok(mut server) = TcpStream::connect(target) else {
        return;
    };
    let (Ok(mut answers), Ok(mut to_client)) = (server.try_clone(), client.try_clone()) else {
        return;
    };
    thread::spawn(move || {
        let mut buffer = [0; 16 * 1024];
        while let Ok(count @ 1..) = answers.read(&mut buffer) {
            if to_client.write_all(&buffer[..count]).is_err() {
                break;
            }
        }
        let _ = to_client.shutdown(Shutdown::Write);
    });
    let mut buffer = [0; 16 * 1024];
    while let Ok(count @ 1..) = client.read(&mut buffer) {
        sent.lock().unwrap().extend_from_slice(&buffer[..count]);
        if server.write_all(&buffer[..count]).is_err() {
            break;
        }
    }
    let _ = server.shutdown(Shutdown::Write);
}

/// Starts the server with `args` and `envs` on a database it can reach; gives everything it
/// printed once it exited, checking that it failed and never listened.
#[track_caller]
fn stopped(args: &[&str], envs: &[(&str, &str)]) -> String {
    match Launch::new(args, envs).run() {
        Launched::Exited { success, output } => {
            assert!(!success, "the server exited with success: {output}");
            assert!(!output.contains(LISTENING), "the server listened: {output}");
            output
        }
        Launched::Started(mut server) => panic!("the server started: {}", server.stop()),
    }
}

/// Starts the server with `args` and `envs` on a database it can reach; waits until it listens.
#[track_caller]
fn start(args: &[&str], envs: &[(&str, &str)]) -> Running {
    Launch::new(args, envs).started()
}

/// A `skald-server` being started on a test database of its own, listening on a free loopback
/// port.
struct Launch {
    child: Child,
    /// Dropped after the server is killed.
    _db: TestDb,
    output: Arc<Mutex<String>>,
    readers: Vec<thread::JoinHandle<()>>,
}

enum Launched {
    Started(Running),
    Exited { success: bool, output: String },
}

/// A started server; killed on drop.
struct Running {
    launch: Launch,
    base: String,
}

impl Launch {
    fn new(args: &[&str], envs: &[(&str, &str)]) -> Self {
        let db = TestDb::create();
        let url = db.url();
        let mut child = Command::new(SERVER)
            .args(["--listen", "127.0.0.1:0", "--database-url", &url])
            .args(args)
            // The platform's root certificates, unless `envs` replace them.
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR")
            .envs(envs.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start skald-server");
        let output = Arc::new(Mutex::new(String::new()));
        let readers = vec![
            collect(child.stdout.take().expect("stdout is piped"), &output),
            collect(child.stderr.take().expect("stderr is piped"), &output),
        ];
        Launch {
            child,
            _db: db,
            output,
            readers,
        }
    }

    /// Waits until the server listens or exits.
    fn run(mut self) -> Launched {
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            let printed = self.output.lock().unwrap().clone();
            if let Some(line) = printed.lines().find(|line| line.starts_with(LISTENING)) {
                let base = line[LISTENING.len()..].trim().to_owned();
                return Launched::Started(Running { launch: self, base });
            }
            if let Some(status) = self.child.try_wait().expect("wait for skald-server") {
                for reader in self.readers.drain(..) {
                    reader.join().expect("read skald-server output");
                }
                let output = self.output.lock().unwrap().clone();
                return Launched::Exited {
                    success: status.success(),
                    output,
                };
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!(
                    "skald-server neither listened nor exited within {START_TIMEOUT:?}: {printed}"
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits until the server listens; panics if it exits first.
    #[track_caller]
    fn started(self) -> Running {
        match self.run() {
            Launched::Started(server) => server,
            Launched::Exited { output, .. } => panic!("the server did not start: {output}"),
        }
    }
}

impl Running {
    #[track_caller]
    fn assert_healthy(&self) {
        let health = get_url(&format!("{}/health", self.base), &[]);
        assert_eq!(health.status, 200);
        assert_eq!(health.body, br#"{"status":"ok"}"#);
    }

    /// Stops the server; gives everything it printed.
    fn stop(&mut self) -> String {
        let _ = self.launch.child.kill();
        let _ = self.launch.child.wait();
        for reader in self.launch.readers.drain(..) {
            reader.join().expect("read skald-server output");
        }
        self.launch.output.lock().unwrap().clone()
    }
}

impl Drop for Launch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
