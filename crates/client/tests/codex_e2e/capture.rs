use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::app_server::AppServer;
use crate::host::Live;

const ACTOR: &str = "skald-e2e-fixture";

pub fn build_actor(dir: &Path) -> PathBuf {
    let exe = dir.join(format!("{ACTOR}{}", env::consts::EXE_SUFFIX));
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let status = Command::new(rustc)
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/codex_e2e/actor.rs"
        ))
        .args(["--edition", "2024", "-o"])
        .arg(&exe)
        .status()
        .expect("compile native Codex fixture");
    assert!(status.success(), "compile native Codex fixture: {status}");
    exe
}

pub struct Capture(PathBuf);

impl Capture {
    pub fn install(live: &Live, settings: &mut Vec<(String, String)>) -> Self {
        let skald = live.codex.helped.exe();
        build_actor(skald.parent().expect("Skald binary folder"));
        let records = live.home().join("hook-stdout");
        fs::create_dir(&records).expect("create hook record directory");
        for (name, value) in [
            ("SKALD_E2E_HOOK_EXE", skald),
            ("SKALD_E2E_HOOK_CAPTURE", &records),
        ] {
            settings.push((name.to_owned(), value.to_string_lossy().into_owned()));
        }

        // Wrap the hooks written by setup after Codex has trusted their original entries.
        let config = live.home().join("config.toml");
        let mut doc: toml_edit::DocumentMut = fs::read_to_string(&config)
            .expect("fixture config")
            .parse()
            .expect("fixture TOML");
        let exec = if cfg!(windows) { "" } else { "exec " };
        let events = [
            "SessionStart",
            "UserPromptSubmit",
            "PostToolUse",
            "Stop",
            "PermissionRequest",
        ];
        for event in events {
            let command = &mut doc["hooks"][event][0]["hooks"][0]["command"];
            assert!(
                command
                    .as_str()
                    .expect("setup hook command")
                    .ends_with(" hook codex")
            );
            *command = toml_edit::value(format!("{exec}{ACTOR} capture {event}"));
        }
        fs::write(config, doc.to_string()).expect("configure hook stdout capture");
        let mut app_server = AppServer::start(live.command(&live.program, settings));
        let (_, trusted) = app_server.trust_hooks(live.dir());
        assert_eq!(trusted.len(), events.len(), "wrapped hooks: {trusted:?}");
        Self(records)
    }

    pub fn outputs(&self) -> Vec<(String, Vec<u8>)> {
        fs::read_dir(&self.0)
            .expect("hook record directory")
            .filter_map(|entry| {
                let entry = entry.expect("hook record");
                if entry.path().extension().is_some() {
                    return None;
                }
                let filename = entry.file_name().into_string().expect("hook record name");
                let (event, _) = filename.split_once('-').expect("hook event and process");
                Some((
                    event.to_owned(),
                    fs::read(entry.path()).expect("hook stdout"),
                ))
            })
            .collect()
    }
}
