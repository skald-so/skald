//! Benchmarks of each channel operation on an MLS prototype through the scenario suite's
//! [`Device`], [`Fleet`] and [`DeliveryService`]: the same steps on every library, at each group
//! size of [`SIZES`]. Benchmarks and results: `docs/dev/mls-prototypes.md`, "Benchmarks".
//!
//! A benchmark times only its operation: each run loads its device from the saved state of
//! [`Group`], untimed, and drops it after the operation. The operation reads its group from the
//! library's storage and writes it back, as each step of one-shot mode does.

use std::{fmt::Write as _, time::Duration};

use mls_scenario::{
    Admitted, Commit, DeliveryService, Device, Fleet, Joining, Received, Result, Roster, Staged,
    browser, revocation,
    scenarios::{CHANNEL, check_joining, leaf, revocations},
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
    /// The runs of the `bench` command.
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
/// channel" says; only device 1 joined from the Welcome. The tree's parent nodes are only those
/// the creating Commit's UpdatePath set, if it has one: no other member has committed, so a
/// Commit's UpdatePath encrypts mostly to members' leaves, the costliest case for a group of that
/// size. Device `size` has a KeyPackage for [`Operation::CommitAdd`]; device `size + 1`,
/// admitted, is outside the group.
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

    /// Device 1 processes `commit`, by `sender`, and merges it once [`read_commit`] passes.
    fn process(&self, commit: &[u8], sender: usize) -> Result<Duration> {
        let mut d = self.load(1)?;
        let (received, elapsed) = time(|| {
            d.process_with(CHANNEL, commit, |staged| {
                read_commit(staged, self.fleet.roster(), &self.admitted)
            })
        })?;
        let want = Received::Commit {
            sender: leaf(self.fleet.identity(sender)),
        };
        if received != want {
            return Err(format!("processed {received:?}, want {want:?}").into());
        }
        Ok(elapsed)
    }

    /// Device 0 makes a Commit with `commit` and merges it.
    fn commit(&self, commit: impl FnOnce(&mut D) -> Result<Commit>) -> Result<Duration> {
        let mut d = self.load(0)?;
        let ((), elapsed) = time(|| {
            commit(&mut d)?;
            d.merge_pending_commit(CHANNEL)
        })?;
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
        let (outsider, joiner) = (self.size + 1, self.size);
        match operation {
            Operation::CreateGroup => {
                let mut d = D::new(self.fleet.identity(0))?;
                let ((), elapsed) = time(|| {
                    d.create_group(CHANNEL)?;
                    d.add(CHANNEL, &self.key_packages, &self.admitted)?;
                    d.merge_pending_commit(CHANNEL)
                })?;
                Ok(elapsed)
            }
            Operation::CommitAdd => self.commit(|d| {
                d.add(
                    CHANNEL,
                    std::slice::from_ref(&self.key_package),
                    &self.admitted,
                )
            }),
            Operation::ProcessAdd => self.process(&self.add.commit, 0),
            Operation::CommitRemove => self.commit(|d| {
                let removed = self.fleet.identity(self.size - 1).device_id();
                d.remove(CHANNEL, &[removed.to_vec()], &self.authenticated_data)
            }),
            Operation::ProcessRemove => self.process(&self.remove.commit, 0),
            Operation::EmptyCommit => self.commit(|d| d.empty_commit(CHANNEL, &[])),
            Operation::ProcessEmptyCommit => self.process(&self.empty.commit, 0),
            Operation::JoinFromWelcome => {
                let welcome = self.add.welcome.as_deref().ok_or("no Welcome")?;
                let mut d = self.load(joiner)?;
                let (group_id, elapsed) = time(|| d.join_with(welcome, self.joining()))?;
                if group_id != CHANNEL {
                    return Err("joined another group".into());
                }
                Ok(elapsed)
            }
            Operation::JoinByExternalCommit => {
                let mut d = self.load(outsider)?;
                let (_, elapsed) =
                    time(|| d.join_external(&self.created.group_info, self.joining()))?;
                Ok(elapsed)
            }
            Operation::ProcessExternalCommit => self.process(&self.external.commit, outsider),
            Operation::KeyPackage => {
                let mut d = self.load(1)?;
                let (_, elapsed) = time(|| d.key_package())?;
                Ok(elapsed)
            }
            Operation::Encrypt => {
                let mut d = self.load(0)?;
                let (_, elapsed) = time(|| d.encrypt(CHANNEL, &[0x5a; MESSAGE_LEN]))?;
                Ok(elapsed)
            }
            Operation::Decrypt => {
                let mut d = self.load(1)?;
                let (received, elapsed) = time(|| d.process(CHANNEL, &self.message))?;
                let want = Received::Application {
                    sender: leaf(self.fleet.identity(0)),
                    data: vec![0x5a; MESSAGE_LEN],
                };
                if received != want {
                    return Err(format!("decrypted {received:?}").into());
                }
                Ok(elapsed)
            }
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

    /// The size of `object` in bytes.
    pub fn size(&self, object: Object) -> usize {
        let welcome = |commit: &Commit| commit.welcome.as_ref().map_or(0, Vec::len);
        match object {
            Object::KeyPackage => self.key_package.len(),
            Object::CreateCommit => self.created.commit.len(),
            Object::AddCommit => self.add.commit.len(),
            Object::RemoveCommit => self.remove.commit.len(),
            Object::EmptyCommit => self.empty.commit.len(),
            Object::ExternalCommit => self.external.commit.len(),
            Object::CreateWelcome => welcome(&self.created),
            Object::AddWelcome => welcome(&self.add),
            Object::GroupInfo => self.created.group_info.len(),
            Object::State => self.fleet.state(1).len(),
        }
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
