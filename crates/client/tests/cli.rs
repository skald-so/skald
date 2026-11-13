use std::path::Path;
use std::process::Command;

fn workspace_version() -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let manifest: toml::Table = std::fs::read_to_string(manifest)
        .expect("read workspace manifest")
        .parse()
        .expect("parse workspace manifest");
    manifest["workspace"]["package"]["version"]
        .as_str()
        .expect("workspace version is a string")
        .to_owned()
}

#[test]
fn version_prints_workspace_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_skald"))
        .arg("--version")
        .output()
        .expect("run skald");

    assert!(output.status.success(), "exit status: {}", output.status);
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        format!("skald {}\n", workspace_version())
    );
    assert!(output.stderr.is_empty(), "unexpected stderr");
}

/// What `skald --help` prints.
fn help() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_skald"))
        .arg("--help")
        .output()
        .expect("run skald");

    assert!(output.status.success(), "exit status: {}", output.status);
    String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

/// `skald --help` names this platform's default state directory, and no other platform's.
#[test]
fn help_names_the_platforms_state_directory() {
    let help = help();
    let defaults = [
        (cfg!(windows), r"[default: %LOCALAPPDATA%\skald]"),
        (
            cfg!(target_os = "macos"),
            "[default: ~/Library/Application Support/skald]",
        ),
        (
            !cfg!(any(windows, target_os = "macos")),
            "[default: $XDG_STATE_HOME/skald or ~/.local/state/skald]",
        ),
    ];
    for (this_platform, default) in defaults {
        assert_eq!(help.contains(default), this_platform, "{default}: {help}");
    }
}

/// `skald --help` says how the secrets file is kept on this platform, and not as on another: with
/// 0600 permissions on Unix, with the access rights of the state directory on Windows.
#[test]
fn help_says_how_the_platform_keeps_the_secrets_file() {
    let help = help();
    let kept = [
        (cfg!(unix), "The file has 0600 permissions"),
        (
            cfg!(windows),
            "The file has the access rights it inherits from the state directory",
        ),
    ];
    for (this_platform, how) in kept {
        assert_eq!(help.contains(how), this_platform, "{how}: {help}");
    }
    assert_eq!(help.contains("0600"), cfg!(unix), "{help}");
}
