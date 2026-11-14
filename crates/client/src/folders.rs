//! `skald add-rule`, `skald rules`, `skald remove-rule`, `skald approve-binding` and `skald folder`:
//! which space a folder is in (`docs/dev/architecture.md`, "Membership", "Folder binding"). It reads
//! only the local state: the device is in the spaces it follows. An approval holds the SHA-256 of
//! the file's bytes.

use std::fs;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use skald_proto::Name;

use crate::connection::printable;
use crate::devices::text;
use crate::plain_path::plain;
use crate::state::{self, State};

/// The file that binds its folder.
const CONFIG: &str = ".skald.toml";

/// The most bytes a `.skald.toml` may hold.
const CONFIG_LIMIT: u64 = 64 * 1024;

/// What a run says of a binding to a space the device is not in.
const NOT_IN: &str = "This device is not in that space, so they are in no space until it is.";

/// What an agent's `skald folder` says of a folder outside its space, alike for every such folder.
const OTHER_FOLDER: &str =
    "as an agent, skald shows only the folders in the space of the agent's folder";

/// A `.skald.toml`.
#[derive(Deserialize)]
struct Config {
    space: String,
}

/// What binds a folder: the rule of a folder, or the approved `.skald.toml` in one.
enum By {
    Rule(PathBuf),
    Config(PathBuf),
}

/// A folder's space.
enum Bound {
    /// `by` binds the folder to the space named `name`: `space`, the device's space of that name,
    /// or none if the device is not in one.
    By {
        by: By,
        name: String,
        space: Option<state::Space>,
    },
    /// Nothing binds the folder: it is in the device's personal space, if the device pinned one.
    Unbound(Option<state::Space>),
}

/// `skald add-rule`: binds `folder` and everything below it to the space named `space`, in the
/// state in `dir`.
pub fn add_rule(dir: &Path, folder: &Path, space: &str) -> Result<(), String> {
    let name: Name = space.parse().map_err(|err| {
        format!(
            "the space name \"{}\" {err}; no rule is added",
            printable(space)
        )
    })?;
    let folder = canonical(folder)?;
    let state = writable(dir)?;
    let earlier = state.set_rule(key(&folder)?, name.as_str()).map_err(text)?;
    let mut said = format!(
        "Rule: {} and everything below it are in the space {}.",
        shown(&folder),
        name.as_str()
    );
    if let Some(earlier) = earlier.filter(|earlier| earlier != name.as_str()) {
        said.push_str(&format!(" It replaces the rule for the space {earlier}."));
    }
    if find(&state, name.as_str())?.is_none() {
        said.push_str(&format!(" {NOT_IN}"));
    }
    println!("{said}");
    Ok(())
}

/// `skald rules`: lists the rules of the state in `dir`.
pub fn rules(dir: &Path) -> Result<(), String> {
    let Some(state) = readable(dir)? else {
        println!("{}", no_rules());
        return Ok(());
    };
    let rules = state.rules().map_err(text)?;
    if rules.is_empty() {
        println!("{}", no_rules());
    }
    for (folder, space) in rules {
        let mut line = format!("{}  {space}", shown(Path::new(&folder)));
        if find(&state, &space)?.is_none() {
            line.push_str("  (this device is not in it)");
        }
        println!("{line}");
    }
    Ok(())
}

fn no_rules() -> &'static str {
    "No rules; add one with skald add-rule <folder> <space>."
}

/// `skald remove-rule`: removes the rule of `folder` from the state in `dir`. A folder that no
/// longer exists is taken as its absolute path.
pub fn remove_rule(dir: &Path, folder: &Path) -> Result<(), String> {
    let folder = match canonical(folder) {
        Ok(folder) => folder,
        Err(err) => std::path::absolute(folder).map_err(|_| err)?,
    };
    let no_rule = || format!("no rule for {}", shown(&folder));
    let Some(state) = readable(dir)? else {
        return Err(no_rule());
    };
    let stored = state
        .rules()
        .map_err(text)?
        .into_iter()
        .map(|(stored, _)| stored)
        .find(|stored| plain(Path::new(stored)) == plain(&folder));
    match stored {
        Some(stored) if state.remove_rule(&stored).map_err(text)? => {
            println!("Removed the rule for {}.", shown(&folder));
            Ok(())
        }
        _ => Err(no_rule()),
    }
}

/// `skald approve-binding`: approves the `.skald.toml` in `folder` as its bytes are now, in the
/// state in `dir`.
pub fn approve(dir: &Path, folder: &Path) -> Result<(), String> {
    let folder = canonical(folder)?;
    let file = folder.join(CONFIG);
    let bytes = read_config(&file).map_err(|err| {
        format!(
            "cannot read the {CONFIG} in {}: {err}; nothing is approved",
            shown(&folder)
        )
    })?;
    let name = config_space(&bytes).map_err(|err| {
        format!(
            "the {CONFIG} in {} {err}; nothing is approved",
            shown(&folder)
        )
    })?;
    let state = writable(dir)?;
    state
        .approve_config(key(&folder)?, &Sha256::digest(&bytes).into())
        .map_err(text)?;
    let mut said = format!(
        "Approved the {CONFIG} in {} as it is now: it binds the folder and everything below it to \
         the space {}.",
        shown(&folder),
        name.as_str()
    );
    if find(&state, name.as_str())?.is_none() {
        said.push_str(&format!(" {NOT_IN}"));
    }
    if state
        .rules()
        .map_err(text)?
        .iter()
        .any(|(rule, _)| Path::new(rule) == folder)
    {
        said.push_str(" The folder's rule wins over it.");
    }
    println!("{said}");
    Ok(())
}

/// `skald folder`: shows the space of `folder`, by default the working directory, and what bound
/// it, by the state in `dir`; warns of each `.skald.toml` it skipped. As an `agent`, it shows
/// another folder than the working directory only in the working directory's space.
pub fn show(dir: &Path, folder: Option<&Path>, agent: bool) -> Result<(), String> {
    let folder = match folder {
        Some(folder) => canonical(folder)?,
        None => working_dir()?,
    };
    let state = readable(dir)?;
    let working = if agent {
        working_dir()?
    } else {
        folder.clone()
    };
    if folder != working {
        let (own, _) = bound(state.as_ref(), &working)?;
        let (there, _) = bound(state.as_ref(), &folder)?;
        if own
            .space()
            .is_none_or(|own| there.space().is_none_or(|there| there.id != own.id))
        {
            return Err(OTHER_FOLDER.to_owned());
        }
    }
    let bound = warned(state.as_ref(), &folder)?;
    println!("Folder: {}", shown(&folder));
    match bound {
        Bound::By { by, name, space } => {
            match space {
                Some(space) => println!("Space: {}", space_line(&space)),
                None => println!("Space: no space: this device is not in the space {name}"),
            }
            match by {
                By::Rule(rule) => println!("Bound by: the rule for {}", shown(&rule)),
                By::Config(config) => println!("Bound by: the {CONFIG} in {}", shown(&config)),
            }
        }
        Bound::Unbound(Some(personal)) => {
            println!("Space: {}", space_line(&personal));
            println!("Bound by: nothing, so it is this device's personal space");
        }
        Bound::Unbound(None) => println!(
            "Space: no space: nothing binds the folder, and this device pinned no personal space"
        ),
    }
    Ok(())
}

impl Bound {
    /// The device's space the folder is in, if any.
    fn space(&self) -> Option<&state::Space> {
        match self {
            Bound::By { space, .. } | Bound::Unbound(space) => space.as_ref(),
        }
    }
}

/// The space of the working directory, by the state in `dir`; warns of each `.skald.toml` it
/// skipped. Fails, saying why, when the working directory is in no space.
pub fn working_space(dir: &Path) -> Result<state::Space, String> {
    let folder = working_dir()?;
    let state = readable(dir)?;
    let in_none = |why: String| format!("{} is in no space: {why}", shown(&folder));
    match warned(state.as_ref(), &folder)? {
        Bound::By {
            space: Some(space), ..
        }
        | Bound::Unbound(Some(space)) => Ok(space),
        Bound::By { name, .. } => Err(in_none(format!("this device is not in the space {name}"))),
        Bound::Unbound(None) => Err(in_none(
            "nothing binds it, and this device pinned no personal space".to_owned(),
        )),
    }
}

/// The working directory's canonical path.
fn working_dir() -> Result<PathBuf, String> {
    let folder = std::env::current_dir()
        .map_err(|err| format!("cannot read the working directory: {err}"))?;
    canonical(&folder)
}

/// The space of `folder`, a canonical path, as [`bound`] gives it; warns of each `.skald.toml`
/// skipped on the way.
fn warned(state: Option<&State>, folder: &Path) -> Result<Bound, String> {
    let (bound, warnings) = bound(state, folder)?;
    for warning in warnings {
        eprintln!("skald: warning: {warning}");
    }
    Ok(bound)
}

fn space_line(space: &state::Space) -> String {
    format!("{} ({})", space.name, space.id)
}

/// The space of `folder`, a canonical path, by the rules and approvals of `state`, and a warning
/// for each `.skald.toml` skipped on the way.
fn bound(state: Option<&State>, folder: &Path) -> Result<(Bound, Vec<String>), String> {
    let rules = match state {
        Some(state) => state.rules().map_err(text)?,
        None => Vec::new(),
    };
    let mut warnings = Vec::new();
    for at in folder.ancestors() {
        let at_key = at.to_str();
        let ruled = rules.iter().find(|(rule, _)| Some(rule.as_str()) == at_key);
        let (by, name) = if let Some((_, space)) = ruled {
            (By::Rule(at.to_owned()), space.clone())
        } else {
            let bytes = match read_config(&at.join(CONFIG)) {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warnings.push(format!(
                        "cannot read the {CONFIG} in {}, so it binds nothing: {err}",
                        shown(at)
                    ));
                    continue;
                }
            };
            let approved = match (state, at_key) {
                (Some(state), Some(at_key)) => state.approved_config(at_key).map_err(text)?,
                _ => None,
            };
            if approved != Some(Sha256::digest(&bytes).into()) {
                warnings.push(format!(
                    "the {CONFIG} in {} is not approved as it is now, so it binds nothing; \
                     approve it with skald approve-binding {}",
                    shown(at),
                    shown(at)
                ));
                continue;
            }
            let name = config_space(&bytes)
                .map_err(|err| format!("the approved {CONFIG} in {} {err}", shown(at)))?;
            (By::Config(at.to_owned()), name.as_str().to_owned())
        };
        let space = match state {
            Some(state) => find(state, &name)?,
            None => None,
        };
        return Ok((Bound::By { by, name, space }, warnings));
    }
    let personal = match state {
        Some(state) => state
            .spaces()
            .map_err(text)?
            .into_iter()
            .find(|space| space.personal),
        None => None,
    };
    Ok((Bound::Unbound(personal), warnings))
}

/// The bytes of `file`, a `.skald.toml`. Fails unless it is a regular file, also through a
/// symlink, of at most [`CONFIG_LIMIT`] bytes: it never opens a FIFO, which would block.
fn read_config(file: &Path) -> io::Result<Vec<u8>> {
    let irregular = || io::Error::other("it is not a regular file");
    if !fs::metadata(file)?.is_file() {
        return Err(irregular());
    }
    let opened = fs::File::open(file)?;
    if !opened.metadata()?.is_file() {
        return Err(irregular());
    }
    let mut bytes = Vec::new();
    opened.take(CONFIG_LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > CONFIG_LIMIT {
        return Err(io::Error::other(format!(
            "it holds more than {CONFIG_LIMIT} bytes"
        )));
    }
    Ok(bytes)
}

/// The space `bytes`, a `.skald.toml`, names.
fn config_space(bytes: &[u8]) -> Result<Name, String> {
    let config = std::str::from_utf8(bytes)
        .map_err(|err| err.to_string())
        .and_then(|toml| toml::from_str::<Config>(toml).map_err(|err| err.to_string()))
        .map_err(|err| {
            let lines: Vec<String> = err.trim_end().lines().map(printable).collect();
            format!(
                "does not name a space as space = \"<name>\": {}",
                lines.join("\n")
            )
        })?;
    config.space.parse().map_err(|err| {
        format!(
            "names the space \"{}\", whose name {err}",
            printable(&config.space)
        )
    })
}

/// The device's space named `name`, if it is in one.
fn find(state: &State, name: &str) -> Result<Option<state::Space>, String> {
    Ok(state
        .spaces()
        .map_err(text)?
        .into_iter()
        .find(|space| space.name == name))
}

/// `folder`'s canonical path; fails unless it is a folder.
fn canonical(folder: &Path) -> Result<PathBuf, String> {
    let canonical =
        fs::canonicalize(folder).map_err(|err| format!("the folder {}: {err}", shown(folder)))?;
    if !canonical.is_dir() {
        return Err(format!("{} is no folder", shown(&canonical)));
    }
    Ok(canonical)
}

/// `folder`, a canonical path, as the state keeps it.
fn key(folder: &Path) -> Result<&str, String> {
    folder.to_str().ok_or_else(|| {
        format!(
            "the path {} is not valid Unicode, so it binds nothing",
            shown(folder)
        )
    })
}

/// `path` as the CLI shows it.
fn shown(path: &Path) -> String {
    printable(&plain(path).display().to_string())
}

/// The state in `dir`, if it exists.
fn readable(dir: &Path) -> Result<Option<State>, String> {
    if !dir.is_dir() {
        return Ok(None);
    }
    State::open(dir).map(Some).map_err(text)
}

/// The state in `dir`, made if missing.
fn writable(dir: &Path) -> Result<State, String> {
    state::create_dir(dir)
        .map_err(|err| format!("cannot create the state directory {}: {err}", shown(dir)))?;
    State::open(dir).map_err(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    #[test]
    fn skald_toml_names_its_space_as_a_name() {
        let space = |toml: &str| config_space(toml.as_bytes()).map(|name| name.to_string());
        assert_eq!(space("space = \"acme\"\n").as_deref(), Ok("acme"));
        assert_eq!(
            space("# acme\nspace = 'acme'\nlater = 1\n").as_deref(),
            Ok("acme")
        );
        for wrong in ["", "space = acme", "space = 7", "[space]\nname = \"acme\""] {
            let err = space(wrong).expect_err(wrong);
            assert!(
                err.starts_with("does not name a space as space = \"<name>\": "),
                "{err}"
            );
        }
        let err = space("space = \"Acme\\u001b\"").expect_err("not a name");
        assert!(
            err.starts_with("names the space \"Acme\\u{1b}\", whose name "),
            "{err}"
        );
        let err = config_space(b"space = \"\xff\"").expect_err("not UTF-8");
        assert!(err.starts_with("does not name a space"), "{err}");
    }

    #[test]
    fn unreadable_skald_toml_is_skipped_with_a_warning() {
        let root = TestDir::new("folders-unreadable");
        let repo = root.join("repo");
        let src = repo.join("src");
        fs::create_dir_all(src.join(CONFIG)).unwrap();
        let (repo, src) = (canonical(&repo).unwrap(), canonical(&src).unwrap());
        let state = writable(&root.join("state")).unwrap();
        state.set_rule(key(&repo).unwrap(), "acme").unwrap();

        let (bound, warnings) = bound(Some(&state), &src).unwrap();
        assert!(
            matches!(bound, Bound::By { by: By::Rule(ref rule), .. } if *rule == repo),
            "the rule binds"
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].starts_with(&format!("cannot read the {CONFIG} in ")));
    }

    #[test]
    fn rule_of_a_removed_folder_is_removed_by_its_path() {
        let root = TestDir::new("folders-removed");
        let dir = root.join("state");
        let gone = root.join("gone");
        fs::create_dir(&gone).unwrap();
        add_rule(&dir, &gone, "acme").unwrap();
        fs::remove_dir(&gone).unwrap();
        let state = readable(&dir).unwrap().unwrap();
        assert_eq!(state.rules().unwrap().len(), 1);

        let (stored, _) = state.rules().unwrap().remove(0);
        remove_rule(&dir, Path::new(&stored)).unwrap();
        assert_eq!(state.rules().unwrap(), []);
        let err = remove_rule(&dir, Path::new(&stored)).expect_err("no rule");
        assert!(err.starts_with("no rule for "), "{err}");
    }
}
