//! The server's attachment storage, a directory or a bucket on an S3-compatible server, and its
//! check at start.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use rusty_s3::{Credentials, S3Action, UrlStyle};
use ureq::http::Response;
use ureq::{Body, Error};
use url::Url;

use crate::github::ClientSecret;
use crate::http_client::{agent, printable};
use crate::oidc::{bare, provider_url};

/// How long a signed request to the bucket stays valid.
const SIGNED_FOR: Duration = Duration::from_secs(60);

/// Where the server keeps attachments.
#[derive(Debug, Clone)]
pub enum AttachmentStorage {
    /// A directory on the server's disk.
    Directory(PathBuf),
    /// A bucket on an S3-compatible server.
    Bucket(BucketSettings),
}

/// A bucket on an S3-compatible server, addressed in the path of its endpoint.
#[derive(Debug, Clone)]
pub struct BucketSettings {
    pub name: String,
    pub endpoint: BucketEndpoint,
    pub region: String,
    pub access_key_id: String,
    pub secret_key: ClientSecret,
}

/// A bucket's S3 endpoint: https, or http on a loopback address, with no user, query or fragment.
#[derive(Debug, Clone)]
pub struct BucketEndpoint(Url);

impl FromStr for BucketEndpoint {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let url = provider_url(text)?;
        bare(&url)
            .then_some(BucketEndpoint(url))
            .ok_or("an endpoint has no user, query or fragment".to_owned())
    }
}

impl fmt::Display for AttachmentStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachmentStorage::Directory(dir) => write!(f, "the directory {}", dir.display()),
            AttachmentStorage::Bucket(bucket) => {
                write!(f, "the bucket {} at {}", bucket.name, bucket.endpoint.0)
            }
        }
    }
}

impl AttachmentStorage {
    /// Writes an object of its own to the storage, reads it back and deletes it; blocks. The error
    /// names the storage and the step that failed.
    pub fn check(&self) -> Result<(), String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("the operating system's random generator failed");
        let random: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let object = format!(".skald-storage-check-{random}");
        let result = match self {
            AttachmentStorage::Directory(dir) => check_directory(dir, &object, random.as_bytes()),
            AttachmentStorage::Bucket(bucket) => bucket.check(&object, random.as_bytes()),
        };
        result.map_err(|err| format!("attachment storage in {self}: {err}"))
    }
}

/// Writes `content` to the new file `object` in `dir`, reads it back and deletes it.
fn check_directory(dir: &Path, object: &str, content: &[u8]) -> Result<(), String> {
    let path = dir.join(object);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|err| format!("cannot write {object}: {err}"))?;
    let written = file.write_all(content).and_then(|()| file.sync_all());
    drop(file);
    let read = written
        .map_err(|err| format!("cannot write {object}: {err}"))
        .and_then(|()| fs::read(&path).map_err(|err| format!("cannot read {object} back: {err}")))
        .and_then(|read| same(&read, content, object));
    let deleted = fs::remove_file(&path).map_err(|err| format!("cannot delete {object}: {err}"));
    read.and(deleted)
}

impl BucketSettings {
    /// Puts `content` as `object` into the bucket, gets it back and deletes it.
    fn check(&self, object: &str, content: &[u8]) -> Result<(), String> {
        let bucket = rusty_s3::Bucket::new(
            self.endpoint.0.clone(),
            UrlStyle::Path,
            self.name.clone(),
            self.region.clone(),
        )
        .map_err(|err| format!("cannot address the bucket: {err}"))?;
        let credentials = Credentials::new(&self.access_key_id, self.secret_key.expose());
        let credentials = Some(&credentials);

        let put = bucket.put_object(credentials, object).sign(SIGNED_FOR);
        succeeded(agent().put(put.as_str()).send(content))
            .map_err(|err| format!("cannot write {object}: {err}"))?;
        let get = bucket.get_object(credentials, object).sign(SIGNED_FOR);
        let read = succeeded(agent().get(get.as_str()).call())
            .and_then(|mut response| {
                response
                    .body_mut()
                    .read_to_vec()
                    .map_err(|err| format!("the answer is unreadable: {err}"))
            })
            .map_err(|err| format!("cannot read {object} back: {err}"))
            .and_then(|read| same(&read, content, object));
        let delete = bucket.delete_object(credentials, object).sign(SIGNED_FOR);
        let deleted = succeeded(agent().delete(delete.as_str()).call())
            .map(drop)
            .map_err(|err| format!("cannot delete {object}: {err}"));
        read.and(deleted)
    }
}

/// Whether `read`, what came back of `object`, is the `written` content.
fn same(read: &[u8], written: &[u8], object: &str) -> Result<(), String> {
    (read == written)
        .then_some(())
        .ok_or(format!("read {object} back with other content"))
}

/// A `2xx` answer of the bucket's server, or why there is none, such as `answered 403 Forbidden
/// SignatureDoesNotMatch`.
fn succeeded(response: Result<Response<Body>, Error>) -> Result<Response<Body>, String> {
    let mut response = response.map_err(|err| format!("the request failed: {err}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response
        .body_mut()
        .with_config()
        .limit(65_536)
        .read_to_string()
        .unwrap_or_default();
    let code = body
        .split_once("<Code>")
        .and_then(|(_, rest)| rest.split_once("</Code>"))
        .map(|(code, _)| format!(" {}", printable(code)))
        .unwrap_or_default();
    Err(format!("answered {status}{code}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_is_https_or_http_on_loopback_without_extras() {
        for url in [
            "https://s3.example",
            "https://s3.example/prefix",
            "http://127.0.0.1:9000",
            "http://[::1]:9000",
        ] {
            assert!(url.parse::<BucketEndpoint>().is_ok(), "{url}");
        }
        for url in [
            "http://s3.example",
            "http://localhost:9000",
            "http://192.0.2.10:9000",
            "ftp://127.0.0.1",
            "https://key:secret@s3.example",
            "https://s3.example/?a=b",
            "https://s3.example/#a",
        ] {
            assert!(url.parse::<BucketEndpoint>().is_err(), "{url}");
        }
    }

    #[test]
    fn a_refusal_names_the_status_and_the_code() {
        let response = Response::builder()
            .status(403)
            .body(Body::builder().data("<Error><Code>SignatureDoesNotMatch</Code></Error>"))
            .expect("a response");
        assert_eq!(
            succeeded(Ok(response)).err().as_deref(),
            Some("answered 403 Forbidden SignatureDoesNotMatch")
        );
    }

    #[test]
    fn other_content_read_back_fails_the_check() {
        assert!(same(b"written", b"written", "o").is_ok());
        assert_eq!(
            same(b"other", b"written", "o"),
            Err("read o back with other content".to_owned())
        );
    }

    #[test]
    fn a_directory_is_left_as_it_was() {
        let dir = std::env::temp_dir().join(format!("skald-check-unit-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create a directory");
        let storage = AttachmentStorage::Directory(dir.clone());
        storage.check().expect("the check passes");
        assert_eq!(fs::read_dir(&dir).expect("read the directory").count(), 0);
        fs::remove_dir(&dir).expect("remove the directory");
    }
}
