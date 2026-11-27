//! Loopback process fixture. The `skald-a2a-fixture` Cargo example reads one JSON
//! document from stdin through EOF. It starts the real HTTP handler and prints
//! its loopback HTTP origin followed by a newline on stdout. The test owns the
//! HTTPS front and the test CA. No configuration enters the production CLI.
//!
//! Config fields are `publicOrigin`, `skaldCaFile`, `routes` (public alias to
//! Skald binding), `credentials` (client, bearer, routes, revoked), and `tasks`
//! (task/context correlation fixtures only). Each queue binding has
//! `skaldOrigin`, `space`, `channel`, `queue`, `serviceToken`, and
//! `signingKey` (base64url without padding, or null for a missing secret).

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use serde_json::{Value, json};
use skald_proto::QueueId;
use skald_server::a2a::tls_config;
use ureq::tls::Certificate;
use ureq::{Agent, Body};
use url::Url;

use crate::common::sign_in::{CLIENT_SECRET, SECRET_ENV, github_settings};
use crate::common::{SERVER, Server, collect, devices, on_free_port};
use crate::front::Front;
use crate::kit::World;
use crate::tls::Ca;

const TIMEOUT: Duration = Duration::from_secs(15);

pub struct Lab {
    _files: Files,
    pub upstream: Front,
    _skald_server: Server,
    incoming: Front,
    pub public: String,
    pub skald: String,
    ca: PathBuf,
    client: Agent,
}

impl Lab {
    pub fn new(world: &World) -> Self {
        let files = Files::new();
        let ca = Ca::new(&files.0);
        let certificate = ca.issue("IP:127.0.0.1");
        let incoming = Front::bind(vec![Arc::clone(&certificate)]);
        let upstream = Front::bind(vec![certificate]);
        let skald = upstream.url("127.0.0.1");
        // Proofs sign the configured public origin. Serve the same database under the TLS
        // front's address while the member fixtures keep their original server and sign-in.
        let settings = github_settings(&skald, &world.stage.f.team.flow.stand.github);
        let skald_server = on_free_port(TIMEOUT, |port| {
            Server::try_launch(
                world.db(),
                &format!("127.0.0.1:{port}"),
                &settings,
                &[(SECRET_ENV, CLIENT_SECRET)],
            )
        });
        upstream.forward_to(skald_server.base().trim_start_matches("http://"));
        let root = CertificateDer::from_pem_file(&ca.cert).expect("test CA");
        let roots = vec![Certificate::from_der(root.as_ref()).to_owned()];
        let client = Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(TIMEOUT))
            .tls_config(tls_config(roots))
            .build()
            .new_agent();
        Self {
            _files: files,
            public: incoming.url("127.0.0.1"),
            skald,
            upstream,
            _skald_server: skald_server,
            incoming,
            ca: ca.cert,
            client,
        }
    }

    pub fn queue_binding(&self, world: &World, queue: QueueId, token: Option<&str>) -> Value {
        json!({
            "skaldOrigin": self.skald,
            "space": world.space.to_string(),
            "channel": world.channel.to_string(),
            "queue": queue.to_string(),
            "serviceToken": token,
            "signingKey": devices::b64(&world.stage.bot.signing.secret),
        })
    }

    pub fn start(
        &self,
        example: &Path,
        routes: Value,
        credentials: Value,
        tasks: Value,
    ) -> Process {
        let config = json!({
            "publicOrigin": self.public,
            "skaldCaFile": self.ca,
            "routes": routes,
            "credentials": credentials,
            "tasks": tasks,
        });
        let (process, origin) = Process::start(example, &config);
        let origin = Url::parse(&origin).expect("fixture example prints its loopback origin");
        assert_eq!(origin.scheme(), "http");
        assert_eq!(origin.host_str(), Some("127.0.0.1"));
        assert_eq!(origin.path(), "/");
        let port = origin.port().expect("fixture loopback port");
        self.incoming.forward_to(&format!("127.0.0.1:{port}"));
        process
    }

    pub fn card(&self) -> Answer {
        Answer::read(
            self.client
                .get(format!("{}{}", self.public, crate::CARD))
                .call(),
        )
    }

    pub fn rpc(&self, route: &str, token: Option<&str>, version: &str, request: &Value) -> Answer {
        let mut post = self
            .client
            .post(format!("{}/a2a/{route}", self.public))
            .header("content-type", "application/json")
            .header("A2A-Version", version);
        if let Some(token) = token {
            post = post.header("authorization", format!("Bearer {token}"));
        }
        Answer::read(post.send(request.to_string().as_bytes()))
    }
}

pub struct Answer {
    pub status: u16,
    pub bytes: Vec<u8>,
}

impl Answer {
    fn read(answer: Result<ureq::http::Response<Body>, ureq::Error>) -> Self {
        let mut answer = answer.expect("local gateway HTTPS request");
        Self {
            status: answer.status().as_u16(),
            bytes: answer.body_mut().read_to_vec().expect("gateway body"),
        }
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).expect("gateway JSON")
    }
}

pub fn example() -> PathBuf {
    let name = format!("skald-a2a-fixture{}", env::consts::EXE_SUFFIX);
    let path = Path::new(SERVER).with_file_name("examples").join(name);
    assert!(
        path.is_file(),
        "the isolated A2A route fixture is unavailable"
    );
    path
}

pub struct Process {
    child: Child,
    readers: Vec<JoinHandle<()>>,
    output: Arc<Mutex<String>>,
}

impl Process {
    fn start(example: &Path, config: &Value) -> (Self, String) {
        let child = Command::new(example)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the isolated A2A fixture example");
        let mut process = Self {
            child,
            readers: Vec::new(),
            output: Arc::default(),
        };
        let (ready, origin) = mpsc::sync_channel(1);
        let output = Arc::clone(&process.output);
        let stdout = process.child.stdout.take().expect("fixture stdout");
        process.readers.push(thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            let mut line = String::new();
            let read = stdout.read_line(&mut line);
            let _ = ready.send(read.map(|_| line.trim_end().to_owned()));
            let mut remaining = String::new();
            let _ = stdout.read_to_string(&mut remaining);
            output.lock().expect("fixture logs").push_str(&line);
            output.lock().expect("fixture logs").push_str(&remaining);
        }));
        let stderr = process.child.stderr.take().expect("fixture stderr");
        process.readers.push(collect(stderr, &process.output));
        process
            .child
            .stdin
            .take()
            .expect("fixture stdin")
            .write_all(config.to_string().as_bytes())
            .expect("configure the isolated fixture");
        let origin = origin
            .recv_timeout(TIMEOUT)
            .expect("the isolated gateway fixture starts")
            .expect("fixture readiness");
        (process, origin)
    }

    pub fn stop(mut self) -> String {
        self.close();
        std::mem::take(&mut *self.output.lock().expect("fixture logs"))
    }

    fn close(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for reader in self.readers.drain(..) {
            reader.join().expect("read fixture logs");
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.close();
    }
}

struct Files(PathBuf);

impl Files {
    fn new() -> Self {
        let unique = crate::common::hex(&devices::random());
        let path = env::temp_dir().join(format!("skald-a2a-{unique}"));
        fs::create_dir(&path).expect("fixture directory");
        Self(path)
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
