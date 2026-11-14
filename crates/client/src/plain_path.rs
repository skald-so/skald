//! Paths as people write them. On Windows `fs::canonicalize` gives a verbatim path, with the
//! prefix `\\?\`: `\\?\C:\Users\ada\skald.exe`, `\\?\UNC\server\share\skald.exe`.

use std::path::{Path, PathBuf};

/// The longest path Windows takes without the verbatim prefix, in UTF-16 units: `MAX_PATH`, 260,
/// less the terminating null.
const MAX_PATH: usize = 259;

/// `path` without the verbatim prefix on Windows, where it means the same file without it:
/// `C:\Users\ada\skald.exe`, `\\server\share\skald.exe`. Any other path stays as it is.
pub fn plain(path: &Path) -> PathBuf {
    match path.to_str().and_then(without_verbatim) {
        Some(plain) if cfg!(windows) => PathBuf::from(plain),
        _ => path.to_owned(),
    }
}

/// The Windows path `path` without its verbatim prefix, if it has one and names the same file
/// without it: a drive's path or a UNC path, no longer than `MAX_PATH`, each of whose names
/// Windows reads as it is.
fn without_verbatim(path: &str) -> Option<String> {
    let (plain, names) = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        // The server and the share are names too.
        if unc.split('\\').filter(|name| !name.is_empty()).count() < 2 {
            return None;
        }
        (format!(r"\\{unc}"), unc)
    } else {
        let disk = path.strip_prefix(r"\\?\")?;
        let drive = disk.as_bytes().get(..3)?;
        if !(drive[0].is_ascii_alphabetic() && drive[1..] == *br":\") {
            return None;
        }
        (disk.to_owned(), &disk[3..])
    };
    let read_as_is = names.is_empty() || names.split('\\').all(read_as_is);
    (read_as_is && plain.encode_utf16().count() <= MAX_PATH).then_some(plain)
}

/// Whether Windows reads the file name `name` as it is in a path without the verbatim prefix: it
/// is not empty, has no character Windows reserves, does not end in a dot or a space, which
/// Windows drops, and is not a device's name, such as `CON` or `nul.txt`.
fn read_as_is(name: &str) -> bool {
    const DEVICES: [&str; 30] = [
        "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
        "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4",
        "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
    ];
    let reserved = |c: char| c.is_ascii_control() || r#"<>:"/\|?*"#.contains(c);
    let stem = name.split('.').next().unwrap_or_default();
    let stem = stem.trim_end_matches([' ', '.']);
    !name.is_empty()
        && !name.contains(reserved)
        && !name.ends_with(['.', ' '])
        && !DEVICES
            .iter()
            .any(|device| stem.eq_ignore_ascii_case(device))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drives_path_loses_the_prefix() {
        let exe = r"C:\Users\ada\AppData\Roaming\npm\node_modules\skald-so\bin\skald.exe";
        assert_eq!(
            without_verbatim(&format!(r"\\?\{exe}")).as_deref(),
            Some(exe)
        );
        assert_eq!(
            without_verbatim(r"\\?\d:\skald.exe").as_deref(),
            Some(r"d:\skald.exe")
        );
        assert_eq!(without_verbatim(r"\\?\C:\").as_deref(), Some(r"C:\"));
        assert_eq!(
            without_verbatim(r"\\?\C:\Program Files\skald\skald.exe").as_deref(),
            Some(r"C:\Program Files\skald\skald.exe")
        );
    }

    #[test]
    fn a_unc_path_becomes_a_plain_unc_path() {
        assert_eq!(
            without_verbatim(r"\\?\UNC\server\share\npm\skald.exe").as_deref(),
            Some(r"\\server\share\npm\skald.exe")
        );
        assert_eq!(
            without_verbatim(r"\\?\UNC\server\share").as_deref(),
            Some(r"\\server\share")
        );
        // Without a share it is no UNC path.
        assert_eq!(without_verbatim(r"\\?\UNC\server"), None);
        assert_eq!(without_verbatim(r"\\?\UNC\server\"), None);
    }

    #[test]
    fn a_path_that_means_another_file_without_the_prefix_keeps_it() {
        for path in [
            // Names Windows reads differently or not at all.
            r"\\?\C:\skald\CON\skald.exe",
            r"\\?\C:\skald\nul.txt",
            r"\\?\C:\skald\com1 .log",
            r"\\?\C:\skald\LPT9",
            r"\\?\C:\skald\CONOUT$",
            r"\\?\C:\skald.\skald.exe",
            r"\\?\C:\skald \skald.exe",
            r"\\?\C:\skald\..\skald.exe",
            r"\\?\C:\skald\.\skald.exe",
            r"\\?\C:\skald\\skald.exe",
            r"\\?\C:\skald\a:b",
            r"\\?\C:\skald\a/b",
            r"\\?\C:\skald\a*b",
            "\\\\?\\C:\\skald\\a\u{1}b",
            r"\\?\UNC\server\share\aux",
            r"\\?\UNC\ser:ver\share\skald.exe",
            // Not a drive's path or a UNC path.
            r"\\?\Volume{2a7f6b3e-0000-0000-0000-100000000000}\skald.exe",
            r"\\?\GLOBALROOT\Device\HarddiskVolume2\skald.exe",
            r"\\?\pipe\skald",
            r"\\?\C:",
            r"\\?\C:skald.exe",
            r"\\?\1:\skald.exe",
            // No prefix.
            r"C:\skald\skald.exe",
            r"\\server\share\skald.exe",
            r"\\.\C:\skald.exe",
            "/usr/local/bin/skald",
        ] {
            assert_eq!(without_verbatim(path), None, "{path}");
        }
        // Device names inside a name are no device.
        assert_eq!(
            without_verbatim(r"\\?\C:\console\nul-free\skald.exe").as_deref(),
            Some(r"C:\console\nul-free\skald.exe")
        );
    }

    #[test]
    fn a_path_too_long_without_the_prefix_keeps_it() {
        let longest = format!(r"C:\{}", "a".repeat(MAX_PATH - 3));
        assert_eq!(
            without_verbatim(&format!(r"\\?\{longest}")).as_deref(),
            Some(longest.as_str())
        );
        let longer = format!(r"\\?\{longest}b");
        assert_eq!(without_verbatim(&longer), None);
        // Counted in UTF-16 units, as Windows counts: each of these takes two.
        let wide = format!(r"\\?\C:\{}", "😀".repeat(MAX_PATH / 2));
        assert_eq!(without_verbatim(&wide), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_canonical_path_is_plain_and_names_the_same_file() {
        let exe = std::env::current_exe().unwrap();
        let canonical = std::fs::canonicalize(&exe).unwrap();
        assert!(
            canonical.to_str().unwrap().starts_with(r"\\?\"),
            "{canonical:?}"
        );
        let plain = plain(&canonical);
        assert!(!plain.to_str().unwrap().starts_with(r"\\?\"), "{plain:?}");
        assert_eq!(std::fs::canonicalize(&plain).unwrap(), canonical);
    }

    #[cfg(not(windows))]
    #[test]
    fn a_path_elsewhere_stays_as_it_is() {
        for path in [r"\\?\C:\skald.exe", "/usr/local/bin/skald", "skald"] {
            assert_eq!(plain(Path::new(path)), Path::new(path));
        }
    }
}
