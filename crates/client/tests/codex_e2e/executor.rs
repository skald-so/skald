use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use skald_adapters::codex::Executor;
use skald_adapters::executor::{Mode, Request};
use skald_proto::TrustLevel;

use crate::machine::TestDir;
use crate::session::{codex_path, command};
use crate::stub::{DONE, PLACEHOLDER, ResponsesReply, Stub, output_of, played};

const MARKER: &str = "fixture-blocked-environment-441";
const MCP: &str = "fixture_mcp_442";
const HOOK: &str = "fixture-hook-443";
const PLUGIN: &str = "fixture-plugin-444";
const REQUEST: &str = "Inspect the fixture input.";

fn plant(home: &Path) {
    fs::create_dir_all(home.join("plugins").join(PLUGIN).join(".codex-plugin"))
        .expect("plugin directory");
    fs::write(
        home.join("config.toml"),
        format!("[mcp_servers.{MCP}]\ncommand = \"{MCP}\"\n\n[plugins.\"{PLUGIN}@local\"]\nenabled = true\n"),
    ).expect("fixture config");
    fs::write(
        home.join("hooks.json"),
        json!({"hooks": {"SessionStart": [{"hooks": [{"type":"command","command": format!("echo {HOOK}") }]}]}}).to_string(),
    ).expect("fixture hooks");
    fs::write(
        home.join("plugins")
            .join(PLUGIN)
            .join(".codex-plugin/plugin.json"),
        json!({"name":PLUGIN, "version":"1.0.0", "description":PLUGIN}).to_string(),
    )
    .expect("fixture plugin");
}

/// Gives the real CLI an explicit loopback provider. Its built-in provider can select a
/// WebSocket endpoint independently of OPENAI_BASE_URL. The launcher passes through the adapter's
/// arguments and environment.
fn local_codex(program: &Path, dir: &Path, url: &str) -> PathBuf {
    let provider = format!(
        r#"model_providers.fixture={{name="fixture",base_url="{url}/v1",env_key="OPENAI_API_KEY",wire_api="responses"}}"#
    );
    let model = [
        "-c",
        r#"model_provider="fixture""#,
        "-c",
        r#"model="skald-stub""#,
        "-c",
        &provider,
    ];
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let quote = |arg: &str| format!("'{}'", arg.replace('\'', "'\\''"));
        let args = model.map(quote).join(" ");
        let file = dir.join("local-codex");
        fs::write(
            &file,
            format!(
                "#!/bin/sh\nexec {} \"$@\" {args}\n",
                quote(&program.to_string_lossy())
            ),
        )
        .expect("loopback launcher");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).expect("executable launcher");
        file
    }
    #[cfg(windows)]
    {
        let quote = |arg: &str| format!(r#""{}""#, arg.replace('"', r#"\""#));
        let args = model.map(quote).join(" ");
        let file = dir.join("local-codex.cmd");
        fs::write(
            &file,
            format!("@echo off\r\n\"{}\" %* {args}\r\n", program.display()),
        )
        .expect("loopback launcher");
        file
    }
}

fn executor(codex: &Path, home: &Path, url: &str) -> Executor {
    let inherited = [
        "PATH",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
    ];
    let mut env: Vec<(OsString, OsString)> = inherited
        .into_iter()
        .filter_map(|name| env::var_os(name).map(|value| (name.into(), value)))
        .collect();
    for name in ["HOME", "USERPROFILE", "TMPDIR", "TEMP", "TMP"] {
        env.push((name.into(), home.into()));
    }
    env.extend([
        ("CODEX_HOME".into(), home.join("config").into()),
        ("OPENAI_API_KEY".into(), PLACEHOLDER.into()),
        ("HTTP_PROXY".into(), url.into()),
        ("HTTPS_PROXY".into(), url.into()),
        ("NO_PROXY".into(), "127.0.0.1".into()),
        ("SKALD_E2E_MARKER".into(), MARKER.into()),
    ]);
    Executor::new(codex).environment(env)
}

fn request() -> Request {
    Request {
        from: "acme/alice/reviewer@skald.example"
            .parse()
            .expect("fixture sender"),
        level: TrustLevel::Own,
        text: REQUEST.to_owned(),
    }
}

#[test]
fn codex_exec_runs_in_its_folder_without_user_environment_or_configuration() {
    let Some(codex) = codex_path() else { return };
    let stub = Stub::start_responses(|body| {
        if played(body) != 0 || !body.to_string().contains(REQUEST) {
            return ResponsesReply::Done;
        }
        let inspect = if cfg!(windows) {
            "Get-Content fixture-input.txt; Write-Output \"$env:SKALD_E2E_MARKER\""
        } else {
            "cat fixture-input.txt; printf '%s' \"${SKALD_E2E_MARKER-}\""
        };
        command(inspect, json!({}))
    });
    let scratch = TestDir::new("codex-e2e-executor");
    let home = scratch.join("home");
    let folder = scratch.join("work");
    plant(&home.join("config"));
    fs::create_dir(&folder).expect("executor folder");
    fs::write(
        folder.join("fixture-input.txt"),
        "fixture-executor-in-folder-445",
    )
    .expect("workspace fixture");
    let launcher = local_codex(&codex, &scratch, stub.url());
    let executor = executor(&launcher, &home, stub.url());
    let run = executor
        .start(&folder, Mode::Full, &request())
        .expect("full executor starts");
    let (done, result) = mpsc::channel();
    thread::scope(|scope| {
        scope.spawn(|| {
            let _ = done.send(run.wait());
        });
        let outcome = result
            .recv_timeout(Duration::from_secs(90))
            .unwrap_or_else(|error| {
                run.stop();
                panic!(
                    "Codex executor did not finish after {} loopback requests: {error}",
                    stub.requests().len()
                );
            });
        assert!(outcome.finished, "executor failed: {outcome:?}");
        assert_eq!(outcome.answer, DONE);
    });

    let calls: Vec<Value> = stub.requests();
    assert!(
        calls
            .iter()
            .filter_map(|body| output_of(body, 0))
            .any(|output| output.contains("fixture-executor-in-folder-445")),
        "executor did not read the workspace fixture"
    );
    for body in calls {
        for marker in [MARKER, MCP, HOOK, PLUGIN] {
            assert!(
                !body.to_string().contains(marker),
                "executor inherited {marker}"
            );
        }
    }
    stub.assert_no_foreign_credential();
}

#[test]
fn codex_read_only_is_refused_before_a_model_request() {
    let Some(codex) = codex_path() else { return };
    let stub = Stub::start_responses(|_| ResponsesReply::Done);
    let scratch = TestDir::new("codex-e2e-refused");
    let executor = executor(&codex, &scratch, stub.url());
    let Err(refusal) = executor.start(&scratch, Mode::ReadOnly, &request()) else {
        panic!("read-only started even though this platform has no supported mode");
    };
    assert!(
        refusal.to_string().contains("read-only sandbox"),
        "{refusal}"
    );
    assert!(
        stub.requests().is_empty(),
        "refused executor contacted the model"
    );
    stub.assert_no_foreign_credential();
}
