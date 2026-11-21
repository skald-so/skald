//! The person sets and lists the delivery mode and the text setting per channel and sender trust
//! level, with the wakes of each channel; as an agent, and with an unknown or incomplete setting,
//! the commands change nothing and say why.

use skald_proto::ChannelId;

use crate::delivery::{
    BATCH, INLINE, LEVELS, LIST, NOW, PULL, SERVICE, SET_MODE, SET_TEXT, SIGNAL, WAKES, WINDOW,
    listed, mode_of, of_channel, other_channel, person_sets, set_mode, set_text, text_of, wakes_of,
};
use crate::kit::issue_channel;
use crate::senders::{OWN, PEER, holds_word};
use crate::solo::{Solo, WEB};
use crate::world::AGENT;

/// What a channel's settings are: per level of [`LEVELS`], its mode, its window in seconds, and
/// its text setting, `None` for `own`.
type Settings = [(&'static str, Option<u64>, Option<&'static str>); 3];

/// The settings of a channel no one set.
const DEFAULTS: Settings = [
    (NOW, None, None),
    (NOW, None, Some(SIGNAL)),
    (NOW, None, Some(SIGNAL)),
];

/// Fails unless the person's `skald delivery-modes --json` gives `channel` `settings` and
/// `wakes`, and its plain listing shows them.
#[track_caller]
fn assert_listed(solo: &Solo, channel: ChannelId, settings: Settings, wakes: u64) {
    let objects = listed(solo);
    let object = of_channel(&objects, channel);
    for (level, (mode, window, text)) in LEVELS.into_iter().zip(settings) {
        assert_eq!(
            mode_of(object, level),
            (mode.to_owned(), window),
            "{level}: {object}"
        );
        if let Some(text) = text {
            assert_eq!(text_of(object, level), text, "{level}: {object}");
        }
    }
    assert_eq!(wakes_of(object), wakes, "{object}");

    let shown = person_sets(solo, &[LIST]).shown();
    let id = channel.to_string();
    for (level, (mode, _, text)) in LEVELS.into_iter().zip(settings) {
        let lines: Vec<&str> = (shown.lines())
            .filter(|line| line.contains(&id) && holds_word(line, level))
            .collect();
        let [line] = lines.as_slice() else {
            panic!("not one line of {id} at {level}: {shown}");
        };
        assert!(holds_word(line, mode), "not {mode}: {line}");
        if let Some(text) = text {
            assert!(holds_word(line, text), "not {text}: {line}");
        }
    }
    let count = wakes.to_string();
    let woken: Vec<&str> = (shown.lines())
        .filter(|line| line.contains(&id) && holds_word(line, WAKES))
        .collect();
    match woken.as_slice() {
        [line] if holds_word(line, &count) => {}
        _ => panic!("no line of {id} with {count} {WAKES}: {shown}"),
    }
}

/// Ada's machine is alone in `clienta`. The person's `skald delivery-modes` there lists both of its
/// channels, the default one and the other, and no other: each level `now`, without a window,
/// `peer` and `service` `signal`, and no wakes.
#[test]
fn person_lists_now_and_signal_for_each_channel_and_level_by_default() {
    let solo = Solo::new(5_207_101_001, "ada-lists-delivery-defaults");

    let objects = listed(&solo);

    assert_eq!(objects.len(), 2, "not the two channels: {objects:#?}");
    for channel in [solo.default_channel(), other_channel(&solo)] {
        assert_listed(&solo, channel, DEFAULTS, 0);
    }
}

/// The person sets, in the default channel, `peer` to `batch` with a window of 5 minutes and its
/// text to `inline`, and `service` to `pull`; in the other channel, `own` to `batch` with a window
/// of 90 seconds and `service`'s text to `inline`. The listing gives each as set, the window in
/// seconds, and every other setting as before. The person sets `peer` in the default channel back
/// to `now` and its text to `signal`: the listing gives them so, without a window, and nothing
/// else changed.
#[test]
fn person_sets_each_setting_per_channel_and_level_and_lists_it() {
    let solo = Solo::new(5_207_102_001, "ada-sets-delivery-modes");
    let (default, other) = (solo.default_channel(), other_channel(&solo));

    set_mode(&solo, default, PEER, BATCH, Some("5m"));
    set_text(&solo, default, PEER, INLINE);
    set_mode(&solo, default, SERVICE, PULL, None);
    set_mode(&solo, other, OWN, BATCH, Some("90s"));
    set_text(&solo, other, SERVICE, INLINE);

    let default_set: Settings = [
        (NOW, None, None),
        (BATCH, Some(300), Some(INLINE)),
        (PULL, None, Some(SIGNAL)),
    ];
    let other_set: Settings = [
        (BATCH, Some(90), None),
        (NOW, None, Some(SIGNAL)),
        (NOW, None, Some(INLINE)),
    ];
    assert_listed(&solo, default, default_set, 0);
    assert_listed(&solo, other, other_set, 0);

    set_mode(&solo, default, PEER, NOW, None);
    set_text(&solo, default, PEER, SIGNAL);

    let default_back: Settings = [
        (NOW, None, None),
        (NOW, None, Some(SIGNAL)),
        (PULL, None, Some(SIGNAL)),
    ];
    assert_listed(&solo, default, default_back, 0);
    assert_listed(&solo, other, other_set, 0);
}

/// The person sets `peer` in the default channel to `pull`. As web, in web's folder, setting it
/// back to `now`, setting its text to `inline`, and listing the settings each fail and say why;
/// the person's listing still gives `pull` and `signal`.
#[test]
fn as_an_agent_setting_or_listing_is_refused_and_changes_nothing() {
    let solo = Solo::new(5_207_103_001, "ada-agent-cannot-set-delivery");
    let default = solo.default_channel();
    set_mode(&solo, default, PEER, PULL, None);
    let id = default.to_string();

    for args in [
        vec![AGENT, WEB, SET_MODE, id.as_str(), PEER, NOW],
        vec![AGENT, WEB, SET_TEXT, id.as_str(), PEER, INLINE],
        vec![AGENT, WEB, LIST],
    ] {
        let tried = solo.machine().start_in(&solo.dir(WEB), &args).finish();
        assert!(!tried.success, "{args:?}: {tried:?}");
        assert!(!tried.shown().trim().is_empty(), "says nothing: {tried:?}");
    }

    let set: Settings = [
        (NOW, None, None),
        (PULL, None, Some(SIGNAL)),
        (NOW, None, Some(SIGNAL)),
    ];
    assert_listed(&solo, default, set, 0);
}

/// The person tries settings the commands do not take: an unknown mode, level or text, `batch`
/// without a window or with a window of no whole positive count of seconds, minutes or hours, a
/// text setting for `own`, and a channel the server issued that `clienta`'s revisions do not
/// hold. Each fails and says why, and the listing gives both channels as no one set them.
#[test]
fn unknown_or_incomplete_settings_are_refused_and_change_nothing() {
    let solo = Solo::new(5_207_104_001, "ada-tries-wrong-delivery-modes");
    let id = solo.default_channel().to_string();
    let (cloud, ada) = (solo.talk.cloud(), &solo.talk.ada);
    let unknown = issue_channel(cloud, &ada.token, solo.talk.space.id).to_string();

    for args in [
        vec![SET_MODE, id.as_str(), PEER, "soon"],
        vec![SET_MODE, id.as_str(), "friend", NOW],
        vec![SET_MODE, id.as_str(), PEER, BATCH],
        vec![SET_MODE, id.as_str(), PEER, BATCH, WINDOW, "0s"],
        vec![SET_MODE, id.as_str(), PEER, BATCH, WINDOW, "soon"],
        vec![SET_MODE, unknown.as_str(), PEER, PULL],
        vec![SET_TEXT, id.as_str(), PEER, "verbatim"],
        vec![SET_TEXT, id.as_str(), OWN, INLINE],
        vec![SET_TEXT, unknown.as_str(), PEER, INLINE],
    ] {
        let tried = solo.person_runs(&args);
        assert!(!tried.success, "{args:?}: {tried:?}");
        assert!(!tried.shown().trim().is_empty(), "says nothing: {tried:?}");
    }

    for channel in [solo.default_channel(), other_channel(&solo)] {
        assert_listed(&solo, channel, DEFAULTS, 0);
    }
}
