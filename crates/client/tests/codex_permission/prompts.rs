//! What the tests share: web's and api's Codex sessions in folders not named after them; the
//! `PermissionRequest` input recorded from Codex and the hooks that end a prompt; what the person's
//! runs show of a prompt, and what an agent's runs and the server must not hold.

use std::fs;
use std::path::{MAIN_SEPARATOR, Path};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::codex::{Codex, CodexProcess, Event, Thread, hook_command};
use crate::hooks::{Hooked, run_hook};
use crate::kit::{RECV, received, received_as_agent};
use crate::lines::JSON_FLAG;
use crate::machine::{Machine, Run};
use crate::senders::holds_word;
use crate::solo::{API, WEB};
use crate::world::{AGENT, dir_of, run};

/// The `PermissionRequest` input recorded from Codex 0.162.1 for a command that waits on the
/// person's approval, with the paths of its temporary directory.
const PERMISSION_REQUEST: &str = include_str!("recorded/permission-request.json");

/// The command that waits on the prompt and the model's justification of it, as recorded; no
/// output may hold either.
pub const COMMAND: &str = "touch prompted.txt";
pub const JUSTIFICATION: &str = "May I create prompted.txt?";

/// The word of the person's line of a prompt.
pub const PERMISSION_WORD: &str = "permission";

/// The `type` of a prompt in `--json`.
pub const RECORD_TYPE: &str = "permission_prompt";

/// The flag of `skald recv` that waits for what is to come.
pub const WAIT_FLAG: &str = "--wait";

/// How soon the person's waiting `skald recv --wait` shows a prompt after the path's event.
pub const WITHIN: Duration = Duration::from_secs(1);

/// How long the person's `skald recv --wait` must keep waiting before a prompt, with nothing else
/// to show.
pub const IDLE_WATCH: Duration = Duration::from_secs(2);

/// How long a test lets the device take in what a path told before the person's one run reads it:
/// twice [`WITHIN`].
pub const SETTLE: Duration = Duration::from_secs(2);

/// How long a test lets the device notice that a session's Codex process exited.
pub const EXIT_NOTICED: Duration = Duration::from_secs(5);

/// The turn after the recorded one.
const NEXT_TURN: &str = "01a1fcb4-1350-7eb2-8963-298e1962d078";

/// web's Codex session in `~/work/clientA/frontend` and api's in `~/work/clientA/backend`, not
/// started yet.
pub fn sessions(codex: &Codex) -> (Thread, Thread) {
    let machine = codex.machine();
    let web = Thread::new(
        machine,
        &dir_of(machine, "work/clientA/frontend"),
        Some(WEB),
    );
    let api = Thread::new(machine, &dir_of(machine, "work/clientA/backend"), Some(API));
    (web, api)
}

/// The [`sessions`], each started: its `SessionStart` run by Codex in this process.
pub fn web_and_api(codex: &Codex) -> (Thread, Thread) {
    let (web, api) = sessions(codex);
    codex.starts(&web);
    codex.starts(&api);
    (web, api)
}

/// The recorded `PermissionRequest` input as `of`'s: its thread, its folder and its rollout.
fn request_input(of: &Thread) -> Value {
    let mut input: Value =
        serde_json::from_str(PERMISSION_REQUEST).expect("a recorded input is JSON");
    input["session_id"] = json!(of.id);
    input["cwd"] = json!(of.dir.to_string_lossy());
    input["transcript_path"] = json!(of.rollout().to_string_lossy());
    input
}

/// The recorded turn of the prompt.
fn prompted_turn() -> String {
    let input: Value = serde_json::from_str(PERMISSION_REQUEST).expect("a recorded input is JSON");
    input["turn_id"].as_str().expect("a turn").to_owned()
}

/// `of`'s recorded input of `event` in `turn`.
fn input_in_turn(of: &Thread, event: Event, turn: &str) -> Value {
    let mut input: Value = serde_json::from_str(&of.input(event)).expect("an input is JSON");
    input["turn_id"] = json!(turn);
    input
}

/// Fails unless `hooked`, the run of `of`'s hook on `event`, exited with success and printed
/// nothing.
#[track_caller]
fn silent(hooked: &Hooked, of: &Thread, event: &str) {
    hooked.assert_silent(&format!("{event} of {}'s session", of.dir.display()));
}

/// `of` stops on the prompt: its `PermissionRequest` hook, run by Codex in this process, prints
/// nothing. Gives when the run returned.
#[track_caller]
pub fn prompts(codex: &Codex, of: &Thread) -> Instant {
    let input = request_input(of).to_string();
    let hooked = run_hook(&mut hook_command(&codex.helped, of, &[]), &input);
    let returned = Instant::now();
    silent(&hooked, of, "PermissionRequest");
    returned
}

/// As [`prompts`], with the hook run by the Codex process `process`.
#[track_caller]
pub fn prompts_by(codex: &Codex, process: &mut CodexProcess, of: &Thread) {
    let input = request_input(of).to_string();
    let hooked = process.run(&hook_command(&codex.helped, of, &[]), &input);
    silent(&hooked, of, "PermissionRequest");
}

/// The person allowed the command, which ran: `of`'s `PostToolUse` of it, in the prompt's turn,
/// prints nothing.
#[track_caller]
pub fn allowed(codex: &Codex, of: &Thread) {
    let mut input = input_in_turn(of, Event::PostToolUse, &prompted_turn());
    input["tool_input"] = json!({ "command": COMMAND });
    let hooked = run_hook(
        &mut hook_command(&codex.helped, of, &[]),
        &input.to_string(),
    );
    silent(&hooked, of, "PostToolUse");
}

/// The person declined the command, which runs no hook, and typed the next prompt: `of`'s
/// `UserPromptSubmit` of the next turn prints nothing.
#[track_caller]
pub fn next_turn(codex: &Codex, of: &Thread) {
    let input = input_in_turn(of, Event::UserPromptSubmit, NEXT_TURN);
    let hooked = run_hook(
        &mut hook_command(&codex.helped, of, &[]),
        &input.to_string(),
    );
    silent(&hooked, of, "UserPromptSubmit");
}

/// A whole turn of `of` without a prompt: `UserPromptSubmit`, `PostToolUse` and `Stop`, each
/// printing nothing.
#[track_caller]
pub fn whole_turn(codex: &Codex, of: &Thread) {
    for event in [Event::UserPromptSubmit, Event::PostToolUse, Event::Stop] {
        codex.silent(of, event, "a turn without a prompt");
    }
}

/// The person's `skald recv --wait`, started once nothing else waits for the person.
pub struct PersonWaits {
    run: Run,
}

impl PersonWaits {
    /// Runs the person's `skald recv`, so nothing waits, then starts the person's `skald recv
    /// --wait` through the helper; fails unless it is still waiting after [`IDLE_WATCH`].
    #[track_caller]
    pub fn start(codex: &Codex) -> Self {
        received(codex.machine());
        let mut run = codex.helped.start(&[], &[RECV, WAIT_FLAG]);
        thread::sleep(IDLE_WATCH);
        run.assert_running();
        assert!(
            permission_lines(&run.shown()).is_empty(),
            "a prompt before any: {}",
            run.shown()
        );
        PersonWaits { run }
    }

    /// Fails unless the run shows a prompt's line within [`WITHIN`] of `since`, the path's event,
    /// then exits with success, its output holding that one line. Gives the line.
    #[track_caller]
    pub fn ends_on_line(self, since: Instant) -> String {
        let mut run = self.run;
        loop {
            if !permission_lines(&run.shown()).is_empty() {
                break;
            }
            assert!(
                since.elapsed() <= WITHIN,
                "no prompt within {WITHIN:?} of the path's event: {}",
                run.shown()
            );
            if run.exited() {
                panic!("skald recv --wait exited without a prompt: {}", run.shown());
            }
            thread::sleep(Duration::from_millis(10));
        }
        let finished = run.finish();
        assert!(finished.success, "{finished:?}");
        let lines = permission_lines(&finished.shown());
        let [line] = lines.as_slice() else {
            panic!("not one prompt's line: {finished:?}");
        };
        line.clone()
    }
}

/// The lines of `shown`, a run's output, that hold the word [`PERMISSION_WORD`], in any case.
pub fn permission_lines(shown: &str) -> Vec<String> {
    (shown.lines())
        .filter(|line| holds_word(line, PERMISSION_WORD))
        .map(str::to_owned)
        .collect()
}

/// The lines of prompts in what one run of the person's `skald recv` on `machine` shows.
#[track_caller]
pub fn person_lines(machine: &Machine) -> Vec<String> {
    let finished = run(machine, &[RECV]);
    assert!(finished.success, "{finished:?}");
    permission_lines(&finished.shown())
}

/// The prompts in what one run of the person's `skald recv --json` on `machine` shows.
#[track_caller]
pub fn person_records(machine: &Machine) -> Vec<Value> {
    let finished = run(machine, &[RECV, JSON_FLAG]);
    assert!(finished.success, "{finished:?}");
    (finished.stdout.lines())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|object| object["type"] == RECORD_TYPE)
        .collect()
}

/// Fails unless one run of the person's `skald recv` on `codex`'s machine shows one prompt's line,
/// naming `of`'s agent and folder; gives it.
#[track_caller]
pub fn person_sees_line_of(codex: &Codex, of: &Thread) -> String {
    let lines = person_lines(codex.machine());
    let [line] = lines.as_slice() else {
        panic!("not one prompt's line: {lines:#?}");
    };
    assert_line_of(codex, line, of);
    line.clone()
}

/// Fails if the person's next `skald recv` on `machine` shows a prompt.
#[track_caller]
pub fn person_sees_none(machine: &Machine, when: &str) {
    let lines = person_lines(machine);
    assert!(lines.is_empty(), "{when}: {lines:#?}");
}

/// Whether `line` names `dir`, a folder under `home`: its path, its path as the system gives it
/// whole, or its path under the home as `~/…`.
fn names_folder(line: &str, dir: &Path, home: &Path) -> bool {
    let mut forms = vec![dir.to_string_lossy().into_owned()];
    if let Ok(whole) = fs::canonicalize(dir) {
        let whole = whole.to_string_lossy().into_owned();
        forms.push(whole.trim_start_matches(r"\\?\").to_owned());
    }
    if let Ok(under) = dir.strip_prefix(home) {
        let under = under.to_string_lossy().into_owned();
        forms.push(format!("~{MAIN_SEPARATOR}{under}"));
        forms.push(format!("~/{}", under.replace('\\', "/")));
    }
    forms.iter().any(|form| line.contains(form.as_str()))
}

/// Fails unless `line`, the person's line of a prompt on `codex`'s machine, names `of`'s agent
/// and folder and holds nothing of the command.
#[track_caller]
pub fn assert_line_of(codex: &Codex, line: &str, of: &Thread) {
    let agent = of.agent.as_deref().expect("an agent");
    assert!(holds_word(line, agent), "the line names no {agent}: {line}");
    let home = codex.machine().root().join("home");
    assert!(
        names_folder(line, &of.dir, &home),
        "the line names no folder {}: {line}",
        of.dir.display()
    );
    assert_nothing_of_the_command(line, "the person's line");
}

/// Fails if `line`, the person's line of a prompt on `codex`'s machine, names `of`'s agent or
/// folder.
#[track_caller]
pub fn assert_line_not_of(codex: &Codex, line: &str, of: &Thread) {
    let agent = of.agent.as_deref().expect("an agent");
    assert!(!holds_word(line, agent), "the line names {agent}: {line}");
    let home = codex.machine().root().join("home");
    assert!(
        !names_folder(line, &of.dir, &home),
        "the line names {}: {line}",
        of.dir.display()
    );
}

/// Fails if `shown` holds the command or its justification.
#[track_caller]
pub fn assert_nothing_of_the_command(shown: &str, what: &str) {
    for part in [COMMAND, JUSTIFICATION] {
        assert!(!shown.contains(part), "{what} holds {part:?}: {shown}");
    }
}

/// Now, in Unix seconds.
pub fn unix_now() -> u64 {
    (SystemTime::now().duration_since(UNIX_EPOCH))
        .expect("after 1970")
        .as_secs()
}

/// Of `records`, the one prompt of `of`; fails unless there is exactly one, of [`RECORD_TYPE`],
/// naming `of`'s agent, with a `timestamp` in `began`, Unix seconds, and nothing of the command.
#[track_caller]
pub fn record_of<'a>(
    codex: &Codex,
    records: &'a [Value],
    of: &Thread,
    began: [u64; 2],
) -> &'a Value {
    let found: Vec<&Value> = (records.iter())
        .filter(|record| record["session"] == of.id.as_str())
        .collect();
    let [record] = found.as_slice() else {
        panic!("not one prompt of {}: {records:#?}", of.id);
    };
    assert_eq!(record["type"], RECORD_TYPE, "{record}");
    let agent = of.agent.as_deref().expect("an agent");
    let named = record["agent"].as_str().unwrap_or_default();
    assert!(
        named == agent || named == codex.web.solo.short(agent),
        "the prompt names {named:?}, not {agent}: {record}"
    );
    let at = record["timestamp"].as_u64();
    assert!(
        at.is_some_and(|at| began[0] <= at && at <= began[1] + 1),
        "the prompt began outside {began:?}: {record}"
    );
    assert_nothing_of_the_command(&record.to_string(), "the prompt's object");
    record
}

/// Fails unless `record` has not ended.
#[track_caller]
pub fn assert_waits(record: &Value) {
    assert!(
        record.get("ended").is_none_or(Value::is_null),
        "the prompt ended: {record}"
    );
}

/// Fails unless `record` ended at or after it began and at most at `by`, Unix seconds.
#[track_caller]
pub fn assert_ended(record: &Value, by: u64) {
    let began = record["timestamp"].as_u64().unwrap_or(u64::MAX);
    let ended = record["ended"].as_u64();
    assert!(
        ended.is_some_and(|ended| began <= ended && ended <= by + 1),
        "the prompt did not end between {began} and {by}: {record}"
    );
}

/// Fails if what `of`'s agent's runs show in its folder on `codex`'s machine holds anything of
/// `of`'s prompt: its `skald recv`, with `--json` too, and, for web, the `inbox` of web's MCP
/// session.
#[track_caller]
pub fn agent_sees_nothing(codex: &mut Codex, of: &Thread, prompted: &Thread) {
    let agent = of.agent.clone().expect("an agent");
    let machine = codex.machine();
    let plain = received_as_agent(machine, &of.dir, &agent).shown();
    let json = (machine.start_in(&of.dir, &[AGENT, &agent, RECV, JSON_FLAG])).finish();
    assert!(json.success, "{json:?}");
    let mut shown = vec![("skald recv", plain), ("skald recv --json", json.shown())];
    if agent == WEB {
        shown.push(("inbox", codex.web.session.inbox()));
    }
    for (what, shown) in shown {
        let what = format!("{agent}'s {what}");
        assert!(permission_lines(&shown).is_empty(), "{what}: {shown}");
        assert!(
            !shown.contains(&prompted.id),
            "{what} names the session: {shown}"
        );
        assert_nothing_of_the_command(&shown, &what);
    }
}

/// How many envelopes the server of `codex`'s machine stores: the rows of its envelopes, the one
/// table with a column `envelope`.
pub fn envelopes(codex: &Codex) -> usize {
    (codex.web.solo.talk.cloud().rows().iter())
        .map(|table| table.matches("<envelope>").count())
        .sum()
}
