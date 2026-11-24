//! Export through the CLI using the same disposable history fixtures as `skald history`.

use std::fs;
use std::path::Path;

use serde_json::Value;
use skald_crypto::HistoryKey;
use skald_proto::{Cbor as _, ChannelId, MessageId, Role};

use crate::chain::key_of;
use crate::fixtures::person_address;
use crate::joiners::joins_after_them;
use crate::keys::holds;
use crate::kit::{
    Change, assert_names_none, b64, bind_client_a, create_space, issue_channel, latest, received,
    revise, with_channel,
};
#[cfg(unix)]
use crate::machine::Run;
use crate::machine::{Finished, Machine};
use crate::reading::{
    Duo, Said, body_of, rewrite_entries, rfc3339, rotates, says, sealed_as_is, sends, warned_of,
    without_key,
};
use crate::talk::{Line, Talk};
use crate::world::{AGENT, CLIENT_A, CLIENT_B, path};

const CLAUDE: &str = "claude-h5t3mq";
const MARKDOWN_BODY: &str =
    "# Still a message\n[link](https://invalid.example)\n<script>literal</script>\n```\n~~~\nCafé";
const FORGED: &str = "a message signed by a different device";
const UNREADABLE: &str = "a message encrypted with a key this reader lacks";

fn args<'a>(scope: [&'a str; 2], format: &'a str, output: &'a Path) -> [&'a str; 7] {
    let [flag, value] = scope;
    [
        "export",
        flag,
        value,
        "--format",
        format,
        "--output",
        path(output),
    ]
}

fn saved(output: &Path, run: &Finished) -> String {
    assert!(run.success, "{run:?}");
    let content = fs::read_to_string(output).expect("a readable export file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(output)
            .expect("export metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "group or others can access the export");
    }
    content
}

fn assert_json(file: &str, entries: &[(ChannelId, &Said)]) {
    let exported: Vec<Value> = serde_json::from_str(file).expect("an array of messages");
    assert_eq!(exported.len(), entries.len(), "{exported:?}");
    for (value, (channel, said)) in exported.iter().zip(entries) {
        assert_eq!(value["channel"], channel.to_string());
        assert_eq!(value["from"], said.from);
        assert_eq!(value["device"], said.device);
        assert_eq!(value["body"], said.text);
        let time = value["timestamp"].as_str().expect("a timestamp");
        assert!(
            (said.sent.0..=said.sent.1).any(|sec| time == rfc3339(sec)),
            "{value:?}"
        );
    }
}

/// Recognizes a Markdown closing fence. An internal line of the message must not close it.
fn closes(line: &str, opening: &str) -> bool {
    let marker = opening.as_bytes()[0];
    let line = line.trim_start_matches(' ');
    let width = line.bytes().take_while(|byte| *byte == marker).count();
    width >= opening.len() && line[width..].trim().is_empty()
}

fn literal_position(markdown: &str, body: &str) -> usize {
    let fenced = format!("\n{body}\n");
    let mut literal = markdown.match_indices(&fenced).filter_map(|(start, text)| {
        let opening = markdown[..start].lines().last()?.trim_start();
        let marker = *opening.as_bytes().first()?;
        if !matches!(marker, b'`' | b'~') {
            return None;
        }
        let width = opening.bytes().take_while(|byte| *byte == marker).count();
        let opening = &opening[..width];
        let closing = markdown[start + text.len()..].lines().next()?;
        (width >= 3 && closes(closing, opening) && !body.lines().any(|line| closes(line, opening)))
            .then_some(start)
    });
    let indented: String = body.lines().map(|line| format!("    {line}\n")).collect();
    let indented = format!("\n{indented}");
    let (first, next) = match literal.next() {
        Some(first) => (Some(first), literal.next()),
        None => {
            let mut literal = markdown.match_indices(&indented).map(|(start, _)| start);
            (literal.next(), literal.next())
        }
    };
    assert!(next.is_none(), "a literal body appears twice: {body:?}");
    first.unwrap_or_else(|| panic!("body is not literal Markdown: {body:?}\n{markdown}"))
}

fn assert_markdown(file: &str, entries: &[(ChannelId, &Said)]) {
    let file = file.replace("\r\n", "\n");
    let mut previous = 0;
    for (channel, said) in entries {
        let marker = said.text.lines().next().expect("a message body");
        assert_eq!(
            file.matches(marker).count(),
            1,
            "not exported once: {marker:?}"
        );
        let position = literal_position(&file, &said.text);
        assert!(position >= previous, "messages are out of order: {file}");
        let metadata = &file[previous..position];
        for field in [channel.to_string().as_str(), &said.from, said.device] {
            assert!(
                metadata.contains(field),
                "missing {field} for {:?}: {file}",
                said.text
            );
        }
        assert!(
            (said.sent.0..=said.sent.1).any(|sec| metadata.contains(&rfc3339(sec))),
            "missing time for {:?}: {file}",
            said.text
        );
        previous = position + 1;
    }
}

fn assert_absent(file: &str, omitted: &[&str]) {
    for value in omitted {
        assert!(
            !file.contains(value),
            "export contains excluded data: {value:?}"
        );
    }
}

fn export_formats(
    machine: &Machine,
    channel: ChannelId,
    entries: &[(ChannelId, &Said)],
) -> [(String, Finished); 2] {
    let channel = channel.to_string();
    [("json", "json"), ("markdown", "md")].map(|(format, extension)| {
        let output = machine.root().join(format!("message history.{extension}"));
        let run = machine
            .start(&args(["--channel", &channel], format, &output))
            .finish();
        let file = saved(&output, &run);
        if format == "json" {
            assert_json(&file, entries);
        } else {
            assert_markdown(&file, entries);
        }
        (file, run)
    })
}

#[test]
fn export_formats_save_verified_messages_once_across_epochs() {
    let mut duo = Duo::new(
        &[4_166_017_001, 4_166_017_002],
        &["ada-exports", "bob-writes-history"],
    );
    let (first, _, first_entry) = duo.bob_says(&duo.bob_s(1, "the first exported message"));
    let at = duo.at;
    assert_eq!(
        rotates(&duo.talk.world.cloud, &mut duo.talk.played[0], at),
        2
    );
    let (second, _, second_entry) = duo.bob_says(&duo.bob_s(2, MARKDOWN_BODY));
    assert_eq!(
        rotates(&duo.talk.world.cloud, &mut duo.talk.played[0], at),
        3
    );
    let (third, _, third_entry) = duo.bob_says(&duo.bob_s(3, "Bob writes after rotation"));
    received(&duo.talk.ada.machine);
    let bob = &duo.talk.played[0];
    let forged = body_of(
        bob.device(),
        at.1,
        (MessageId::generate(), &duo.bob_s(4, FORGED)),
    );
    let forged = sealed_as_is(key_of(bob, at.1, 3), (at.1, 3), &forged, &duo.talk.ada.key);
    let unreadable = body_of(
        bob.device(),
        at.1,
        (MessageId::generate(), &duo.bob_s(5, UNREADABLE)),
    );
    let unreadable = HistoryKey::from_bytes(&[0x42; 16])
        .seal_entry(9, &unreadable, &bob.person.key)
        .expect("an entry under an unavailable key");
    let unordered = [
        third_entry,
        second_entry,
        first_entry.clone(),
        forged.to_cbor(),
        unreadable.to_cbor(),
        first_entry,
    ]
    .map(|entry| Value::String(b64(&entry)));
    rewrite_entries(duo.talk.cloud(), at, move |entries| {
        *entries = unordered.to_vec();
    });

    let machine = &duo.talk.ada.machine;
    let expected = [(at.1, &first), (at.1, &second), (at.1, &third)];
    for (file, run) in export_formats(machine, at.1, &expected) {
        assert_absent(
            &file,
            &[FORGED, UNREADABLE, &duo.talk.ada.token, &bob.person.token],
        );
        assert!(
            !holds(file.as_bytes(), &*duo.talk.ada.key.to_bytes()),
            "export contains the device's signing key"
        );
        assert!(
            warned_of(&run, bob.device())
                .iter()
                .any(|line| line.contains("signature")),
            "the invalid signature was not reported: {run:?}"
        );
        assert_eq!(
            without_key(&run),
            Some(1),
            "the unreadable entry was not counted"
        );
        assert_absent(&run.shown(), &[FORGED, UNREADABLE]);
    }
}

#[test]
fn export_formats_respect_the_join_cutoff() {
    let joined = joins_after_them(
        false,
        &[4_166_018_001, 4_166_018_002, 4_166_018_003],
        &[
            "ada-exports-from-join",
            "bob-has-old-history",
            "carol-has-old-history",
        ],
    );
    let machine = &joined.talk.ada.machine;
    let expected = [(joined.channel, &joined.after)];
    let earlier: Vec<_> = joined
        .before
        .iter()
        .map(|said| said.text.as_str())
        .collect();
    for (file, run) in export_formats(machine, joined.channel, &expected) {
        assert_absent(&file, &earlier);
        assert_absent(&run.shown(), &earlier);
    }
}

#[test]
fn export_agent_reads_only_the_selected_scope() {
    let mut talk = Talk::in_space(
        CLIENT_B,
        &[4_166_019_001, 4_166_019_002],
        &["ada-exports-for-agent", "bob-writes-for-clients"],
        false,
    );
    let cloud = &talk.world.cloud;
    let a = create_space(cloud, &talk.ada, CLIENT_A);
    let channel_a = with_channel(
        cloud,
        a.id,
        &talk.ada,
        &[(&talk.played[0].person, Role::Admin)],
    );
    let at_b = talk.at();
    let from_bob_b = talk.address(&talk.played[0].person);
    received(&talk.ada.machine);
    let machine = &talk.ada.machine;
    let folder = bind_client_a(machine);
    let bob = &mut talk.played[0];
    let from_bob_a = person_address(CLIENT_A, &bob.person);
    let private = issue_channel(cloud, &bob.person.token, a.id);
    revise(
        cloud,
        &latest(cloud, &bob.person.token, a.id),
        &bob.person,
        &Change {
            channels: vec![(private, vec![&bob.person])],
            ..Change::default()
        },
    );
    bob.creates_alone(cloud, (a.id, private));
    bob.uploads_group_info(cloud, (a.id, private));
    let first = sends(
        &talk.ada,
        (CLIENT_A, a.default),
        "the first client A channel",
    );
    let (second, ..) = says(
        cloud,
        bob,
        (a.id, channel_a),
        &Line::new(&from_bob_a, 1, "the second client A channel"),
    );
    let (other, ..) = says(
        cloud,
        bob,
        at_b,
        &Line::new(&from_bob_b, 1, "the private client B message"),
    );
    let (private_message, ..) = says(
        cloud,
        bob,
        (a.id, private),
        &Line::new(&from_bob_a, 1, "a message for Bob alone"),
    );
    let channel_a_text = channel_a.to_string();
    let channel_b_text = at_b.1.to_string();
    let person_file = machine.root().join("person.json");
    let person = machine
        .start(&args(["--channel", &channel_b_text], "json", &person_file))
        .finish();
    assert_json(&saved(&person_file, &person), &[(at_b.1, &other)]);

    let run_agent = |scope, output: &Path| {
        let command = [vec![AGENT, CLAUDE], args(scope, "json", output).to_vec()].concat();
        machine.start_in(&folder, &command).finish()
    };
    let mark = cloud.relay.mark();
    for (select, scope, expected) in [
        (
            "one channel",
            ["--channel", channel_a_text.as_str()],
            &[(channel_a, &second)][..],
        ),
        (
            "one space",
            ["--space", CLIENT_A],
            &[(a.default, &first), (channel_a, &second)][..],
        ),
    ] {
        let output = machine.root().join(format!("{select}.json"));
        let file = saved(&output, &run_agent(scope, &output));
        assert_json(&file, expected);
        assert_absent(
            &file,
            &[
                &other.text,
                &private_message.text,
                &talk.ada.token,
                &bob.person.token,
            ],
        );
    }
    for scope in [["--space", CLIENT_B], ["--channel", &channel_b_text]] {
        let output = machine.root().join("refused.json");
        let refused = run_agent(scope, &output);
        assert!(!refused.success && !output.exists(), "{refused:?}");
        assert_names_none(
            &refused,
            &[
                CLIENT_B.to_owned(),
                talk.space.id.to_string(),
                channel_b_text.clone(),
                other.text.clone(),
            ],
        );
    }
    let output = machine.root().join("invalid scope.json");
    let base = [
        AGENT,
        CLAUDE,
        "export",
        "--format",
        "json",
        "--output",
        path(&output),
    ];
    for extra in [
        &[][..],
        &["--space", CLIENT_A, "--channel", &channel_a_text][..],
    ] {
        let invalid = machine
            .start_in(&folder, &[base.as_slice(), extra].concat())
            .finish();
        assert!(!invalid.success && !output.exists(), "{invalid:?}");
    }
    let foreign = format!("/spaces/{}/channels/", talk.space.id);
    let private_history = format!("/channels/{private}/history/");
    assert!(
        !cloud
            .relay
            .sent_since(mark)
            .iter()
            .any(|sent| sent.head.contains(&private_history)
                || (sent.head.contains(&foreign) && sent.head.contains("/history/"))),
        "the agent requested history outside its scope"
    );
}

#[test]
fn export_write_failure_does_not_publish_or_replace_a_file() {
    let mut duo = Duo::new(
        &[4_166_020_001, 4_166_020_002],
        &["ada-saves-a-file", "bob-writes-for-a-file"],
    );
    let (first, ..) = duo.bob_says(&duo.bob_s(1, "the first complete export"));
    #[cfg(unix)]
    let large_channel = {
        let at = (duo.at.0, duo.talk.space.default);
        let large = "archive line ".repeat(6_000);
        let line = Line::new(&duo.from_bob, 1, &large);
        says(&duo.talk.world.cloud, &mut duo.talk.played[0], at, &line);
        at.1.to_string()
    };
    let machine = &duo.talk.ada.machine;
    let directory = machine.root().join("exports");
    fs::create_dir(&directory).expect("an output directory");
    let output = directory.join("existing.json");
    let channel = duo.at.1.to_string();
    let scope = ["--channel", &channel];
    let run = machine.start(&args(scope, "json", &output)).finish();
    let original = saved(&output, &run);
    assert_json(&original, &[(duo.at.1, &first)]);

    let missing = output.join("new.json");
    let run = machine.start(&args(scope, "json", &missing)).finish();
    assert!(!run.success && !missing.exists(), "{run:?}");
    let no_path = machine
        .start(&["export", "--channel", &channel, "--format", "json"])
        .finish();
    assert!(!no_path.success, "{no_path:?}");

    #[cfg(unix)]
    {
        // The other channel's export exceeds the limit and would leave a partial direct write.
        let absent = directory.join("new.json");
        for target in [&absent, &output] {
            let command_args = args(["--channel", &large_channel], "json", target);
            let mut command = machine.command_in(machine.root(), &command_args);
            use std::os::unix::process::CommandExt as _;
            // SAFETY: the child hook makes only async-signal-safe POSIX calls.
            unsafe {
                command.pre_exec(|| {
                    let limit = libc::rlimit {
                        rlim_cur: 32_768,
                        rlim_max: 32_768,
                    };
                    if libc::signal(libc::SIGXFSZ, libc::SIG_IGN) == libc::SIG_ERR
                        || libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let run = Run::spawn(&mut command).finish();
            assert!(
                !run.success,
                "export succeeded with a file-size limit: {run:?}"
            );
        }
        assert!(!absent.exists(), "a partial export was published");
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let open = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&output)
            .expect("a file that cannot be replaced while open");
        let run = machine.start(&args(scope, "json", &output)).finish();
        assert!(!run.success, "export replaced an open file: {run:?}");
        drop(open);
    }

    assert_eq!(
        fs::read_to_string(&output).expect("the prior export"),
        original
    );
    let files: Vec<_> = fs::read_dir(&directory)
        .expect("exports")
        .map(|entry| entry.expect("a directory entry").path())
        .collect();
    assert_eq!(files, vec![output], "the failed export left output behind");
}
