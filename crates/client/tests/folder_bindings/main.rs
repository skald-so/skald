//! Binding a folder to a space: a folder is in the space a `.skald.toml` in it or in a folder above
//! it names, or a rule in the local state gives for a folder and everything below it, such as
//! `~/work/clientA/** → clientA`. The nearest folder with a binding decides; at the same folder the
//! rule wins. A `.skald.toml` binds only once the device's user approved it for its folder, and
//! again after each change; an unapproved one is skipped with a warning. A folder without a binding
//! is in the personal space the device pinned, never one the server marks; without one, in no
//! space. A binding to a space the device is not in leaves the folder in no space. Folders compare
//! as canonical paths.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, which rewrites the server's answers, as a compromised server would; a test
//! plays the person on the verification page in headless Chrome. Each machine keeps its state in a
//! directory of its own and its folders in its home there. The device is in three spaces: its
//! personal space, named its account's username, `clienta` and `clientb`.
//!
//! The CLI, as these tests use it, beside `skald create-space <name>` and `skald spaces`:
//! - `skald add-rule <folder> <space>`: binds `<folder>` and everything below it to the space named
//!   `<space>`, also one the device is not in;
//! - `skald rules`: one line per rule, holding its folder's canonical path and its space's name;
//! - `skald remove-rule <folder>`: removes the rule of `<folder>`;
//! - `skald approve-binding <folder>`: approves the `.skald.toml` in `<folder>` as it is now, also
//!   one naming a space the device is not in. The file is TOML: `space = "<name>"`;
//! - `skald folder [<folder>]`: shows the space of `<folder>`, by default of the working directory,
//!   and what bound it: a line holding the word `rule` and the rule's folder; a line holding
//!   `.skald.toml` and the file's folder; or, for the personal space, the word `personal`. Folders
//!   show as canonical paths. A folder in no space shows a line saying `no space` and names none
//!   of the device's spaces; when a binding names a space the device is not in, it also names that
//!   space and what bound it;
//! - a skipped `.skald.toml` is warned of by one line saying `warning`, on either output, holding
//!   the file's folder and `approve-binding`.
//!
//! The personal space is the account's, with `POST /account/personal-space` and the `personal`
//! mark the server gives in the account's spaces.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../roster_revisions/world.rs"]
mod world;

#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/relay.rs"]
mod relay;

#[path = "../sign_in/machine.rs"]
mod machine;

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;

use std::fs;
use std::path::{Path, PathBuf};

use machine::{FIRST_DEVICE, Finished, Machine, NO};
use serde_json::{Value, json};
use skald_proto::SpaceId;
use world::{Person, SPACES, World, create, holds_word, plain_lines, run, warnings, words};

/// The commands that add, list and remove rules, approve a `.skald.toml` and show a folder's space.
const ADD_RULE: &str = "add-rule";
const RULES: &str = "rules";
const REMOVE_RULE: &str = "remove-rule";
const APPROVE: &str = "approve-binding";
const FOLDER: &str = "folder";

/// The file that binds its folder, and what `skald folder` says of each binding and of none.
const CONFIG: &str = ".skald.toml";
const RULE: &str = "rule";
const PERSONAL: &str = "personal";
const NO_SPACE: &str = "no space";

/// The team spaces the device is in.
const CLIENT_A: &str = "clienta";
const CLIENT_B: &str = "clientb";

/// The requests for the account's personal space and for its spaces, as their lines start.
const PERSONAL_REQUEST: &str = "POST /account/personal-space ";
const LIST_REQUEST: &str = "GET /spaces ";

/// A person's first machine, in its personal space, `clienta` and `clientb`.
struct Desk {
    world: World,
    ada: Person,
    /// The name of the account's personal space: its username.
    personal: String,
    client_a: SpaceId,
}

impl Desk {
    fn start(id: u64, login: &str) -> Self {
        let world = World::start();
        let ada = world.sign_up(id, login, "ada-workstation");
        let client_a = create(&world.cloud, &ada, CLIENT_A).body().space;
        create(&world.cloud, &ada, CLIENT_B);
        Desk {
            personal: ada.handle.to_string(),
            world,
            ada,
            client_a,
        }
    }

    /// The machine's folder `rel`, such as `work/clientA`, under its home; made if missing.
    fn dir(&self, rel: &str) -> PathBuf {
        dir_of(&self.ada.machine, rel)
    }

    /// The device's spaces.
    fn spaces(&self) -> [&str; 3] {
        [self.personal.as_str(), CLIENT_A, CLIENT_B]
    }

    /// Runs `skald folder` on the machine in `dir`.
    fn folder(&self, dir: &Path) -> Finished {
        folder(&self.ada.machine, dir)
    }

    /// Fails unless `skald folder` in `dir` shows the space `space`, bound as `by`, and no
    /// `.skald.toml` was skipped.
    fn assert_in(&self, dir: &Path, space: &str, by: By) {
        let shown = self.folder(dir);
        assert_in(&shown, space, by, &self.spaces());
        assert_unwarned(&shown);
    }

    /// Fails unless `skald folder` in `dir` shows the space `space`, bound as `by`, and warns that
    /// the `.skald.toml` in the folder `rel` was skipped.
    fn assert_skipped(&self, dir: &Path, space: &str, by: By, rel: &str) {
        let shown = self.folder(dir);
        assert_in(&shown, space, by, &self.spaces());
        assert_warned(&shown, rel);
    }

    /// Runs `skald <args>` on the machine; fails unless it succeeds.
    fn ok(&self, args: &[&str]) -> Finished {
        let finished = run(&self.ada.machine, args);
        assert!(finished.success, "{args:?}: {finished:?}");
        finished
    }

    fn add_rule(&self, dir: &Path, space: &str) {
        self.ok(&[ADD_RULE, path(dir), space]);
    }

    fn approve(&self, dir: &Path) {
        self.ok(&[APPROVE, path(dir)]);
    }
}

/// What bound a folder: a rule or a `.skald.toml` at the folder `rel` under the home, or nothing,
/// leaving it in the personal space.
#[derive(Clone, Copy, Debug)]
enum By<'a> {
    Rule(&'a str),
    Config(&'a str),
    Personal,
}

/// `machine`'s folder `rel` under its home; made if missing.
fn dir_of(machine: &Machine, rel: &str) -> PathBuf {
    let dir = machine.root().join("home").join(tail(rel));
    fs::create_dir_all(&dir).expect("make a folder");
    dir
}

/// `rel`, such as `work/clientA`, as the end of a path on this OS.
fn tail(rel: &str) -> String {
    rel.split('/').collect::<PathBuf>().display().to_string()
}

fn path(dir: &Path) -> &str {
    dir.to_str().expect("a UTF-8 path")
}

/// Runs `skald folder` on `machine` in the working directory `dir`.
fn folder(machine: &Machine, dir: &Path) -> Finished {
    machine.start_in(dir, &[FOLDER]).finish()
}

/// Writes a `.skald.toml` naming `space` to `dir`.
fn write_config(dir: &Path, space: &str) {
    fs::write(dir.join(CONFIG), format!("space = \"{space}\"\n")).expect("write a .skald.toml");
}

/// Fails unless the lines `shown` shows outside its warnings say the folder was bound as `by`, and
/// by nothing else.
fn assert_bound_by(shown: &Finished, by: By) {
    let lines = plain_lines(shown);
    let by_rule = |line: &String| words(line).contains(&RULE);
    let by_config = |line: &String| line.contains(CONFIG);
    match by {
        By::Rule(rel) => {
            assert!(
                lines
                    .iter()
                    .any(|line| by_rule(line) && line.contains(&tail(rel))),
                "not bound by the rule of {rel}: {shown:?}"
            );
            assert!(!lines.iter().any(by_config), "{shown:?}");
        }
        By::Config(rel) => {
            assert!(
                lines
                    .iter()
                    .any(|line| by_config(line) && line.contains(&tail(rel))),
                "not bound by the {CONFIG} of {rel}: {shown:?}"
            );
            assert!(!lines.iter().any(by_rule), "{shown:?}");
        }
        By::Personal => {
            assert!(
                holds_word(&lines, PERSONAL),
                "not the personal space: {shown:?}"
            );
            assert!(
                !lines.iter().any(|line| by_rule(line) || by_config(line)),
                "{shown:?}"
            );
        }
    }
}

/// Fails unless `shown` succeeded and shows the folder in `space`, bound as `by`, and in none of
/// the others of `spaces`.
fn assert_in(shown: &Finished, space: &str, by: By, spaces: &[&str]) {
    assert!(shown.success, "{shown:?}");
    let lines = plain_lines(shown);
    assert!(holds_word(&lines, space), "not in {space}: {shown:?}");
    for other in spaces.iter().filter(|other| **other != space) {
        assert!(!holds_word(&lines, other), "also {other}: {shown:?}");
    }
    assert!(
        !lines
            .iter()
            .any(|line| line.to_lowercase().contains(NO_SPACE)),
        "in no space: {shown:?}"
    );
    assert_bound_by(shown, by);
}

/// Fails unless `shown` shows the folder in no space, naming none of `spaces`; with `named`, it
/// names that space, which the device is not in, and what bound the folder to it.
fn assert_in_none(shown: &Finished, named: Option<(&str, By)>, spaces: &[&str]) {
    let lines = plain_lines(shown);
    assert!(
        lines
            .iter()
            .any(|line| line.to_lowercase().contains(NO_SPACE)),
        "not in no space: {shown:?}"
    );
    for space in spaces {
        assert!(!holds_word(&lines, space), "{space} is named: {shown:?}");
    }
    if let Some((space, by)) = named {
        assert!(holds_word(&lines, space), "{space} is not named: {shown:?}");
        assert_bound_by(shown, by);
    }
}

/// Fails unless `shown` warns that the `.skald.toml` in the folder `rel` was skipped, and how to
/// approve it.
fn assert_warned(shown: &Finished, rel: &str) {
    let folder = tail(rel).to_lowercase();
    assert!(
        warnings(shown)
            .iter()
            .any(|line| line.contains(APPROVE) && line.contains(&folder)),
        "no warning of the {CONFIG} of {rel}: {shown:?}"
    );
}

/// Fails if `shown` warns of a skipped `.skald.toml`.
fn assert_unwarned(shown: &Finished) {
    assert!(
        !warnings(shown)
            .iter()
            .any(|line| line.contains(APPROVE) || line.contains(CONFIG)),
        "{shown:?}"
    );
}

/// Rules for `~/work/clientA` and `~/work/clientB` bind them and their subfolders, not a sibling
/// whose name starts the same, and `skald rules` lists both. Once the rule of `~/work/clientB` is
/// removed, its folders are in the personal space.
#[test]
fn rules_bind_their_folders_and_everything_below() {
    let desk = Desk::start(4_120_801_001, "ada-rules-standin");
    let a = desk.dir("work/clientA");
    let b = desk.dir("work/clientB");
    desk.add_rule(&a, CLIENT_A);
    desk.add_rule(&b, CLIENT_B);

    desk.assert_in(&a, CLIENT_A, By::Rule("work/clientA"));
    let deep = desk.dir("work/clientA/app/src");
    desk.assert_in(&deep, CLIENT_A, By::Rule("work/clientA"));
    desk.assert_in(&b, CLIENT_B, By::Rule("work/clientB"));
    let docs = desk.dir("work/clientB/docs");
    desk.assert_in(&docs, CLIENT_B, By::Rule("work/clientB"));
    let sibling = desk.dir("work/clientA-old");
    desk.assert_in(&sibling, &desk.personal, By::Personal);

    let listed = desk.ok(&[RULES]);
    let lines = plain_lines(&listed);
    for (rel, space) in [("work/clientA", CLIENT_A), ("work/clientB", CLIENT_B)] {
        assert!(
            lines
                .iter()
                .any(|line| line.contains(&tail(rel)) && words(line).contains(&space)),
            "no rule {rel} → {space}: {listed:?}"
        );
    }

    desk.ok(&[REMOVE_RULE, path(&b)]);

    let listed = desk.ok(&[RULES]);
    assert!(
        !plain_lines(&listed)
            .iter()
            .any(|line| line.contains(&tail("work/clientB"))),
        "the removed rule is listed: {listed:?}"
    );
    desk.assert_in(&docs, &desk.personal, By::Personal);
    desk.assert_in(&deep, CLIENT_A, By::Rule("work/clientA"));
}

/// Under the rule of `~/work/clientA`, a `.skald.toml` naming `clientb` in a deeper folder is
/// skipped with a warning until approved; approved, it binds its folder and those below it. A rule
/// deeper still wins over it.
#[test]
fn approved_skald_toml_in_a_deeper_folder_wins_over_the_rule_above() {
    let desk = Desk::start(4_120_801_101, "ada-deeper-standin");
    let a = desk.dir("work/clientA");
    desk.add_rule(&a, CLIENT_A);
    let shared = desk.dir("work/clientA/shared");
    write_config(&shared, CLIENT_B);
    let src = desk.dir("work/clientA/shared/src");

    desk.assert_skipped(
        &src,
        CLIENT_A,
        By::Rule("work/clientA"),
        "work/clientA/shared",
    );

    desk.approve(&shared);

    desk.assert_in(&src, CLIENT_B, By::Config("work/clientA/shared"));
    desk.assert_in(&shared, CLIENT_B, By::Config("work/clientA/shared"));
    desk.assert_in(&a, CLIENT_A, By::Rule("work/clientA"));

    let vendor = desk.dir("work/clientA/shared/vendor");
    desk.add_rule(&vendor, CLIENT_A);
    let lib = desk.dir("work/clientA/shared/vendor/lib");
    desk.assert_in(&lib, CLIENT_A, By::Rule("work/clientA/shared/vendor"));
    desk.assert_in(&src, CLIENT_B, By::Config("work/clientA/shared"));
}

/// An approved `.skald.toml` naming `clientb` in `~/work/clientA`, whose rule binds it to
/// `clienta`: the rule wins; once it is removed, the file binds.
#[test]
fn rule_wins_over_skald_toml_at_the_same_folder() {
    let desk = Desk::start(4_120_801_201, "ada-same-standin");
    let a = desk.dir("work/clientA");
    write_config(&a, CLIENT_B);
    desk.approve(&a);
    desk.add_rule(&a, CLIENT_A);
    let src = desk.dir("work/clientA/src");

    desk.assert_in(&a, CLIENT_A, By::Rule("work/clientA"));
    desk.assert_in(&src, CLIENT_A, By::Rule("work/clientA"));

    desk.ok(&[REMOVE_RULE, path(&a)]);

    desk.assert_in(&src, CLIENT_B, By::Config("work/clientA"));
}

/// An approved `.skald.toml` that changes, by a comment or by the space it names, binds nothing
/// and is warned of until approved again.
#[test]
fn changed_skald_toml_binds_nothing_until_approved_again() {
    let desk = Desk::start(4_120_801_301, "ada-changed-standin");
    let repo = desk.dir("work/repo");
    write_config(&repo, CLIENT_A);
    desk.approve(&repo);
    desk.assert_in(&repo, CLIENT_A, By::Config("work/repo"));

    let comment = format!("space = \"{CLIENT_A}\"\n# moved\n");
    let other = format!("space = \"{CLIENT_B}\"\n");
    for (changed, space) in [(comment, CLIENT_A), (other, CLIENT_B)] {
        fs::write(repo.join(CONFIG), changed).expect("change the .skald.toml");
        desk.assert_skipped(&repo, &desk.personal, By::Personal, "work/repo");

        desk.approve(&repo);
        desk.assert_in(&repo, space, By::Config("work/repo"));
    }
}

/// An unapproved `.skald.toml` binds nothing, with the warning; the approval of one holds only for
/// its own folder, not for the same file in another, as a cloned repository would bring it.
#[test]
fn skald_toml_is_approved_only_for_its_folder() {
    let desk = Desk::start(4_120_801_401, "ada-cloned-standin");
    let repo = desk.dir("work/repo");
    write_config(&repo, CLIENT_A);

    desk.assert_skipped(&repo, &desk.personal, By::Personal, "work/repo");

    desk.approve(&repo);
    desk.assert_in(&repo, CLIENT_A, By::Config("work/repo"));

    let b = desk.dir("work/clientB");
    desk.add_rule(&b, CLIENT_B);
    let clone = desk.dir("work/clientB/vendor");
    fs::copy(repo.join(CONFIG), clone.join(CONFIG)).expect("copy the .skald.toml");
    let by = By::Rule("work/clientB");
    desk.assert_skipped(&clone, CLIENT_B, by, "work/clientB/vendor");
}

/// A folder without a binding is in the personal space the device pinned, also when the proxy
/// marks `clienta` as personal in the account's spaces and gives `clienta`'s ID, or a made-up one,
/// as the personal space. The account's next machine, not linked, pinned none: its folder is in no
/// space.
#[test]
fn folder_without_a_binding_is_in_the_pinned_personal_space() {
    let desk = Desk::start(4_120_801_501, "ada-personal-standin");
    let cloud = &desk.world.cloud;
    let notes = desk.dir("notes");
    desk.assert_in(&notes, &desk.personal, By::Personal);

    let marked = desk.client_a.to_string();
    cloud.relay.rewrite(LIST_REQUEST, move |body| {
        let mut list: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        for space in list["spaces"].as_array_mut().into_iter().flatten() {
            space["personal"] = json!(space["space"] == json!(marked));
        }
        list.to_string()
    });
    for (other, name) in [(desk.client_a, CLIENT_A), (SpaceId::generate(), "globex")] {
        let answer = json!({ "space": other.to_string(), "name": name }).to_string();
        cloud
            .relay
            .rewrite(PERSONAL_REQUEST, move |_| answer.clone());
        // Its result does not matter: a run that asks the server for the account's spaces.
        run(&desk.ada.machine, &[SPACES]);

        desk.assert_in(&notes, &desk.personal, By::Personal);
    }

    let laptop = Machine::new().with_host_name("ada-second-laptop");
    let page = desk.world.browser.page();
    let waiting = cloud.signed_in(&page, &laptop, &desk.ada.user, &[FIRST_DEVICE, NO]);
    assert!(waiting.success, "the second machine: {waiting:?}");
    let shown = folder(&laptop, &dir_of(&laptop, "notes"));
    assert_in_none(&shown, None, &desk.spaces());
}

/// A binding to a space the device is not in, an approved `.skald.toml` naming a space nobody has
/// below the rule of `~/work/clientA`, or a rule for another account's space, leaves the folder in
/// no space; it never falls back to the rule above or to the personal space.
#[test]
fn binding_to_a_space_the_device_is_not_in_leaves_the_folder_in_none() {
    let desk = Desk::start(4_120_801_601, "ada-elsewhere-standin");
    let bob = desk
        .world
        .sign_up(4_120_801_602, "bob-elsewhere-standin", "bob-workstation");
    create(&desk.world.cloud, &bob, "hooli");

    let a = desk.dir("work/clientA");
    desk.add_rule(&a, CLIENT_A);
    let initech = desk.dir("work/clientA/initech");
    write_config(&initech, "initech");
    desk.approve(&initech);
    let src = desk.dir("work/clientA/initech/src");
    let shown = desk.folder(&src);
    let by = By::Config("work/clientA/initech");
    assert_in_none(&shown, Some(("initech", by)), &desk.spaces());
    assert_unwarned(&shown);

    let hooli = desk.dir("work/hooli");
    desk.add_rule(&hooli, "hooli");
    let app = desk.dir("work/hooli/app");
    let shown = desk.folder(&app);
    let by = By::Rule("work/hooli");
    assert_in_none(&shown, Some(("hooli", by)), &desk.spaces());

    desk.assert_in(&a, CLIENT_A, By::Rule("work/clientA"));
}

/// A symlink to `~/work/clientA` binds as its target, whether given to `skald folder` or as the
/// working directory; a rule added and a `.skald.toml` approved through a symlink bind the target,
/// and the rule lists the target.
#[cfg(unix)]
#[test]
fn symlink_binds_as_its_target() {
    use std::os::unix::fs::symlink;

    let desk = Desk::start(4_120_801_701, "ada-symlink-standin");
    let home = desk.ada.machine.root().join("home");
    let a = desk.dir("work/clientA");
    desk.add_rule(&a, CLIENT_A);
    let link_a = home.join("link-a");
    symlink(&a, &link_a).expect("link to clientA");

    let shown = run(&desk.ada.machine, &[FOLDER, path(&link_a)]);
    assert_in(&shown, CLIENT_A, By::Rule("work/clientA"), &desk.spaces());
    desk.assert_in(&link_a, CLIENT_A, By::Rule("work/clientA"));

    let b = desk.dir("work/clientB");
    let link_b = home.join("link-b");
    symlink(&b, &link_b).expect("link to clientB");
    desk.add_rule(&link_b, CLIENT_B);
    let docs = desk.dir("work/clientB/docs");
    desk.assert_in(&docs, CLIENT_B, By::Rule("work/clientB"));
    let listed = desk.ok(&[RULES]);
    let lines = plain_lines(&listed);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(&tail("work/clientB"))),
        "the rule does not list the target: {listed:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("link-b")),
        "the rule lists the symlink: {listed:?}"
    );

    let repo = desk.dir("work/repo");
    write_config(&repo, CLIENT_B);
    let link_repo = home.join("link-repo");
    symlink(&repo, &link_repo).expect("link to the repository");
    desk.approve(&link_repo);
    desk.assert_in(&repo, CLIENT_B, By::Config("work/repo"));
}
