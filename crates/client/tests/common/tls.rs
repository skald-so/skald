//! A test CA and the server certificates it issues, made with openssl, each with the TLS config of a
//! rustls server that presents it.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// The CA's config: its name, `skald test CA {id}`, and extensions.
const CA_CONFIG: &str = "\
[req]
prompt = no
distinguished_name = name

[name]
CN = skald test CA {id}

[ca]
basicConstraints = critical, CA:true
keyUsage = critical, keyCertSign, cRLSign
subjectKeyIdentifier = hash
";

/// A server certificate's config: its name and extensions, its subjectAltName `{names}`.
const SERVER_CONFIG: &str = "\
[req]
prompt = no
distinguished_name = name

[name]
CN = skald test server

[server]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = {names}
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid
";

/// A test CA, its files in a directory of its own.
pub struct Ca {
    dir: PathBuf,
    /// Its certificate, PEM: what `SSL_CERT_FILE` names for a client to trust it.
    pub cert: PathBuf,
    /// How many server certificates it issued.
    issued: Cell<u32>,
}

impl Ca {
    /// Makes a CA in `dir`, with a name of its own: no other CA's certificate can pass for one it
    /// issued.
    pub fn new(dir: &Path) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = format!("{}-{}", process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        fs::create_dir_all(dir).expect("create the CA's directory");
        let ca = Ca {
            dir: dir.to_owned(),
            cert: dir.join("ca.pem"),
            issued: Cell::new(0),
        };
        fs::write(dir.join("ca.cnf"), CA_CONFIG.replace("{id}", &id))
            .expect("write the openssl config");
        let (config, key, cert) = (ca.path("ca.cnf"), ca.path("ca.key"), ca.path("ca.pem"));
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
            &key,
            "-out",
            &cert,
        ]);
        ca
    }

    /// The TLS config of a server presenting a certificate the CA issued for `names`, the value of
    /// its subjectAltName, such as `DNS:skald.test, IP:127.0.0.1`.
    pub fn issue(&self, names: &str) -> Arc<ServerConfig> {
        let serial = self.issued.get() + 1;
        self.issued.set(serial);
        let stem = format!("server-{serial}");
        let config = self.path(&format!("{stem}.cnf"));
        fs::write(&config, SERVER_CONFIG.replace("{names}", names))
            .expect("write the openssl config");
        let (key, request, cert) = (
            self.path(&format!("{stem}.key")),
            self.path(&format!("{stem}.csr")),
            self.path(&format!("{stem}.pem")),
        );
        openssl(&[
            "req", "-new", "-config", &config, "-newkey", "rsa:2048", "-nodes", "-keyout", &key,
            "-out", &request,
        ]);
        openssl(&[
            "x509",
            "-req",
            "-in",
            &request,
            "-CA",
            &self.path("ca.pem"),
            "-CAkey",
            &self.path("ca.key"),
            "-set_serial",
            &serial.to_string(),
            "-days",
            "2",
            "-extfile",
            &config,
            "-extensions",
            "server",
            "-out",
            &cert,
        ]);
        let chain = CertificateDer::pem_file_iter(&cert)
            .expect("read the server's certificate")
            .collect::<Result<Vec<_>, _>>()
            .expect("parse the server's certificate");
        let key = PrivateKeyDer::from_pem_file(&key).expect("read the server's key");
        let tls =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("TLS versions")
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .expect("a TLS config of the server's certificate");
        Arc::new(tls)
    }

    /// The path of the CA's file `name`, as openssl takes it.
    fn path(&self, name: &str) -> String {
        self.dir
            .join(name)
            .to_str()
            .expect("a UTF-8 path")
            .to_owned()
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
