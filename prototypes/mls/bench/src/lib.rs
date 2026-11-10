//! Benchmarks of each channel operation on an MLS prototype through the scenario suite's
//! [`Device`], [`Fleet`] and [`DeliveryService`]: the same steps on every library, at each group
//! size of [`SIZES`]. Benchmarks and results: `docs/dev/mls-prototypes.md`, "Benchmarks".
//!
//! A benchmark times only its operation: each run loads its device from the saved state of
//! [`Group`], untimed, and drops it after the operation. The operation reads its group from the
//! library's storage and writes it back, as each step of one-shot mode does. Each run then checks,
//! untimed, that the operation did what it names, and fails if not.

use std::{fmt::Write as _, time::Duration};

use mls_scenario::{
    Admitted, Commit, DeliveryService, Device, Fleet, Joining, Received, Result, Roster, Staged,
    browser, key_packages, revocation,
    scenarios::{CHANNEL, check_joining, leaf, leaves, revocations},
    wire,
};
use web_time::Instant;

/// The group sizes, in devices, that Skald expects of a channel.
pub const SIZES: [usize; 4] = [2, 10, 100, 1000];

/// The length of the application message of [`Operation::Encrypt`] and [`Operation::Decrypt`].
pub const MESSAGE_LEN: usize = 1024;

/// The empty Commits a device catches up on in [`Operation::CatchUp`].
pub const MISSED: usize = 10;

/// How many times a benchmark runs: at least `min_runs` times, then until its runs take `time`
/// in all, at most `max_runs` times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub min_runs: usize,
    pub max_runs: usize,
    pub time: Duration,
}

impl Budget {
    /// One run: the smoke run of `script/test`.
    pub const ONCE: Self = Self {
        min_runs: 1,
        max_runs: 1,
        time: Duration::ZERO,
    };
    /// The runs of the `benchmarks` command.
    pub const FULL: Self = Self {
        min_runs: 5,
        max_runs: 100,
        time: Duration::from_secs(1),
    };
}

/// The times of a benchmark's runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub runs: usize,
    pub median: Duration,
    pub min: Duration,
    pub max: Duration,
}

impl Stats {
    /// The statistics of `samples`; none without samples.
    pub fn of(mut samples: Vec<Duration>) -> Option<Self> {
        samples.sort();
        let (&min, &max) = (samples.first()?, samples.last()?);
        let runs = samples.len();
        let middle = runs / 2;
        let median = if runs % 2 == 1 {
            samples[middle]
        } else {
            (samples[middle - 1] + samples[middle]) / 2
        };
        Some(Self {
            runs,
            median,
            min,
            max,
        })
    }
}

/// Runs `run` as `budget` says; each run returns the time of its timed part.
pub fn measure(budget: Budget, mut run: impl FnMut() -> Result<Duration>) -> Result<Stats> {
    let mut samples = Vec::new();
    let mut total = Duration::ZERO;
    while samples.len() < budget.max_runs
        && (samples.len() < budget.min_runs || total < budget.time)
    {
        let sample = run()?;
        total += sample;
        samples.push(sample);
    }
    Stats::of(samples).ok_or_else(|| "a budget of no runs".into())
}

/// Runs `operation` and returns its output and how long it took.
pub fn time<T>(operation: impl FnOnce() -> Result<T>) -> Result<(T, Duration)> {
    let start = Instant::now();
    let output = operation()?;
    Ok((output, start.elapsed()))
}

/// A benchmarked operation on a group of `n` devices, by device 0 or device 1 of [`Group`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Device 0 creates the group and commits the Add of the other `n - 1` devices from their
    /// KeyPackages, then merges it.
    CreateGroup,
    /// Device 0 commits the Add of one device and merges it.
    CommitAdd,
    /// Device 1 reads and merges that Add.
    ProcessAdd,
    /// Device 0 commits the Remove of device `n - 1`, its revocation in the `authenticated_data`,
    /// and merges it.
    CommitRemove,
    /// Device 1 reads the revocation and merges that Remove; in a group of 2, as the removed
    /// device.
    ProcessRemove,
    /// Device 0 commits an empty Commit and merges it.
    EmptyCommit,
    /// Device 1 reads and merges that empty Commit.
    ProcessEmptyCommit,
    /// The device of [`Operation::CommitAdd`] checks the Welcome's signer and members and joins.
    JoinFromWelcome,
    /// A device outside the group checks the latest GroupInfo's signer and members and joins by
    /// external Commit.
    JoinByExternalCommit,
    /// Device 1 reads the joiner as an Add and merges that external Commit.
    ProcessExternalCommit,
    /// Device 1 makes a KeyPackage.
    KeyPackage,
    /// Device 0 encrypts a message of [`MESSAGE_LEN`] bytes.
    Encrypt,
    /// Device 1 decrypts that message.
    Decrypt,
    /// Device 1 reads and merges, in order, [`MISSED`] empty Commits by device 0 it missed.
    CatchUp,
    /// The Delivery Service, following the group from its GroupInfo, accepts the empty Commit.
    DeliveryService,
    /// A linked browser takes the members from the latest GroupInfo for a wrap by device 0.
    BrowserCheck,
}

impl Operation {
    pub const ALL: [Self; 16] = [
        Self::CreateGroup,
        Self::CommitAdd,
        Self::ProcessAdd,
        Self::CommitRemove,
        Self::ProcessRemove,
        Self::EmptyCommit,
        Self::ProcessEmptyCommit,
        Self::JoinFromWelcome,
        Self::JoinByExternalCommit,
        Self::ProcessExternalCommit,
        Self::KeyPackage,
        Self::Encrypt,
        Self::Decrypt,
        Self::CatchUp,
        Self::DeliveryService,
        Self::BrowserCheck,
    ];

    /// Its name in the results.
    pub fn name(self) -> &'static str {
        match self {
            Self::CreateGroup => "Create the group with its devices",
            Self::CommitAdd => "Commit an Add",
            Self::ProcessAdd => "Process an Add",
            Self::CommitRemove => "Commit a Remove",
            Self::ProcessRemove => "Process a Remove",
            Self::EmptyCommit => "Commit an empty Commit",
            Self::ProcessEmptyCommit => "Process an empty Commit",
            Self::JoinFromWelcome => "Join from a Welcome",
            Self::JoinByExternalCommit => "Join by external Commit",
            Self::ProcessExternalCommit => "Process an external Commit",
            Self::KeyPackage => "Make a KeyPackage",
            Self::Encrypt => "Encrypt a message",
            Self::Decrypt => "Decrypt a message",
            Self::CatchUp => "Catch up on missed Commits",
            Self::DeliveryService => "Delivery Service's Commit check",
            Self::BrowserCheck => "Browser's GroupInfo check",
        }
    }
}

/// An encoded object of a group of `n` devices, whose size the results give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    /// A device's KeyPackage, as an `MLSMessage`.
    KeyPackage,
    /// The Commit of [`Operation::CreateGroup`].
    CreateCommit,
    /// The Commit of [`Operation::CommitAdd`].
    AddCommit,
    /// The Commit of [`Operation::CommitRemove`].
    RemoveCommit,
    /// The Commit of [`Operation::EmptyCommit`].
    EmptyCommit,
    /// The Commit of [`Operation::JoinByExternalCommit`].
    ExternalCommit,
    /// The Welcome of [`Operation::CreateGroup`], for `n - 1` devices.
    CreateWelcome,
    /// The Welcome of [`Operation::CommitAdd`], for one device.
    AddWelcome,
    /// The GroupInfo of the group, with the `ratchet_tree` and `external_pub` extensions.
    GroupInfo,
    /// The saved state of device 1, a member of only this group.
    State,
}

impl Object {
    pub const ALL: [Self; 10] = [
        Self::KeyPackage,
        Self::CreateCommit,
        Self::AddCommit,
        Self::RemoveCommit,
        Self::EmptyCommit,
        Self::ExternalCommit,
        Self::CreateWelcome,
        Self::AddWelcome,
        Self::GroupInfo,
        Self::State,
    ];

    /// Its name in the results.
    pub fn name(self) -> &'static str {
        match self {
            Self::KeyPackage => "KeyPackage",
            Self::CreateCommit => "Commit creating the group",
            Self::AddCommit => "Commit of an Add",
            Self::RemoveCommit => "Commit of a Remove",
            Self::EmptyCommit => "Empty Commit",
            Self::ExternalCommit => "External Commit",
            Self::CreateWelcome => "Welcome creating the group",
            Self::AddWelcome => "Welcome of an Add",
            Self::GroupInfo => "GroupInfo",
            Self::State => "Member's saved state",
        }
    }
}

/// What a member reads of another member's Commit before it merges it, as the scenarios'
/// `process_commit` does: the revocations in its `authenticated_data`, and each device it adds.
/// Fails unless every added device is admitted, so that a benchmark measures only the path where
/// the member merges the Commit.
pub fn read_commit(staged: &Staged, roster: &Roster, admitted: &Admitted) -> Result<()> {
    if !staged.authenticated_data.is_empty() {
        revocation::revoked(&staged.authenticated_data, roster)?;
    }
    staged
        .added
        .iter()
        .try_for_each(|leaf| admitted.check(leaf))
}

/// A channel's group of `size` devices at epoch 1, and the messages of each benchmark, made in
/// that epoch.
///
/// Device 0 created the group with devices `1..size` from their KeyPackages, as "Creating a
/// channel" says; only device 1 joined from the Welcome. The creating Commit has no UpdatePath and
/// no member has committed one. Device `size` has a KeyPackage for [`Operation::CommitAdd`];
/// device `size + 1`, admitted, is outside the group.
pub struct Group<D: Device> {
    size: usize,
    fleet: Fleet<D>,
    admitted: Admitted,
    /// The KeyPackages of devices `1..size`.
    key_packages: Vec<Vec<u8>>,
    created: Commit,
    key_package: Vec<u8>,
    add: Commit,
    authenticated_data: Vec<u8>,
    remove: Commit,
    empty: Commit,
    external: Commit,
    message: Vec<u8>,
    /// [`MISSED`] empty Commits by device 0, one after another from epoch 1.
    missed: Vec<Vec<u8>>,
}

impl<D: Device> Group<D> {
    /// The group of `size` devices, at least 2.
    pub fn new(size: usize) -> Result<Self> {
        if size < 2 {
            return Err("a group of fewer than 2 devices".into());
        }
        let (joiner, outsider) = (size, size + 1);
        let mut fleet = Fleet::<D>::new(size + 2)?;
        let key_packages = (1..size)
            .map(|device| fleet.step(device, |d| d.key_package()))
            .collect::<Result<Vec<_>>>()?;
        let admitted = fleet.admitted();
        let created = fleet.step(0, |d| {
            d.create_group(CHANNEL)?;
            let commit = d.add(CHANNEL, &key_packages, &admitted)?;
            d.merge_pending_commit(CHANNEL)?;
            Ok(commit)
        })?;
        let welcome = created.welcome.as_deref().ok_or("no Welcome")?;
        fleet.step(1, |d| {
            d.join_with(welcome, |joining| check_joining(&admitted, joining))
        })?;
        let key_package = fleet.step(joiner, |d| d.key_package())?;
        let removed = fleet.identity(size - 1).device_id();
        let authenticated_data = revocations(&fleet, 0, &[removed]);

        // Each from device 0's saved state, on a copy that is not saved.
        let committer = || D::load(fleet.identity(0), fleet.state(0));
        let add = committer()?.add(CHANNEL, std::slice::from_ref(&key_package), &admitted)?;
        let remove = committer()?.remove(CHANNEL, &[removed.to_vec()], &authenticated_data)?;
        let empty = committer()?.empty_commit(CHANNEL, &[])?;
        let message = committer()?.encrypt(CHANNEL, &[0x5a; MESSAGE_LEN])?;
        let mut offline = committer()?;
        let missed = (0..MISSED)
            .map(|_| {
                let commit = offline.empty_commit(CHANNEL, &[])?;
                offline.merge_pending_commit(CHANNEL)?;
                Ok(commit.commit)
            })
            .collect::<Result<_>>()?;
        let external = D::load(fleet.identity(outsider), fleet.state(outsider))?
            .join_external(&created.group_info, |joining| {
                check_joining(&admitted, joining)
            })?;
        Ok(Self {
            size,
            fleet,
            admitted,
            key_packages,
            created,
            key_package,
            add,
            authenticated_data,
            remove,
            empty,
            external,
            message,
            missed,
        })
    }

    /// Device `device` from its saved state.
    fn load(&self, device: usize) -> Result<D> {
        D::load(self.fleet.identity(device), self.fleet.state(device))
    }

    /// Fails unless `d` is in the group at `epoch` with the devices `devices`, in leaf order.
    fn check_group(
        &self,
        d: &D,
        epoch: u64,
        devices: impl IntoIterator<Item = usize>,
    ) -> Result<()> {
        let (got, want) = (d.epoch(CHANNEL)?, epoch);
        if got != want {
            return Err(format!("at epoch {got}, want {want}").into());
        }
        if d.members(CHANNEL)? != leaves(&self.fleet, devices) {
            return Err("other members".into());
        }
        Ok(())
    }

    /// Device 1 processes `commit`, by `sender`, and merges it once [`read_commit`] passes. Fails
    /// unless the Commit adds the devices `added` and removes the devices `removed`.
    fn process(
        &self,
        commit: &[u8],
        sender: usize,
        added: &[usize],
        removed: &[usize],
    ) -> Result<Duration> {
        let mut d = self.load(1)?;
        let mut changes = None;
        let (received, elapsed) = time(|| {
            d.process_with(CHANNEL, commit, |staged| {
                read_commit(staged, self.fleet.roster(), &self.admitted)?;
                changes = Some((staged.added.clone(), staged.removed.clone()));
                Ok(())
            })
        })?;
        let want = Received::Commit {
            sender: leaf(self.fleet.identity(sender)),
        };
        if received != want {
            return Err(format!("processed {received:?}, want {want:?}").into());
        }
        let want = (
            leaves(&self.fleet, added.iter().copied()),
            leaves(&self.fleet, removed.iter().copied()),
        );
        if changes.as_ref() != Some(&want) {
            return Err(format!("added and removed {changes:?}, want {want:?}").into());
        }
        Ok(elapsed)
    }

    /// Device 0 makes a Commit with `commit` and merges it. Fails unless the group then has the
    /// devices `members`.
    fn commit(
        &self,
        members: impl IntoIterator<Item = usize>,
        commit: impl FnOnce(&mut D) -> Result<Commit>,
    ) -> Result<Duration> {
        let mut d = self.load(0)?;
        let ((), elapsed) = time(|| {
            commit(&mut d)?;
            d.merge_pending_commit(CHANNEL)
        })?;
        self.check_group(&d, 2, members)?;
        Ok(elapsed)
    }

    /// Fails unless `key_package` is a KeyPackage of device 1 that holds now and that its key
    /// signed: signed again with that key, it is the same bytes.
    fn check_key_package(&self, key_package: &[u8]) -> Result<()> {
        let identity = self.fleet.identity(1);
        let fields = wire::key_package(key_package)?;
        if (fields.identity.as_slice(), fields.signature_key.as_slice())
            != (&identity.device_id()[..], &identity.public_key()[..])
        {
            return Err("a KeyPackage of another device".into());
        }
        if !fields.lifetime.holds(key_packages::now()) {
            return Err("a KeyPackage that does not hold now".into());
        }
        let mut signed = fields.clone();
        signed.sign(identity);
        if signed.encode() != key_package {
            return Err("a KeyPackage with another signature".into());
        }
        Ok(())
    }

    /// Device 1 decrypts `message`. Fails unless it is device 0's message of [`MESSAGE_LEN`]
    /// bytes.
    fn decrypt(&self, message: &[u8]) -> Result<Duration> {
        let mut d = self.load(1)?;
        let (received, elapsed) = time(|| d.process(CHANNEL, message))?;
        let want = Received::Application {
            sender: leaf(self.fleet.identity(0)),
            data: vec![0x5a; MESSAGE_LEN],
        };
        if received != want {
            return Err(format!("decrypted {received:?}").into());
        }
        Ok(elapsed)
    }

    /// The check of a device joining the group.
    fn joining(&self) -> impl FnOnce(&Joining) -> Result<()> + '_ {
        |joining| check_joining(&self.admitted, joining)
    }

    /// Runs `operation` as `budget` says.
    pub fn measure(&self, operation: Operation, budget: Budget) -> Result<Stats> {
        measure(budget, || self.run(operation))
    }

    /// One run of `operation`: the time of its timed part.
    fn run(&self, operation: Operation) -> Result<Duration> {
        let (outsider, joiner, removed) = (self.size + 1, self.size, self.size - 1);
        match operation {
            Operation::CreateGroup => {
                let mut d = D::new(self.fleet.identity(0))?;
                let ((), elapsed) = time(|| {
                    d.create_group(CHANNEL)?;
                    d.add(CHANNEL, &self.key_packages, &self.admitted)?;
                    d.merge_pending_commit(CHANNEL)
                })?;
                self.check_group(&d, 1, 0..self.size)?;
                Ok(elapsed)
            }
            Operation::CommitAdd => self.commit(0..=joiner, |d| {
                d.add(
                    CHANNEL,
                    std::slice::from_ref(&self.key_package),
                    &self.admitted,
                )
            }),
            Operation::ProcessAdd => self.process(&self.add.commit, 0, &[joiner], &[]),
            Operation::CommitRemove => self.commit(0..removed, |d| {
                let removed = self.fleet.identity(removed).device_id();
                d.remove(CHANNEL, &[removed.to_vec()], &self.authenticated_data)
            }),
            Operation::ProcessRemove => self.process(&self.remove.commit, 0, &[], &[removed]),
            Operation::EmptyCommit => self.commit(0..self.size, |d| d.empty_commit(CHANNEL, &[])),
            Operation::ProcessEmptyCommit => self.process(&self.empty.commit, 0, &[], &[]),
            Operation::JoinFromWelcome => {
                let welcome = self.add.welcome.as_deref().ok_or("no Welcome")?;
                let mut d = self.load(joiner)?;
                let (group_id, elapsed) = time(|| d.join_with(welcome, self.joining()))?;
                if group_id != CHANNEL {
                    return Err("joined another group".into());
                }
                self.check_group(&d, 2, 0..=joiner)?;
                Ok(elapsed)
            }
            Operation::JoinByExternalCommit => {
                let mut d = self.load(outsider)?;
                let (_, elapsed) =
                    time(|| d.join_external(&self.created.group_info, self.joining()))?;
                self.check_group(&d, 2, (0..self.size).chain([outsider]))?;
                Ok(elapsed)
            }
            Operation::ProcessExternalCommit => {
                self.process(&self.external.commit, outsider, &[outsider], &[])
            }
            Operation::KeyPackage => {
                let mut d = self.load(1)?;
                let (key_package, elapsed) = time(|| d.key_package())?;
                self.check_key_package(&key_package)?;
                Ok(elapsed)
            }
            Operation::Encrypt => {
                let mut d = self.load(0)?;
                let (message, elapsed) = time(|| d.encrypt(CHANNEL, &[0x5a; MESSAGE_LEN]))?;
                self.decrypt(&message)?;
                Ok(elapsed)
            }
            Operation::Decrypt => self.decrypt(&self.message),
            Operation::CatchUp => {
                let mut d = self.load(1)?;
                let ((), elapsed) = time(|| {
                    self.missed.iter().try_for_each(|commit| {
                        d.process_with(CHANNEL, commit, |staged| {
                            read_commit(staged, self.fleet.roster(), &self.admitted)
                        })
                        .map(drop)
                    })
                })?;
                let epoch = d.epoch(CHANNEL)?;
                if epoch != 1 + MISSED as u64 {
                    return Err(format!("caught up to epoch {epoch}").into());
                }
                Ok(elapsed)
            }
            Operation::DeliveryService => {
                // The service follows the group from its first Commit and GroupInfo.
                let creator = self.fleet.identity(0).device_id();
                let mut service = DeliveryService::<D::Observer>::default();
                service
                    .send(&creator, &self.created.commit)
                    .map_err(|refusal| format!("{refusal:?}"))?;
                service.upload_group_info(&creator, &self.created.group_info)?;
                let ((), elapsed) = time(|| {
                    service
                        .send(&creator, &self.empty.commit)
                        .map_err(|refusal| format!("{refusal:?}").into())
                })?;
                if service.epoch(CHANNEL) != Some(2) {
                    return Err("the service is not in the Commit's epoch".into());
                }
                if service.members(CHANNEL)? != Some(leaves(&self.fleet, 0..self.size)) {
                    return Err("the service has other members".into());
                }
                Ok(elapsed)
            }
            Operation::BrowserCheck => {
                let signer = self.fleet.identity(0).device_id();
                let (members, elapsed) = time(|| {
                    browser::members::<D::Observer>(
                        &self.created.group_info,
                        CHANNEL,
                        1,
                        &signer,
                        &self.admitted,
                    )
                })?;
                if members.len() != self.size {
                    return Err(format!("{} members", members.len()).into());
                }
                Ok(elapsed)
            }
        }
    }

    /// The encoding of `object`; none for a missing Welcome.
    pub fn object(&self, object: Object) -> &[u8] {
        fn welcome(commit: &Commit) -> &[u8] {
            commit.welcome.as_deref().unwrap_or_default()
        }
        match object {
            Object::KeyPackage => &self.key_package,
            Object::CreateCommit => &self.created.commit,
            Object::AddCommit => &self.add.commit,
            Object::RemoveCommit => &self.remove.commit,
            Object::EmptyCommit => &self.empty.commit,
            Object::ExternalCommit => &self.external.commit,
            Object::CreateWelcome => welcome(&self.created),
            Object::AddWelcome => welcome(&self.add),
            Object::GroupInfo => &self.created.group_info,
            Object::State => self.fleet.state(1),
        }
    }

    /// The size of `object` in bytes.
    pub fn size(&self, object: Object) -> usize {
        self.object(object).len()
    }
}

/// The results of one library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Results {
    pub library: &'static str,
    pub sizes: Vec<usize>,
    /// Each operation's statistics, one per size.
    pub times: Vec<(Operation, Vec<Stats>)>,
    /// Each object's size in bytes, one per size.
    pub objects: Vec<(Object, Vec<usize>)>,
}

/// Runs each of `operations` as `budget` says on a [`Group`] of each of `sizes`, and gives the
/// sizes of every [`Object`]. `progress` gets what runs next.
pub fn run<D: Device>(
    sizes: &[usize],
    operations: &[Operation],
    budget: Budget,
    mut progress: impl FnMut(&str),
) -> Result<Results> {
    let mut times: Vec<_> = operations.iter().map(|&op| (op, Vec::new())).collect();
    let mut objects: Vec<_> = Object::ALL.iter().map(|&o| (o, Vec::new())).collect();
    for &size in sizes {
        progress(&format!("{}, {size} devices: the group", D::LIBRARY));
        let group = Group::<D>::new(size)?;
        for (operation, stats) in &mut times {
            progress(&format!(
                "{}, {size} devices: {}",
                D::LIBRARY,
                operation.name()
            ));
            stats.push(group.measure(*operation, budget)?);
        }
        for (object, sizes) in &mut objects {
            sizes.push(group.size(*object));
        }
    }
    Ok(Results {
        library: D::LIBRARY,
        sizes: sizes.to_vec(),
        times,
        objects,
    })
}

/// A time in milliseconds to three significant digits, at most four decimals.
pub fn millis(time: Duration) -> String {
    let decimals = match time.as_nanos() {
        100_000_000.. => 0,
        10_000_000.. => 1,
        1_000_000.. => 2,
        100_000.. => 3,
        _ => 4,
    };
    format!("{:.decimals$}", time.as_secs_f64() * 1e3)
}

/// The header of a table of `first` and each of `sizes`.
fn header(first: &str, sizes: &[usize]) -> String {
    let mut table = format!("| {first} | Library |");
    for size in sizes {
        write!(table, " {size} |").unwrap();
    }
    table.push_str("\n|---|---|");
    table.push_str(&"---|".repeat(sizes.len()));
    table.push('\n');
    table
}

/// The times of `results` as a Markdown table, a row per operation and library, a column per
/// group size: the median, and the fastest and the slowest run, in milliseconds.
pub fn times_table(results: &[Results]) -> String {
    let Some(first) = results.first() else {
        return String::new();
    };
    let mut table = header("Operation, ms", &first.sizes);
    for (row, (operation, _)) in first.times.iter().enumerate() {
        for library in results {
            write!(table, "| {} | {} |", operation.name(), library.library).unwrap();
            for stats in &library.times[row].1 {
                let (median, min, max) =
                    (millis(stats.median), millis(stats.min), millis(stats.max));
                write!(table, " {median} ({min}–{max}) |").unwrap();
            }
            table.push('\n');
        }
    }
    table
}

/// The object sizes of `results` as a Markdown table, a row per object and library, a column per
/// group size, in bytes.
pub fn sizes_table(results: &[Results]) -> String {
    let Some(first) = results.first() else {
        return String::new();
    };
    let mut table = header("Object, bytes", &first.sizes);
    for (row, (object, _)) in first.objects.iter().enumerate() {
        for library in results {
            write!(table, "| {} | {} |", object.name(), library.library).unwrap();
            for size in &library.objects[row].1 {
                write!(table, " {size} |").unwrap();
            }
            table.push('\n');
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn stats_give_the_median_and_the_bounds() {
        assert_eq!(Stats::of(Vec::new()), None);
        assert_eq!(
            Stats::of(vec![ms(7)]),
            Some(Stats {
                runs: 1,
                median: ms(7),
                min: ms(7),
                max: ms(7)
            })
        );
        assert_eq!(
            Stats::of(vec![ms(9), ms(1), ms(4)]),
            Some(Stats {
                runs: 3,
                median: ms(4),
                min: ms(1),
                max: ms(9)
            })
        );
        assert_eq!(
            Stats::of(vec![ms(8), ms(2), ms(10), ms(4)]),
            Some(Stats {
                runs: 4,
                median: ms(6),
                min: ms(2),
                max: ms(10)
            })
        );
    }

    /// The number of runs `measure` makes with `budget` of a run that takes `each`.
    fn runs(budget: Budget, each: Duration) -> usize {
        let mut runs = 0;
        let stats = measure(budget, || {
            runs += 1;
            Ok(each)
        })
        .unwrap();
        assert_eq!(stats.runs, runs);
        runs
    }

    #[test]
    fn measures_within_the_budget() {
        let budget = Budget {
            min_runs: 3,
            max_runs: 10,
            time: ms(1000),
        };
        assert_eq!(runs(budget, ms(300)), 4, "until the runs take the time");
        assert_eq!(runs(budget, ms(250)), 4, "until they take it, not past it");
        assert_eq!(runs(budget, ms(600)), 3, "at least min_runs");
        assert_eq!(runs(budget, ms(1)), 10, "at most max_runs");
        assert_eq!(runs(Budget::ONCE, ms(0)), 1);
        assert_eq!(runs(Budget::FULL, ms(400)), 5);

        let none = Budget {
            max_runs: 0,
            ..budget
        };
        assert!(measure(none, || Ok(ms(1))).is_err());
        let mut runs = 0;
        assert!(
            measure(budget, || {
                runs += 1;
                Err("fails".into())
            })
            .is_err()
        );
        assert_eq!(runs, 1, "stops at a failed run");
    }

    #[test]
    fn times_what_it_runs() {
        let (output, elapsed) = time(|| {
            std::thread::sleep(ms(20));
            Ok(7)
        })
        .unwrap();
        assert_eq!(output, 7);
        assert!(elapsed >= ms(20) && elapsed < ms(2000), "{elapsed:?}");
        assert!(time(|| Err::<(), _>("fails".into())).is_err());
    }

    #[test]
    fn gives_milliseconds_to_three_digits() {
        for (time, want) in [
            (Duration::from_micros(123_456_700), "123457"),
            (ms(100), "100"),
            (Duration::from_micros(99_940), "99.9"),
            (Duration::from_micros(12_345), "12.3"),
            (ms(10), "10.0"),
            (Duration::from_micros(1_234), "1.23"),
            (ms(1), "1.00"),
            (Duration::from_micros(123), "0.123"),
            (Duration::from_micros(100), "0.100"),
            (Duration::from_nanos(12_345), "0.0123"),
        ] {
            assert_eq!(millis(time), want, "{time:?}");
        }
    }

    fn stats(median: u64, min: u64, max: u64) -> Stats {
        Stats {
            runs: 5,
            median: ms(median),
            min: ms(min),
            max: ms(max),
        }
    }

    fn results(library: &'static str, scale: u64) -> Results {
        Results {
            library,
            sizes: vec![2, 10],
            times: vec![
                (
                    Operation::Encrypt,
                    vec![stats(2 * scale, scale, 3 * scale), stats(4, 4, 5)],
                ),
                (Operation::Decrypt, vec![stats(1, 1, 1), stats(6, 5, 7)]),
            ],
            objects: vec![
                (Object::KeyPackage, vec![300, 300]),
                (Object::GroupInfo, vec![900, 4000 * scale as usize]),
            ],
        }
    }

    #[test]
    fn tables_have_a_row_per_library_and_a_column_per_size() {
        let results = [results("A", 1), results("B", 10)];
        assert_eq!(
            times_table(&results),
            "| Operation, ms | Library | 2 | 10 |\n\
             |---|---|---|---|\n\
             | Encrypt a message | A | 2.00 (1.00–3.00) | 4.00 (4.00–5.00) |\n\
             | Encrypt a message | B | 20.0 (10.0–30.0) | 4.00 (4.00–5.00) |\n\
             | Decrypt a message | A | 1.00 (1.00–1.00) | 6.00 (5.00–7.00) |\n\
             | Decrypt a message | B | 1.00 (1.00–1.00) | 6.00 (5.00–7.00) |\n"
        );
        assert_eq!(
            sizes_table(&results),
            "| Object, bytes | Library | 2 | 10 |\n\
             |---|---|---|---|\n\
             | KeyPackage | A | 300 | 300 |\n\
             | KeyPackage | B | 300 | 300 |\n\
             | GroupInfo | A | 900 | 4000 |\n\
             | GroupInfo | B | 900 | 40000 |\n"
        );
        assert_eq!(times_table(&[]), "");
        assert_eq!(sizes_table(&[]), "");
    }

    #[test]
    fn reads_a_commit_before_merging_it() {
        use mls_scenario::{Identity, Leaf};

        let leaf = |identity: &Identity| Leaf {
            identity: identity.device_id().to_vec(),
            signature_key: identity.public_key().to_vec(),
        };
        let (admitted, other) = (leaf(&Identity::generate()), leaf(&Identity::generate()));
        let roster = Roster::default();
        let staged = |added: Vec<Leaf>, authenticated_data: &[u8]| Staged {
            sender: admitted.clone(),
            removed: Vec::new(),
            added,
            authenticated_data: authenticated_data.to_vec(),
        };
        let admits = Admitted::default().with(&admitted);
        assert!(read_commit(&staged(Vec::new(), b""), &roster, &admits).is_ok());
        assert!(read_commit(&staged(vec![admitted.clone()], b""), &roster, &admits).is_ok());
        assert!(
            read_commit(
                &staged(vec![admitted.clone(), other], b""),
                &roster,
                &admits
            )
            .is_err(),
            "adds a device that is not admitted"
        );
        assert!(
            read_commit(&staged(Vec::new(), b"\xff"), &roster, &admits).is_err(),
            "authenticated_data that is no list of revocations"
        );
    }

    /// Each run fails when its operation does something else than it names.
    fn runs_check_what_they_did<D: Device>() {
        let lib = D::LIBRARY;
        let group = Group::<D>::new(3).unwrap();
        let (joiner, removed, outsider) = (3, 2, 4);
        let (add, remove, empty) = (&group.add.commit, &group.remove.commit, &group.empty.commit);
        for (commit, sender, added, removed, what) in [
            (
                empty,
                0,
                &[joiner][..],
                &[][..],
                "an empty Commit as an Add",
            ),
            (add, 0, &[], &[], "an Add as an empty Commit"),
            (empty, 0, &[], &[removed], "an empty Commit as a Remove"),
            (remove, 0, &[], &[], "a Remove as an empty Commit"),
            (add, 0, &[outsider], &[], "an Add of another device"),
            (empty, 1, &[], &[], "a Commit by another device"),
        ] {
            assert!(
                group.process(commit, sender, added, removed).is_err(),
                "{lib}: processed {what}"
            );
        }
        let empty_commit = |d: &mut D| d.empty_commit(CHANNEL, &[]);
        assert!(
            group.commit(0..removed, empty_commit).is_err(),
            "{lib}: a Remove"
        );
        assert!(
            group.commit(0..=joiner, empty_commit).is_err(),
            "{lib}: an Add"
        );
        assert!(
            group.commit(0..3, empty_commit).is_ok(),
            "{lib}: an empty Commit"
        );

        let other = group.load(0).unwrap();
        assert!(group.check_group(&other, 1, 0..3).is_ok());
        assert!(
            group.check_group(&other, 2, 0..3).is_err(),
            "{lib}: another epoch"
        );
        assert!(
            group.check_group(&other, 1, 0..2).is_err(),
            "{lib}: other members"
        );

        assert!(group.check_key_package(&group.key_packages[0]).is_ok());
        let mut forged = group.key_packages[0].clone();
        *forged.last_mut().unwrap() ^= 1;
        assert!(
            group.check_key_package(&forged).is_err(),
            "{lib}: a bad signature"
        );
        assert!(
            group.check_key_package(&group.key_package).is_err(),
            "{lib}: another device's KeyPackage"
        );

        assert!(group.decrypt(&group.message).is_ok());
        let other = group
            .load(1)
            .unwrap()
            .encrypt(CHANNEL, &[0x5a; MESSAGE_LEN])
            .unwrap();
        assert!(
            group.decrypt(&other).is_err(),
            "{lib}: device 1's own message"
        );
        let other = group.load(0).unwrap().encrypt(CHANNEL, b"other").unwrap();
        assert!(group.decrypt(&other).is_err(), "{lib}: another message");
    }

    #[test]
    fn openmls_runs_check_what_they_did() {
        runs_check_what_they_did::<mls_proto_openmls::OpenMlsDevice>();
    }

    #[test]
    fn mls_rs_runs_check_what_they_did() {
        runs_check_what_they_did::<mls_proto_mls_rs::MlsRsDevice>();
    }

    /// Each object is the one it names.
    fn objects_are_what_they_name<D: Device>() {
        let lib = D::LIBRARY;
        let group = Group::<D>::new(3).unwrap();
        let (joiner, removed, outsider) = (3, 2, 4);
        let object = |object| group.object(object);

        let key_package = wire::key_package(object(Object::KeyPackage)).unwrap();
        assert_eq!(
            key_package.identity,
            group.fleet.identity(joiner).device_id(),
            "{lib}: the KeyPackage of the joiner"
        );

        let created = wire::framing(object(Object::CreateCommit)).unwrap();
        assert_eq!(
            (created.epoch, created.content_type),
            (0, wire::COMMIT),
            "{lib}: the Commit of epoch 0"
        );
        let add = object(Object::AddCommit);
        assert_eq!(
            wire::framing(add).unwrap().epoch,
            1,
            "{lib}: an Add of epoch 1"
        );
        assert!(
            group.process(add, 0, &[joiner], &[]).is_ok(),
            "{lib}: the Add of the joiner"
        );
        let commit = |object| wire::public_commit(group.object(object)).unwrap();
        for (object, adds) in [(Object::CreateCommit, 2), (Object::AddCommit, 1)] {
            let adding = commit(object);
            assert_eq!(
                (adding.sender, adding.other_proposals, adding.path),
                (wire::CommitSender::Member(0), adds, false),
                "{lib}: {object:?}: device 0's {adds} Adds without an UpdatePath"
            );
        }
        let (sender, none) = (wire::CommitSender::Member(0), Vec::<u32>::new());
        let remove = commit(Object::RemoveCommit);
        assert_eq!(
            (remove.sender, &remove.removed, remove.external_init),
            (sender, &vec![removed as u32], false),
            "{lib}: device 0's Remove of leaf {removed}"
        );
        let empty = commit(Object::EmptyCommit);
        assert_eq!(
            (empty.sender, &empty.removed, empty.external_init),
            (sender, &none, false),
            "{lib}: device 0's empty Commit"
        );
        assert!(
            empty.path && empty.other_proposals == 0,
            "{lib}: an UpdatePath and no proposals"
        );
        let external = commit(Object::ExternalCommit);
        assert_eq!(
            (external.sender, external.external_init),
            (wire::CommitSender::NewMember, true),
            "{lib}: an external Commit"
        );
        assert!(
            group
                .process(object(Object::ExternalCommit), outsider, &[outsider], &[])
                .is_ok(),
            "{lib}: the outsider's external Commit"
        );

        let reference = |key_package: &[u8]| wire::key_package_ref(key_package).unwrap();
        let recipients = |object| wire::welcome_recipients(group.object(object)).unwrap();
        assert_eq!(
            recipients(Object::CreateWelcome),
            group
                .key_packages
                .iter()
                .map(|key_package| reference(key_package))
                .collect::<Vec<_>>(),
            "{lib}: the Welcome for devices 1 and 2"
        );
        assert_eq!(
            recipients(Object::AddWelcome),
            [reference(&group.key_package)],
            "{lib}: the Welcome for the joiner"
        );

        let group_info = wire::group_info(object(Object::GroupInfo)).unwrap();
        assert_eq!(
            (
                group_info.group_id.as_slice(),
                group_info.epoch,
                group_info.signer
            ),
            (CHANNEL, 1, 0),
            "{lib}: the group's GroupInfo of epoch 1, by device 0"
        );

        let mut member = D::load(group.fleet.identity(1), object(Object::State)).unwrap();
        assert_eq!(
            member.process(CHANNEL, &group.message).unwrap(),
            Received::Application {
                sender: leaf(group.fleet.identity(0)),
                data: vec![0x5a; MESSAGE_LEN],
            },
            "{lib}: device 1's state reads device 0's message"
        );
    }

    #[test]
    fn openmls_objects_are_what_they_name() {
        objects_are_what_they_name::<mls_proto_openmls::OpenMlsDevice>();
    }

    #[test]
    fn mls_rs_objects_are_what_they_name() {
        objects_are_what_they_name::<mls_proto_mls_rs::MlsRsDevice>();
    }

    #[test]
    fn names_every_operation_and_object_once() {
        let operations: std::collections::BTreeSet<_> =
            Operation::ALL.iter().map(|op| op.name()).collect();
        assert_eq!(operations.len(), Operation::ALL.len());
        let objects: std::collections::BTreeSet<_> =
            Object::ALL.iter().map(|object| object.name()).collect();
        assert_eq!(objects.len(), Object::ALL.len());
    }
}
