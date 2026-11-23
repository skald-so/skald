//! Bounded delivery measurements on a disposable loopback fixture.

mod delivery;
mod http;
mod population;
mod socket;

use std::net::SocketAddrV4;
use std::time::Duration;

use serde::Serialize;
use skald_github_stand_in::GitHub;
use skald_proto::DeviceId;
use tokio::runtime::{Builder, Runtime};

use crate::http::Http;
use crate::population::{ChannelAt, Device};
use crate::socket::Socket;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const MAX_DEVICES: usize = 6;
pub const MAX_RATE: u64 = 6;
pub const MAX_STEPS: usize = 3;
pub const MAX_SECONDS: u64 = 5;
pub const FIXTURE_SEND_LIMIT: u64 = 12;
pub const FIXTURE_SERVER_NAME: &str = "skald.test";

#[derive(Debug, Clone)]
pub struct Settings {
    pub devices: usize,
    pub rates: Vec<u64>,
    pub seconds: u64,
    pub text_bytes: usize,
    pub max_p99_push: Duration,
    pub min_history_per_second: f64,
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !(2..=MAX_DEVICES).contains(&self.devices) {
            return Err(format!("use 2 to {MAX_DEVICES} fixture devices").into());
        }
        if self.rates.is_empty()
            || self.rates.len() > MAX_STEPS
            || self.rates.windows(2).any(|pair| pair[0] >= pair[1])
            || self.rates.iter().any(|rate| !(1..=MAX_RATE).contains(rate))
        {
            return Err(
                format!("use up to {MAX_STEPS} increasing rates from 1 to {MAX_RATE}").into(),
            );
        }
        if !(1..=MAX_SECONDS).contains(&self.seconds) || !(1..=256).contains(&self.text_bytes) {
            return Err(format!("use 1 to {MAX_SECONDS} seconds and 1 to 256 text bytes").into());
        }
        if self.max_p99_push > Duration::from_secs(2)
            || !self.min_history_per_second.is_finite()
            || self.min_history_per_second < 0.0
        {
            return Err("use a push bound up to 2 seconds and a finite history rate bound".into());
        }
        Ok(())
    }
}

/// A prepared fixture population. The CLI supplies the address of the server it just started.
pub struct Delivery {
    runtime: Runtime,
    http: Http,
    at: ChannelAt,
    devices: Vec<Device>,
    sockets: Vec<Socket>,
    settings: Settings,
}

impl Delivery {
    /// Prepare a population on the owned loopback fixture served by `github`.
    pub fn prepare(address: SocketAddrV4, github: &GitHub, settings: Settings) -> Result<Self> {
        settings.validate()?;
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err("the fixture must listen on a local loopback port".into());
        }
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let http = Http::new(format!("http://{address}"));
        let (at, devices, sockets) = runtime.block_on(population::prepare(
            address,
            &http,
            github,
            settings.devices,
        ))?;
        Ok(Self {
            runtime,
            http,
            at,
            devices,
            sockets,
            settings,
        })
    }

    pub fn devices(&self) -> impl Iterator<Item = DeviceId> + '_ {
        self.devices.iter().map(|device| device.id)
    }

    pub fn run(self) -> Result<Report> {
        self.runtime.block_on(delivery::run(
            &self.http,
            self.at,
            self.devices,
            self.sockets,
            &self.settings,
        ))
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    /// Millisecond offsets between observations on one client clock. A push observed before
    /// its Stored frame is recorded as zero. This is not a server-side commit latency.
    pub push_latency_basis: &'static str,
    pub max_p99_push_ms: f64,
    pub min_history_entries_per_second: f64,
    pub steps: Vec<Step>,
    pub maximum_passed_rate: Option<u64>,
    pub first_failed_check: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Step {
    pub offered_messages_per_second: u64,
    pub offered_seconds: u64,
    pub measured_seconds: f64,
    pub stored_messages_per_second: f64,
    pub history_entries_per_second: f64,
    pub push_latency_ms: Option<Percentiles>,
    pub pushes_observed_before_stored: usize,
    pub attempted: usize,
    pub stored: usize,
    pub expected_pushes: usize,
    pub delivered: usize,
    pub acknowledged: usize,
    pub history_stored: usize,
    pub errors: Vec<String>,
    pub failed_check: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}
