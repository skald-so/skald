//! `skald update`: replaces the running `skald` with the latest stable release on GitHub Releases,
//! from the release's archive for this machine, checked against the release's `<archive>.sha256`.
//! It trusts that checksum because releases are immutable: `docs/dev/infrastructure.md`, "Trust
//! in builds".

use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::tls::{Certificate, RootCerts, TlsConfig};

use crate::connection::printable;
use crate::plain_path::plain;

/// The base URL of GitHub's REST API.
pub const GITHUB_API_URL: &str = "https://api.github.com";

/// The repository of the releases.
const REPO: &str = "skald-so/skald";

/// The file beside a `skald` installed from npm: the `skald-so` package's launcher writes it beside
/// the `skald` it unpacks.
const NPM_MARKER: &str = "installed-by-npm";

/// The most bytes read of an archive, 64 MiB; of a checksum, 4 KiB; of the API's answer, 4 MiB.
const ARCHIVE_LIMIT: u64 = 64 << 20;
const CHECKSUM_LIMIT: u64 = 4 << 10;
const API_LIMIT: u64 = 4 << 20;

/// The target of this machine's archive in a release, as dist names it.
const TARGET: Option<&str> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("aarch64-apple-darwin")
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some("x86_64-apple-darwin")
} else if cfg!(all(
    target_os = "linux",
    target_env = "gnu",
    target_arch = "aarch64"
)) {
    Some("aarch64-unknown-linux-gnu")
} else if cfg!(all(
    target_os = "linux",
    target_env = "gnu",
    target_arch = "x86_64"
)) {
    Some("x86_64-unknown-linux-gnu")
} else if cfg!(all(windows, target_env = "msvc", target_arch = "aarch64")) {
    Some("aarch64-pc-windows-msvc")
} else if cfg!(all(windows, target_env = "msvc", target_arch = "x86_64")) {
    Some("x86_64-pc-windows-msvc")
} else {
    None
};

/// The fields of a GitHub release the client reads.
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// The fields of a release file the client reads.
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

impl Release {
    /// The URL of the release's file `name`.
    fn file(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.as_str())
    }
}

/// The archive of `target` in a release, as dist names and lays it out: a zip with `skald.exe` on
/// Windows, a gzipped tar with `skald` elsewhere.
struct Archive {
    name: String,
    zip: bool,
}

impl Archive {
    fn of(target: &str) -> Self {
        let zip = target.ends_with("-windows-msvc");
        let extension = if zip { "zip" } else { "tar.gz" };
        Archive {
            name: format!("skald-client-{target}.{extension}"),
            zip,
        }
    }

    /// The name of `skald` in the archive.
    fn binary(&self) -> &'static str {
        if self.zip { "skald.exe" } else { "skald" }
    }

    /// The bytes of `skald` in `archive`, the bytes of this archive.
    fn unpack(&self, archive: &[u8]) -> Result<Vec<u8>, String> {
        let binary = self.binary();
        let unreadable =
            |err: &dyn std::fmt::Display| format!("cannot unpack {}: {err}", self.name);
        let found = if self.zip {
            unpack_zip(archive, binary).map_err(|err| unreadable(&err))?
        } else {
            unpack_tar_gz(archive, binary).map_err(|err| unreadable(&err))?
        };
        found.ok_or_else(|| format!("{} has no {binary}", self.name))
    }
}

/// The first regular file named `binary` in the zip `archive`.
fn unpack_zip(archive: &[u8], binary: &str) -> zip::result::ZipResult<Option<Vec<u8>>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))?;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index)?;
        let named = file
            .enclosed_name()
            .is_some_and(|path| path.file_name().is_some_and(|name| name == binary));
        if file.is_file() && named {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

/// The first regular file named `binary` in the gzipped tar `archive`.
fn unpack_tar_gz(archive: &[u8], binary: &str) -> io::Result<Option<Vec<u8>>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let named = entry.path()?.file_name().is_some_and(|name| name == binary);
        if entry.header().entry_type().is_file() && named {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            return Ok(Some(bytes));
        }
    }
    Ok(None)
}

/// `skald update`: looks up the latest release at `api`, GitHub's REST API, with requests that
/// each fail after `timeout` without progress, and installs it when it is newer than this skald.
pub fn run(api: &str, timeout: Duration) -> Result<(), String> {
    let running = env!("CARGO_PKG_VERSION");
    // As people write the path: it shows in errors.
    let exe = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map(|exe| plain(&exe))
        .map_err(|err| format!("cannot find the running skald: {err}"))?;
    if exe.with_file_name(NPM_MARKER).exists() {
        return Err(format!(
            "{} is installed from npm: update it through npm",
            exe.display()
        ));
    }
    let target = TARGET.ok_or("no release of skald is built for this OS and architecture")?;
    if !api.starts_with("https://") {
        return Err(format!("{api:?} is not an HTTPS URL of GitHub's API"));
    }
    remove_set_aside(&exe);

    let agent = agent(timeout);
    let release = latest(&agent, api)?;
    let current = Version::parse(running).expect("the version of skald is a semantic version");
    let Some((release, version)) = release.and_then(|release| {
        let version = newer(&release, &current)?;
        Some((release, version))
    }) else {
        println!("skald {running} is current: there is no newer stable release");
        return Ok(());
    };

    let tag = printable(&release.tag_name);
    let archive = Archive::of(target);
    let checksum_name = format!("{}.sha256", archive.name);
    let archive_url = release.file(&archive.name).ok_or_else(|| {
        format!(
            "release {tag} has no archive for this machine, {}",
            archive.name
        )
    })?;
    let checksum_url = release.file(&checksum_name).ok_or_else(|| {
        format!(
            "release {tag} has no checksum of {}, {checksum_name}: skald is not updated",
            archive.name
        )
    })?;
    let checksum = download(
        &agent,
        checksum_url,
        CHECKSUM_LIMIT,
        &format!("the checksum of {}, {checksum_name}", archive.name),
    )?;
    let expected = parse_checksum(&checksum).ok_or_else(|| {
        format!(
            "the checksum of {}, {checksum_name}, has no SHA-256: skald is not updated",
            archive.name
        )
    })?;
    let bytes = download(&agent, archive_url, ARCHIVE_LIMIT, &archive.name)?;
    if <[u8; 32]>::from(Sha256::digest(&bytes)) != expected {
        return Err(format!(
            "{} does not match its checksum, {checksum_name}: skald is not updated",
            archive.name
        ));
    }
    let binary = archive.unpack(&bytes)?;
    replace(&exe, &binary).map_err(|err| format!("cannot replace {}: {err}", exe.display()))?;
    println!("skald updated from {running} to {version}");
    Ok(())
}

/// The version of `release` if it is a stable release newer than `current`.
fn newer(release: &Release, current: &Version) -> Option<Version> {
    if release.draft || release.prerelease {
        return None;
    }
    let version = Version::parse(release.tag_name.strip_prefix('v')?).ok()?;
    (version.pre.is_empty() && version > *current).then_some(version)
}

/// The SHA-256 of a checksum file as dist writes it, `<hex> *<name>`.
fn parse_checksum(file: &[u8]) -> Option<[u8; 32]> {
    let hex = std::str::from_utf8(file).ok()?.split_whitespace().next()?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut sum = [0; 32];
    for (byte, pair) in sum.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(sum)
}

/// The HTTPS client of the releases: the platform's root certificates, or those `SSL_CERT_FILE`
/// and `SSL_CERT_DIR` name; the proxy `HTTPS_PROXY` names, as `ureq` reads it; redirects followed,
/// to HTTPS only.
fn agent(timeout: Duration) -> ureq::Agent {
    let roots: Vec<Certificate<'static>> = rustls_native_certs::load_native_certs()
        .certs
        .into_iter()
        .map(|der| Certificate::from_der(der.as_ref()).to_owned())
        .collect();
    let provider = rustls::crypto::ring::default_provider();
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::Specific(Arc::new(roots)))
        .unversioned_rustls_crypto_provider(Arc::new(provider))
        .build();
    ureq::Agent::config_builder()
        .https_only(true)
        .http_status_as_error(false)
        .timeout_connect(Some(timeout))
        .timeout_per_read(Some(timeout))
        .timeout_per_write(Some(timeout))
        .user_agent(concat!("skald/", env!("CARGO_PKG_VERSION")))
        .tls_config(tls)
        .build()
        .new_agent()
}

/// The latest stable release, from `api`; `None` if there is none.
fn latest(agent: &ureq::Agent, api: &str) -> Result<Option<Release>, String> {
    let url = format!("{}/repos/{REPO}/releases/latest", api.trim_end_matches('/'));
    let what = "the latest release";
    match get(agent, &url, API_LIMIT, what)? {
        (200, body) => serde_json::from_slice(&body)
            .map(Some)
            .map_err(|err| format!("unexpected answer about {what}: {err}")),
        (404, _) => Ok(None),
        (status, _) => Err(format!("cannot get {what}: the answer is {status}")),
    }
}

/// The release file at `url`, `what`, of at most `limit` bytes.
fn download(agent: &ureq::Agent, url: &str, limit: u64, what: &str) -> Result<Vec<u8>, String> {
    match get(agent, url, limit, what)? {
        (200, body) => Ok(body),
        (status, _) => Err(format!("cannot get {what}: the answer is {status}")),
    }
}

/// The status and the body, of at most `limit` bytes, of the answer to `GET <url>`, `what`.
fn get(agent: &ureq::Agent, url: &str, limit: u64, what: &str) -> Result<(u16, Vec<u8>), String> {
    let failed = |err: ureq::Error| format!("cannot get {what}: {}", printable(&err.to_string()));
    let mut response = agent
        .get(url)
        .header(
            "accept",
            "application/vnd.github+json, application/octet-stream",
        )
        .header("x-github-api-version", "2022-11-28")
        .call()
        .map_err(failed)?;
    let body = response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(failed)?;
    Ok((response.status().as_u16(), body))
}

/// Puts `binary` in place of the running skald at `exe`, with its permissions. The new file is
/// written beside it first, so a failure leaves the old one.
fn replace(exe: &Path, binary: &[u8]) -> io::Result<()> {
    let staged = beside(exe, "new");
    let _ = fs::remove_file(&staged);
    let written = (|| {
        let mut file = fs::File::create_new(&staged)?;
        file.write_all(binary)?;
        file.sync_all()?;
        fs::set_permissions(&staged, fs::metadata(exe)?.permissions())
    })();
    // Windows does not replace a running program's file, but renames it: it is set aside, and a
    // later update removes it.
    let aside = cfg!(windows).then(|| beside(exe, "old"));
    let result = written.and_then(|()| swap(exe, &staged, aside.as_deref()));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

/// Moves `staged` to `exe`, after it moves `exe` to `aside` if given; the old `exe` is back on a
/// failure. An `aside` it cannot remove, such as a running program on Windows, stays.
fn swap(exe: &Path, staged: &Path, aside: Option<&Path>) -> io::Result<()> {
    let Some(aside) = aside else {
        return fs::rename(staged, exe);
    };
    fs::rename(exe, aside)?;
    if let Err(err) = fs::rename(staged, exe) {
        let _ = fs::rename(aside, exe);
        return Err(err);
    }
    let _ = fs::remove_file(aside);
    Ok(())
}

/// A file beside `exe` for an update of it, by this process: `.<exe's name>.<pid>.<kind>`.
fn beside(exe: &Path, kind: &str) -> PathBuf {
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    exe.with_file_name(format!(".{name}.{}.{kind}", std::process::id()))
}

/// Removes the files earlier updates set aside beside `exe`, those that no longer run.
fn remove_set_aside(exe: &Path) {
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) else {
        return;
    };
    let prefix = format!(".{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.starts_with(&prefix) && file.ends_with(".old") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    fn release(tag: &str, prerelease: bool) -> Release {
        Release {
            tag_name: tag.to_owned(),
            draft: false,
            prerelease,
            assets: Vec::new(),
        }
    }

    #[test]
    fn installs_only_a_newer_stable_release() {
        let current = Version::parse("0.4.2").unwrap();
        let newer_of = |tag, prerelease| newer(&release(tag, prerelease), &current);
        assert_eq!(
            newer_of("v0.4.3", false),
            Some(Version::parse("0.4.3").unwrap())
        );
        assert_eq!(
            newer_of("v1.0.0", false),
            Some(Version::parse("1.0.0").unwrap())
        );
        assert_eq!(newer_of("v0.4.2", false), None);
        assert_eq!(newer_of("v0.4.1", false), None);
        assert_eq!(newer_of("v0.5.0-beta.1", false), None);
        assert_eq!(newer_of("v0.5.0", true), None);
        assert_eq!(newer_of("0.5.0", false), None);
        assert_eq!(newer_of("vnext", false), None);
        let draft = Release {
            draft: true,
            ..release("v0.5.0", false)
        };
        assert_eq!(newer(&draft, &current), None);
    }

    #[test]
    fn a_beta_updates_to_its_release() {
        let beta = Version::parse("0.5.0-beta.2").unwrap();
        assert!(newer(&release("v0.5.0", false), &beta).is_some());
        assert!(newer(&release("v0.4.9", false), &beta).is_none());
    }

    #[test]
    fn reads_a_checksum_as_dist_writes_it() {
        let hex = "00ff10".repeat(10) + "abcd";
        let mut sum = [0u8; 32];
        for (i, byte) in sum.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
        }
        let file = format!("{hex} *skald-client-x86_64-apple-darwin.tar.gz\n\n");
        assert_eq!(parse_checksum(file.as_bytes()), Some(sum));
        assert_eq!(parse_checksum(hex.to_uppercase().as_bytes()), Some(sum));
        assert_eq!(parse_checksum(b""), None);
        assert_eq!(parse_checksum(&hex.as_bytes()[..62]), None);
        assert_eq!(parse_checksum(format!("{hex}0").as_bytes()), None);
        assert_eq!(parse_checksum(hex.replace('a', "g").as_bytes()), None);
        assert_eq!(parse_checksum(hex.replacen("00", "+0", 1).as_bytes()), None);
        assert_eq!(parse_checksum(hex.replacen("00", "é", 1).as_bytes()), None);
    }

    #[test]
    fn names_the_archive_of_each_target() {
        let mac = Archive::of("aarch64-apple-darwin");
        assert_eq!(mac.name, "skald-client-aarch64-apple-darwin.tar.gz");
        assert_eq!(mac.binary(), "skald");
        let windows = Archive::of("x86_64-pc-windows-msvc");
        assert_eq!(windows.name, "skald-client-x86_64-pc-windows-msvc.zip");
        assert_eq!(windows.binary(), "skald.exe");
    }

    /// A gzipped tar of `files`, each a path and its bytes.
    fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        for (path, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            tar.append_data(&mut header, path, *bytes).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    /// A zip of `files`, deflated.
    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (path, bytes) in files {
            zip.start_file(*path, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn unpacks_skald_from_a_tar_gz_as_dist_lays_it_out() {
        let archive = Archive::of("x86_64-unknown-linux-gnu");
        let bytes = tar_gz(&[
            (
                "skald-client-x86_64-unknown-linux-gnu/README.md",
                b"# skald\n",
            ),
            ("skald-client-x86_64-unknown-linux-gnu/skald", b"the binary"),
        ]);
        assert_eq!(archive.unpack(&bytes).unwrap(), b"the binary");

        let without = tar_gz(&[("skald-client-x86_64-unknown-linux-gnu/README.md", b"#\n")]);
        let err = archive.unpack(&without).unwrap_err();
        assert!(err.contains("has no skald"), "{err}");
        let err = archive.unpack(b"not an archive").unwrap_err();
        assert!(err.contains("cannot unpack"), "{err}");
    }

    #[test]
    fn unpacks_skald_from_a_zip_as_dist_lays_it_out() {
        let archive = Archive::of("aarch64-pc-windows-msvc");
        let bytes = zip(&[("README.md", b"# skald\n"), ("skald.exe", b"the binary")]);
        assert_eq!(archive.unpack(&bytes).unwrap(), b"the binary");

        let without = zip(&[("README.md", b"# skald\n"), ("skald", b"another")]);
        let err = archive.unpack(&without).unwrap_err();
        assert!(err.contains("has no skald.exe"), "{err}");
        let err = archive.unpack(b"not an archive").unwrap_err();
        assert!(err.contains("cannot unpack"), "{err}");
    }

    /// The names of the files in `dir`, sorted.
    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replaces_the_binary_with_its_permissions() {
        let dir = TestDir::new("update-replace");
        let exe = dir.join("skald");
        fs::write(&exe, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o751)).unwrap();
        }
        let permissions = fs::metadata(&exe).unwrap().permissions();

        replace(&exe, b"new").unwrap();

        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert_eq!(fs::metadata(&exe).unwrap().permissions(), permissions);
        assert_eq!(files(&dir), ["skald"]);
    }

    #[test]
    fn a_failed_replace_leaves_the_old_binary() {
        let dir = TestDir::new("update-replace-fails");
        let exe = dir.join("skald");
        fs::write(&exe, "old").unwrap();
        // No staged file to move: the old binary is back in place, and nothing else is left.
        let staged = dir.join("staged");

        for aside in [None, Some(dir.join("aside"))] {
            assert!(swap(&exe, &staged, aside.as_deref()).is_err());

            assert_eq!(fs::read(&exe).unwrap(), b"old");
            assert_eq!(files(&dir), ["skald"]);
        }
    }

    #[test]
    fn swaps_through_a_file_set_aside() {
        let dir = TestDir::new("update-swap-aside");
        let exe = dir.join("skald");
        fs::write(&exe, "old").unwrap();
        let staged = dir.join("staged");
        fs::write(&staged, "new").unwrap();

        swap(&exe, &staged, Some(&dir.join("aside"))).unwrap();

        assert_eq!(fs::read(&exe).unwrap(), b"new");
        // Nothing runs the old one here, so it is removed.
        assert_eq!(files(&dir), ["skald"]);
    }

    #[test]
    fn removes_only_files_set_aside_by_updates() {
        let dir = TestDir::new("update-set-aside");
        let exe = dir.join("skald.exe");
        for name in [
            "skald.exe",
            ".skald.exe.12.old",
            ".skald.exe.34.old",
            ".skald.exe.12.new",
            ".other.12.old",
            "installed-by-npm",
        ] {
            fs::write(dir.join(name), "").unwrap();
        }

        remove_set_aside(&exe);

        assert_eq!(
            files(&dir),
            [
                ".other.12.old",
                ".skald.exe.12.new",
                "installed-by-npm",
                "skald.exe"
            ]
        );
    }
}
