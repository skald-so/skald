//! `skald set-delivery-mode`, `skald set-delivery-text` and `skald delivery-modes`: how a message
//! reaches an agent's session by the paths that push it in, the person's setting per channel of the
//! working directory's space and sender trust level, kept in the local state
//! (`docs/protocol/delivery.md`, "Device to agent"). An agent cannot run the commands
//! ([`crate::agents::person_only`]).
//!
//! - The mode, `now` by default: `now` pushes each message as it comes; `batch` keeps the level's
//!   messages of the channel until its window, opened by the first of them that waits, ends, then
//!   pushes them in one handover; `pull` pushes none, so only `inbox` and the agent's `skald recv`
//!   hand them over ([`Settings::plan`]).
//! - The text, for `peer` and `service`, `signal` by default: `inline` pushes such a message's text
//!   as data from its sender, after the warning line, in place of the signal.
//! - Each handover a path pushes into a session counts one wake for each channel whose messages,
//!   or whose signal, it carries; the listing shows them per channel.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;
use skald_proto::{ChannelId, TrustLevel};

use crate::connection::printable;
use crate::devices::text;
use crate::folders;
use crate::state::{self, State};

/// A sender trust level, as the person names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
pub enum Level {
    Own,
    Peer,
    Service,
}

impl Level {
    const ALL: [Level; 3] = [Level::Own, Level::Peer, Level::Service];

    fn name(self) -> &'static str {
        self.trust().name()
    }

    fn trust(self) -> TrustLevel {
        match self {
            Level::Own => TrustLevel::Own,
            Level::Peer => TrustLevel::Peer,
            Level::Service => TrustLevel::Service,
        }
    }
}

/// A delivery mode.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
pub enum Mode {
    /// Push each message as it comes.
    #[default]
    Now,
    /// Push the messages of each window together, once it ends.
    Batch,
    /// Push none: the agent takes them with inbox or skald recv.
    Pull,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Now => "now",
            Mode::Batch => "batch",
            Mode::Pull => "pull",
        }
    }
}

/// A text setting of `peer` and `service`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
pub enum Text {
    /// Push only a line that tells the agent to call inbox.
    #[default]
    Signal,
    /// Push the text, labelled as data from its sender.
    Inline,
}

impl Text {
    fn name(self) -> &'static str {
        match self {
            Text::Signal => "signal",
            Text::Inline => "inline",
        }
    }
}

/// `window`, a window of `batch` such as `30s`, `5m` or `2h`, in seconds.
pub fn window(window: &str) -> Result<u64, String> {
    let invalid = || {
        format!(
            "the window \"{}\": use whole seconds, minutes or hours, such as 30s, 5m or 2h",
            printable(window)
        )
    };
    let unit = match window.chars().last() {
        Some('s') => 1,
        Some('m') => 60,
        Some('h') => 60 * 60,
        _ => return Err(invalid()),
    };
    let count = &window[..window.len() - 1];
    if count.is_empty() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    (count.parse::<u64>().ok())
        .filter(|count| *count > 0)
        .and_then(|count| count.checked_mul(unit))
        .ok_or_else(invalid)
}

/// The setting of a channel and level.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    pub mode: Mode,
    /// The window of `batch`, in seconds.
    pub window: Option<u64>,
    pub text: Text,
}

/// The person's settings, by channel and level; the default for any other.
#[derive(Debug, Default)]
pub struct Settings(BTreeMap<(String, TrustLevel), Setting>);

/// A kept message that waits for an agent, as a push path takes it.
#[derive(Debug, Clone)]
pub struct Waiting {
    pub seq: u64,
    /// Its channel, if what the device kept of it says.
    pub channel: Option<String>,
    pub level: TrustLevel,
    /// When the device kept it, in Unix milliseconds.
    pub came: u64,
}

/// What a push path does with the messages that wait.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The messages it pushes now, by `seq`.
    pub goes: BTreeSet<u64>,
    /// When the first window that holds messages back ends, in Unix milliseconds.
    pub due: Option<u64>,
}

impl Settings {
    /// The settings in `state`.
    pub fn load(state: &State) -> Result<Self, String> {
        let mut settings = BTreeMap::new();
        for kept in state.delivery_settings().map_err(text)? {
            let Ok(level) = kept.level.parse::<TrustLevel>() else {
                continue;
            };
            let setting = Setting {
                mode: match kept.mode.as_str() {
                    "batch" => Mode::Batch,
                    "pull" => Mode::Pull,
                    _ => Mode::Now,
                },
                window: kept.window,
                text: match kept.text.as_str() {
                    "inline" if level < TrustLevel::Own => Text::Inline,
                    _ => Text::Signal,
                },
            };
            settings.insert((kept.channel, level), setting);
        }
        Ok(Settings(settings))
    }

    /// The setting of `level` in `channel`.
    pub fn of(&self, channel: Option<&str>, level: TrustLevel) -> Setting {
        (channel.and_then(|channel| self.0.get(&(channel.to_owned(), level))))
            .copied()
            .unwrap_or_default()
    }

    /// Whether a message of `level` in `channel` goes in as its text: `own`, or `inline`.
    pub fn as_text(&self, channel: Option<&str>, level: TrustLevel) -> bool {
        level >= TrustLevel::Own || self.of(channel, level).text == Text::Inline
    }

    /// Which of `waiting` a push path pushes at `now`, in Unix milliseconds: under `now`, each;
    /// under `batch`, all of a channel and level once the window that the first of them opened
    /// ended; under `pull`, none.
    pub fn plan(&self, waiting: &[Waiting], now: u64) -> Plan {
        let mut plan = Plan::default();
        let mut opened: BTreeMap<(Option<&str>, TrustLevel), u64> = BTreeMap::new();
        for message in waiting {
            let key = (message.channel.as_deref(), message.level);
            let setting = self.of(key.0, key.1);
            match (setting.mode, setting.window) {
                (Mode::Pull, _) => {}
                (Mode::Batch, Some(_)) => {
                    let first = opened.entry(key).or_insert(message.came);
                    *first = (*first).min(message.came);
                }
                _ => {
                    plan.goes.insert(message.seq);
                }
            }
        }
        for message in waiting {
            let channel = message.channel.as_deref();
            let Some(first) = opened.get(&(channel, message.level)) else {
                continue;
            };
            let window = self.of(channel, message.level).window.unwrap_or(0);
            let ends = first.saturating_add(window.saturating_mul(1_000));
            if ends <= now {
                plan.goes.insert(message.seq);
            } else {
                plan.due = Some(plan.due.map_or(ends, |due| due.min(ends)));
            }
        }
        plan
    }
}

/// `skald set-delivery-mode`: sets the mode of `level` in `channel`, a channel of the working
/// directory's space, to `mode`, with `window` for `batch`, by the state in `dir`.
pub fn set_mode(
    dir: &Path,
    channel: &str,
    level: Level,
    (mode, window): (Mode, Option<u64>),
) -> Result<(), String> {
    match (mode, window) {
        (Mode::Batch, None) => {
            return Err("batch needs --window, such as --window 5m; nothing changed".to_owned());
        }
        (Mode::Now | Mode::Pull, Some(_)) => {
            return Err("only batch takes --window; nothing changed".to_owned());
        }
        _ => {}
    }
    let (state, channel) = in_space(dir, channel)?;
    (state.set_delivery_mode(channel, level.name(), mode.name(), window)).map_err(text)?;
    let window = window.map_or_else(String::new, |window| format!(", window {window}s"));
    println!("{channel} {}: {}{window}.", level.name(), mode.name());
    Ok(())
}

/// `skald set-delivery-text`: sets the text setting of `level`, `peer` or `service`, in `channel`,
/// a channel of the working directory's space, to `text`, by the state in `dir`.
pub fn set_text(dir: &Path, channel: &str, level: Level, text_setting: Text) -> Result<(), String> {
    if level == Level::Own {
        return Err(
            "own messages always go in as text: set peer or service; nothing changed".to_owned(),
        );
    }
    let (state, channel) = in_space(dir, channel)?;
    (state.set_delivery_text(channel, level.name(), text_setting.name())).map_err(text)?;
    println!("{channel} {}: {}.", level.name(), text_setting.name());
    Ok(())
}

/// `skald delivery-modes`: lists, for each channel of the working directory's space this device
/// is in, the setting of each level and the channel's wakes, by the state in `dir`; as JSON, one
/// object per channel, if `json`.
pub fn list(dir: &Path, json: bool) -> Result<(), String> {
    let space = folders::working_space(dir)?;
    let state = State::open(dir).map_err(text)?;
    let settings = Settings::load(&state)?;
    let wakes = state.wakes().map_err(text)?;
    for channel in channels(&state, &space)? {
        let id = channel.to_string();
        let woken = wakes.get(&id).copied().unwrap_or(0);
        let setting = |level: Level| settings.of(Some(&id), level.trust());
        if json {
            let mut object = json!({ "channel": id, "wakes": woken });
            for level in Level::ALL {
                let setting = setting(level);
                let mut value = json!({ "mode": setting.mode.name(), "window": setting.window });
                if level != Level::Own {
                    value["text"] = json!(setting.text.name());
                }
                object[level.name()] = value;
            }
            println!("{object}");
            continue;
        }
        for level in Level::ALL {
            let setting = setting(level);
            let window = (setting.window.filter(|_| setting.mode == Mode::Batch))
                .map_or_else(String::new, |window| format!(" {window}s"));
            let text = match level {
                Level::Own => String::new(),
                _ => format!(", {}", setting.text.name()),
            };
            println!(
                "{id} {}: {}{window}{text}",
                level.name(),
                setting.mode.name()
            );
        }
        println!("{id} wakes: {woken}");
    }
    Ok(())
}

/// The state in `dir` and `given`, a channel of the working directory's space this device is in.
fn in_space(dir: &Path, given: &str) -> Result<(State, ChannelId), String> {
    let space = folders::working_space(dir)?;
    let state = State::open(dir).map_err(text)?;
    let unknown = || {
        format!(
            "{} is no channel of the space {} this device is in: see skald channels {}; nothing \
             changed",
            printable(given),
            space.name,
            space.name
        )
    };
    let channel: ChannelId = given.trim().parse().map_err(|_| unknown())?;
    if !channels(&state, &space)?.contains(&channel) {
        return Err(unknown());
    }
    Ok((state, channel))
}

/// The channels of `space`, as its last accepted revision gives them, whose members hold the
/// account of the device of `state`.
fn channels(state: &State, space: &state::Space) -> Result<Vec<ChannelId>, String> {
    let roster = crate::follow::decode(&space.accepted, &space.name)?;
    let account = (state.device().map_err(text)?).and_then(|device| device.account);
    Ok((roster.body().channels.iter())
        .filter(|channel| {
            (channel.members.iter())
                .any(|member| account.as_deref() == Some(member.account.to_string().as_str()))
        })
        .map(|channel| channel.id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = "channel-a";
    const OTHER: &str = "channel-b";

    fn settings(set: &[(&str, TrustLevel, Setting)]) -> Settings {
        Settings(
            (set.iter())
                .map(|(channel, level, setting)| (((*channel).to_owned(), *level), *setting))
                .collect(),
        )
    }

    fn waiting(seq: u64, channel: &str, level: TrustLevel, came: u64) -> Waiting {
        Waiting {
            seq,
            channel: Some(channel.to_owned()),
            level,
            came,
        }
    }

    fn batch(window: u64) -> Setting {
        Setting {
            mode: Mode::Batch,
            window: Some(window),
            text: Text::Signal,
        }
    }

    /// A window is a whole positive count of seconds, minutes or hours.
    #[test]
    fn window_takes_seconds_minutes_or_hours() {
        assert_eq!(window("30s"), Ok(30));
        assert_eq!(window("5m"), Ok(300));
        assert_eq!(window("2h"), Ok(7_200));
        for wrong in [
            "0s",
            "soon",
            "5",
            "m",
            "-5m",
            "1.5h",
            "5 m",
            "99999999999999999999h",
        ] {
            assert!(window(wrong).is_err(), "{wrong}");
        }
    }

    /// With no setting, every message goes at once, and nothing is due.
    #[test]
    fn with_no_setting_each_goes_now() {
        let waiting = [
            waiting(1, DEFAULT, TrustLevel::Own, 5),
            waiting(2, OTHER, TrustLevel::Peer, 6),
            Waiting {
                seq: 3,
                channel: None,
                level: TrustLevel::Service,
                came: 0,
            },
        ];
        let plan = Settings::default().plan(&waiting, 10);
        assert_eq!(plan.goes, BTreeSet::from([1, 2, 3]));
        assert_eq!(plan.due, None);
    }

    /// Under `batch`, a channel and level's messages wait until the window that the first opened
    /// ends, then go together; another level and channel go as theirs say.
    #[test]
    fn batch_holds_a_window_s_messages_until_it_ends() {
        let set = settings(&[(DEFAULT, TrustLevel::Own, batch(10))]);
        let waiting = [
            waiting(1, DEFAULT, TrustLevel::Own, 2_000),
            waiting(2, DEFAULT, TrustLevel::Own, 9_000),
            waiting(3, DEFAULT, TrustLevel::Peer, 3_000),
            waiting(4, OTHER, TrustLevel::Own, 3_000),
        ];
        let within = set.plan(&waiting, 11_999);
        assert_eq!(within.goes, BTreeSet::from([3, 4]));
        assert_eq!(within.due, Some(12_000));
        let after = set.plan(&waiting, 12_000);
        assert_eq!(after.goes, BTreeSet::from([1, 2, 3, 4]));
        assert_eq!(after.due, None);
    }

    /// Under `pull`, the level's messages of the channel never go, and nothing is due for them.
    #[test]
    fn pull_pushes_none() {
        let pull = Setting {
            mode: Mode::Pull,
            ..Setting::default()
        };
        let set = settings(&[(OTHER, TrustLevel::Peer, pull)]);
        let waiting = [
            waiting(1, OTHER, TrustLevel::Peer, 0),
            waiting(2, OTHER, TrustLevel::Own, 0),
            waiting(3, DEFAULT, TrustLevel::Peer, 0),
        ];
        let plan = set.plan(&waiting, u64::MAX);
        assert_eq!(plan.goes, BTreeSet::from([2, 3]));
        assert_eq!(plan.due, None);
    }

    /// `own` goes in as text; `peer` and `service` only under `inline`, in its channel.
    #[test]
    fn text_goes_in_for_own_and_inline() {
        let inline = Setting {
            text: Text::Inline,
            ..Setting::default()
        };
        let set = settings(&[(DEFAULT, TrustLevel::Peer, inline)]);
        assert!(set.as_text(Some(DEFAULT), TrustLevel::Own));
        assert!(set.as_text(Some(DEFAULT), TrustLevel::Peer));
        assert!(!set.as_text(Some(DEFAULT), TrustLevel::Service));
        assert!(!set.as_text(Some(OTHER), TrustLevel::Peer));
        assert!(!set.as_text(None, TrustLevel::Peer));
    }
}
