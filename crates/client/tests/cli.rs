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
