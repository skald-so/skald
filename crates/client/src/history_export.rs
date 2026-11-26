//! `skald export` saves the verified history the device can read from a selected space or channel.

use std::collections::BTreeSet;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use skald_proto::{ChannelId, VersionRange};
use tempfile::{Builder, NamedTempFile};

use crate::connection::printable;
use crate::history_reading::{Read, Reader, no_key_line, rfc3339};
use crate::spaces::{self, Scope};
use crate::{folders, secrets, send};

#[derive(Clone, Copy, Deserialize, Serialize, clap::ValueEnum)]
pub enum Format {
    Json,
    Markdown,
}

pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    (speaks, timeout): (VersionRange, Duration),
    selected: (Option<&str>, Option<&str>),
    (format, output): (Format, &Path),
    agent: bool,
) -> Result<(), String> {
    let scope = if agent {
        let space = folders::working_space(dir)?;
        if selected.0.is_some_and(|name| name != space.name) {
            return Err(spaces::OUT_OF_SCOPE.to_owned());
        }
        Scope::Only(space.id)
    } else {
        Scope::All
    };
    let (synced, channel) = match selected {
        (Some(name), None) => {
            let synced = spaces::of_name(dir, storage, speaks, timeout, name, scope);
            (synced, None)
        }
        (None, Some(channel)) => {
            let channel = send::channel_id(channel)?;
            let synced = spaces::of_channel(dir, storage, speaks, timeout, channel, scope);
            (synced, Some(channel))
        }
        _ => return Err("select a space or a channel to export".to_owned()),
    };
    let mut reader = Reader::new(synced?)?;
    let mut channels = reader.member_channels();
    if let Some(channel) = channel {
        if !channels.contains(&channel) {
            return Err("this account is not a member of the selected channel".to_owned());
        }
        channels = vec![channel];
    }
    let mut read = Vec::new();
    let mut without_key = 0;
    for channel in channels {
        let (messages, missing) = reader.read(channel)?;
        without_key += missing;
        read.extend(messages.into_iter().map(|message| (channel, message)));
    }
    // The history endpoints do not report retention cutoffs. Report the entries the reader
    // could see but lacked keys for, alongside the reader's warnings for failed checks.
    if let Some(line) = no_key_line(without_key) {
        eprintln!("{line}");
    }
    read.sort_by_key(|(channel, entry)| (entry.timestamp, *channel, entry.sender, entry.msg_id));
    let mut seen = BTreeSet::new();
    read.retain(|(_, entry)| seen.insert((entry.sender, entry.msg_id)));
    let entries: Vec<_> = read
        .iter()
        .map(|(channel, message)| Entry::new(*channel, message))
        .collect();
    write_file(output, format, &entries).map_err(|err| {
        format!(
            "cannot save the export to {}: {err}",
            printable(&output.display().to_string())
        )
    })?;
    println!("Saved history export.");
    Ok(())
}

/// The same verified fields in either format.
#[derive(Serialize)]
struct Entry<'a> {
    channel: String,
    from: String,
    device: &'a str,
    device_id: String,
    timestamp: String,
    body: &'a str,
}

impl<'a> Entry<'a> {
    fn new(channel: ChannelId, message: &'a Read) -> Self {
        Self {
            channel: channel.to_string(),
            from: message.from.to_string(),
            device: &message.device,
            device_id: message.sender.to_string(),
            timestamp: rfc3339(message.timestamp),
            body: &message.text,
        }
    }

    fn markdown(&self, out: &mut impl Write) -> io::Result<()> {
        let device = printable(self.device);
        let inline = fence(&device, '`', 1);
        let block = fence(self.body, '~', 3);
        writeln!(
            out,
            "### {}\n\nChannel `{}` · From `{}` · Device {inline} {device} {inline} (`{}`)\n\n{block}\n{}\n{block}\n",
            self.timestamp, self.channel, self.from, self.device_id, self.body
        )
    }
}

/// A Markdown fence wide enough to keep every marker in `text` literal.
fn fence(text: &str, marker: char, minimum: usize) -> String {
    let longest = text.split(|c| c != marker).map(str::len).max().unwrap_or(0);
    marker.to_string().repeat(minimum.max(longest + 1))
}

/// Writes a private sibling file and replaces `path` only after it is complete.
fn write_file(path: &Path, format: Format, entries: &[Entry]) -> io::Result<()> {
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path contains NUL",
        ));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = private_file(parent)?;
    {
        let mut out = BufWriter::new(temporary.as_file_mut());
        match format {
            Format::Json => {
                serde_json::to_writer_pretty(&mut out, entries)?;
                writeln!(out)?;
            }
            Format::Markdown => {
                writeln!(out, "# Skald history\n")?;
                for entry in entries {
                    entry.markdown(&mut out)?;
                }
            }
        }
        out.flush()?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

fn temporary() -> Builder<'static, 'static> {
    let mut builder = Builder::new();
    builder.prefix(".skald-export-").rand_bytes(16);
    builder
}

#[cfg(all(unix, not(target_os = "macos")))]
fn private_file(parent: &Path) -> io::Result<NamedTempFile> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt as _;

    let permissions = Permissions::from_mode(0o600);
    let file = temporary()
        .permissions(permissions.clone())
        .tempfile_in(parent)?;
    file.as_file().set_permissions(permissions)?;
    Ok(file)
}

#[cfg(target_os = "macos")]
fn private_file(parent: &Path) -> io::Result<NamedTempFile> {
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd as _;

    private_macos_file(parent, |file| {
        let mut volume = MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: `file` is open and `volume` has room for fstatfs to initialize on success.
        if unsafe { libc::fstatfs(file.as_raw_fd(), volume.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fstatfs succeeded and initialized `volume`.
        Ok(unsafe { volume.assume_init() }.f_flags)
    })
}

#[cfg(target_os = "macos")]
fn private_macos_file(
    parent: &Path,
    mount_flags: impl FnOnce(&std::fs::File) -> io::Result<u32>,
) -> io::Result<NamedTempFile> {
    use std::ffi::CString;
    use std::fs::{File, Permissions};
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::io::FromRawFd as _;

    use libc::{O_CLOEXEC, O_CREAT, O_EXCL, O_WRONLY, c_char, c_int, c_void, mode_t};

    const FILESEC_MODE: c_int = 4;
    const FILESEC_ACL: c_int = 5;

    // macOS can inherit ACL entries that grant access beyond the POSIX mode.
    unsafe extern "C" {
        fn acl_from_text(text: *const c_char) -> *mut c_void;
        fn acl_free(acl: *mut c_void) -> c_int;
        fn filesec_init() -> *mut c_void;
        fn filesec_free(security: *mut c_void);
        fn filesec_set_property(
            security: *mut c_void,
            property: c_int,
            value: *const c_void,
        ) -> c_int;
        fn openx_np(path: *const c_char, flags: c_int, security: *mut c_void) -> c_int;
    }

    // SAFETY: this NUL-terminated string describes an empty, non-inheriting ACL.
    let acl = unsafe { acl_from_text(c"!#acl 1 no_inherit\n".as_ptr()) };
    if acl.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: filesec_init returns a new file security object or null.
    let security = unsafe { filesec_init() };
    let created = (|| {
        let mode: mode_t = 0o600;
        // SAFETY: the properties take pointers to a mode_t and an ACL pointer, respectively.
        if security.is_null()
            || unsafe { filesec_set_property(security, FILESEC_MODE, (&raw const mode).cast()) }
                == -1
            || unsafe { filesec_set_property(security, FILESEC_ACL, (&raw const acl).cast()) } == -1
        {
            return Err(io::Error::last_os_error());
        }
        let file = temporary().make_in(parent, |path| {
            let path = CString::new(path.as_os_str().as_bytes())?;
            // SAFETY: the path and file security object stay valid through openx_np.
            let fd = unsafe {
                openx_np(
                    path.as_ptr(),
                    O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC,
                    security,
                )
            };
            if fd == -1 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: openx_np returned an owned file descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        })?;
        let handle = file.as_file();
        if mount_flags(handle)? & libc::MNT_IGNORE_OWNERSHIP as u32 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "enable ownership on the output volume or choose another volume",
            ));
        }
        handle.set_permissions(Permissions::from_mode(mode.into()))?;
        Ok(file)
    })();
    // SAFETY: the objects were allocated by filesec_init and acl_from_text, respectively.
    unsafe {
        if !security.is_null() {
            filesec_free(security);
        }
        acl_free(acl);
    }
    created
}

#[cfg(windows)]
fn private_file(parent: &Path) -> io::Result<NamedTempFile> {
    use std::ffi::CStr;
    use std::fs::File;
    use std::os::windows::ffi::OsStrExt as _;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidA, ConvertStringSecurityDescriptorToSecurityDescriptorA,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, SECURITY_ATTRIBUTES, SECURITY_MAX_SID_SIZE, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetVolumeInformationByHandleW,
    };
    use windows_sys::Win32::System::SystemServices::FILE_PERSISTENT_ACLS;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    #[repr(C)]
    struct User {
        token_user: TOKEN_USER,
        sid: [u8; SECURITY_MAX_SID_SIZE as usize],
    }

    let mut token = null_mut();
    // SAFETY: `token` receives a new process token handle on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful call returned an owned handle.
    let _token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut user = User {
        token_user: TOKEN_USER::default(),
        sid: [0; SECURITY_MAX_SID_SIZE as usize],
    };
    let mut size = 0;
    // SAFETY: `user` is aligned for TOKEN_USER and has room for the largest SID.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            (&raw mut user).cast(),
            size_of::<User>() as u32,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut sid = null_mut();
    // SAFETY: the token's user SID points into `user`, which remains live.
    if unsafe { ConvertSidToStringSidA(user.token_user.User.Sid, &mut sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the conversion allocated a NUL-terminated ASCII SID with LocalAlloc.
    let descriptor = unsafe {
        let user = CStr::from_ptr(sid.cast()).to_string_lossy();
        let descriptor = format!("O:{user}D:P(A;;FA;;;{user})\0");
        LocalFree(sid.cast());
        descriptor
    };
    let mut security = null_mut();
    // SAFETY: `descriptor` is NUL-terminated and `security` is an out pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorA(
            descriptor.as_ptr(),
            SDDL_REVISION_1,
            &mut security,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security,
        bInheritHandle: 0,
    };
    let file = temporary().make_in(parent, |path| {
        let wide: Vec<_> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: `wide` is NUL-terminated and the security descriptor is still allocated.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateFileW returned a new handle owned by this process.
        Ok(unsafe { File::from_raw_handle(handle) })
    });
    // SAFETY: `security` is the descriptor allocated by the SDDL conversion.
    unsafe { LocalFree(security) };
    let file = file?;
    let mut flags = 0;
    // SAFETY: the file handle is open and `flags` is the only output requested.
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_file().as_raw_handle(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            &mut flags,
            null_mut(),
            0,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if flags & FILE_PERSISTENT_ACLS == 0 {
        return Err(io::Error::other(
            "the output volume does not support file access controls",
        ));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn completed_export_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("history.json");
        fs::write(&output, "older export").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&output, fs::Permissions::from_mode(0o644)).unwrap();
        }

        write_file(&output, Format::Json, &[]).unwrap();

        assert_eq!(fs::read_to_string(&output).unwrap(), "[]\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&output).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn export_ignores_inherited_file_access_rules() {
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("history.json");
        let setup = Command::new("chmod")
            .args(["+a", "group:everyone allow read,file_inherit"])
            .arg(dir.path())
            .status()
            .unwrap();
        assert!(setup.success());

        write_file(&output, Format::Json, &[]).unwrap();

        let listed = Command::new("ls").arg("-le").arg(output).output().unwrap();
        assert!(listed.status.success());
        let acl = String::from_utf8_lossy(&listed.stdout);
        assert_eq!(acl.lines().count(), 1, "{acl}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn export_refuses_a_volume_that_ignores_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let err = private_macos_file(dir.path(), |file| {
            assert_eq!(file.metadata().unwrap().len(), 0);
            Ok((libc::MNT_LOCAL | libc::MNT_IGNORE_OWNERSHIP) as u32)
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn windows_export_keeps_a_protected_current_user_dacl() {
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let permissive = Command::new("icacls.exe")
            .arg(dir.path())
            .args(["/grant", "*S-1-1-0:(OI)(IO)R"])
            .output()
            .unwrap();
        assert!(permissive.status.success(), "{permissive:?}");

        let control = dir.path().join("inherited.txt");
        fs::write(&control, "control").unwrap();
        let temporary = private_file(dir.path()).unwrap();
        let output = dir.path().join("history.json");
        fs::write(&output, "older export").unwrap();
        write_file(&output, Format::Json, &[]).unwrap();
        assert_eq!(fs::read_to_string(&output).unwrap(), "[]\n");

        let checked = Command::new("powershell.exe")
            .env("SKALD_TEST_ACL_CONTROL", control)
            .env("SKALD_TEST_ACL_TEMP", temporary.path())
            .env("SKALD_TEST_ACL_EXPORT", output)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                r"
$ErrorActionPreference = 'Stop'
$sidType = [System.Security.Principal.SecurityIdentifier]
$user = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
# Read ACLs through .NET without PowerShell module discovery.
$control = [System.IO.File]::GetAccessControl($env:SKALD_TEST_ACL_CONTROL)
$everyone = $control.GetAccessRules($false, $true, $sidType) | Where-Object {
    $_.IdentityReference.Value -eq 'S-1-1-0' -and $_.AccessControlType -eq 'Allow' -and
        ($_.FileSystemRights -band [System.Security.AccessControl.FileSystemRights]::ReadData)
}
if (-not $everyone) { throw 'The control file did not inherit the permissive rule' }
foreach ($path in @($env:SKALD_TEST_ACL_TEMP, $env:SKALD_TEST_ACL_EXPORT)) {
    $acl = [System.IO.File]::GetAccessControl($path)
    $rules = $acl.GetAccessRules($true, $true, $sidType)
    if (-not $acl.AreAccessRulesProtected -or $acl.GetOwner($sidType).Value -ne $user -or
        $rules.Count -ne 1 -or $rules[0].IsInherited -or
        $rules[0].IdentityReference.Value -ne $user -or $rules[0].AccessControlType -ne 'Allow' -or
        $rules[0].FileSystemRights -ne 'FullControl') {
        throw 'The export DACL does not grant access exclusively to the current user'
    }
}
",
            ])
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "ACL check failed: {}",
            String::from_utf8_lossy(&checked.stderr)
        );
    }
}
