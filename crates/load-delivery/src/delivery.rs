//! Paced sends, history uploads and per-recipient delivery checks.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use skald_proto::{DeviceId, Frame, HistoryItem, MessageId, Push, RawEnvelope};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::{Instant as TokioInstant, sleep_until, timeout_at};

use crate::http::{Http, json as read_json, unb64};
use crate::population::{ChannelAt, Device};
use crate::socket::Socket;
use crate::{Percentiles, Report, Result, Settings, Step};

const DRAIN: Duration = Duration::from_millis(800);
const QUIET: Duration = Duration::from_millis(80);

struct Event {
    device: usize,
    observed: Instant,
    response: Result<Frame>,
}

async fn serve(
    device: usize,
    mut socket: Socket,
    mut commands: UnboundedReceiver<Frame>,
    events: UnboundedSender<Event>,
) {
    let result: Result<()> = async {
        loop {
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else { return Ok(()) };
                    socket.send(&command).await?;
                }
                received = socket.next() => {
                    let (observed, frame) = received?;
                    if let Frame::Push(Push::Message { seq, .. } | Push::Status { seq, .. }) = &frame {
                        socket.send(&Frame::Ack { seq: *seq }).await?;
                    }
                    if events.send(Event { device, observed, response: Ok(frame) }).is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
    .await;
    if let Err(err) = result {
        let _ = events.send(Event {
            device,
            observed: Instant::now(),
            response: Err(err),
        });
    }
}

struct Sent {
    device: usize,
    envelope: RawEnvelope,
    entry: Vec<u8>,
    upload: u64,
    stored: Option<Instant>,
    history_answered: bool,
    pushes: BTreeMap<usize, Instant>,
}

struct Checks {
    sent: BTreeMap<MessageId, Sent>,
    order: Vec<Vec<MessageId>>,
    highest: Vec<u64>,
    errors: Vec<String>,
    last: Instant,
}

impl Checks {
    fn new(devices: usize, start: Instant) -> Self {
        Self {
            sent: BTreeMap::new(),
            order: vec![Vec::new(); devices],
            highest: vec![0; devices],
            errors: Vec::new(),
            last: start,
        }
    }

    fn observe(&mut self, event: Event, at: ChannelAt, workers: &[UnboundedSender<Frame>]) {
        let Event {
            device,
            observed,
            response,
        } = event;
        let frame = match response {
            Ok(frame) => frame,
            Err(err) => {
                self.errors.push(format!("device {device}: {err}"));
                return;
            }
        };
        match frame {
            Frame::Stored(stored) => {
                let msg = stored.send_key.msg_id;
                let Some(sent) = self.sent.get_mut(&msg) else {
                    self.errors.push(format!("unexpected Stored for {msg}"));
                    return;
                };
                if device != sent.device
                    || stored.send_key.recipient.is_some()
                    || sent.stored.is_some()
                {
                    self.errors
                        .push(format!("incorrect or repeated Stored for {msg}"));
                    return;
                }
                sent.stored = Some(observed);
                self.last = self.last.max(observed);
                let frame = Frame::History {
                    upload: sent.upload,
                    space: at.space,
                    channel: at.id,
                    item: HistoryItem::Entry {
                        msg_id: msg,
                        entry: sent.entry.clone(),
                    },
                };
                if workers[device].send(frame).is_err() {
                    self.errors.push(format!(
                        "device {device} could not upload history for {msg}"
                    ));
                }
            }
            Frame::HistoryStored { upload } => {
                let sent = self
                    .sent
                    .iter_mut()
                    .find(|(_, sent)| sent.upload == upload && sent.device == device);
                match sent {
                    Some((_, sent)) if sent.stored.is_some() && !sent.history_answered => {
                        sent.history_answered = true;
                        self.last = self.last.max(observed);
                    }
                    _ => self.errors.push(format!(
                        "unexpected HistoryStored {upload} on device {device}"
                    )),
                }
            }
            Frame::SendRefused { send_key, problem } => {
                self.errors
                    .push(format!("send {} refused: {problem}", send_key.msg_id));
            }
            Frame::HistoryRefused { upload, problem } => {
                self.errors
                    .push(format!("history upload {upload} refused: {problem}"));
            }
            Frame::Push(Push::Message { seq, envelope }) => {
                let msg = envelope.envelope().header.msg_id;
                let Some(sent) = self.sent.get_mut(&msg) else {
                    self.errors
                        .push(format!("unexpected push {msg} on device {device}"));
                    return;
                };
                if sent.device == device || sent.envelope != envelope {
                    self.errors
                        .push(format!("incorrect push {msg} on device {device}"));
                    return;
                }
                if sent.pushes.contains_key(&device) {
                    self.errors
                        .push(format!("duplicate push {msg} on device {device}"));
                    return;
                }
                if seq.get() <= self.highest[device] {
                    self.errors.push(format!(
                        "non-increasing mailbox sequence for {msg} on device {device}"
                    ));
                }
                self.highest[device] = seq.get();
                sent.pushes.insert(device, observed);
                self.order[device].push(msg);
                self.last = self.last.max(observed);
            }
            _ => {}
        }
    }

    fn complete(&self, attempted: usize, devices: usize) -> bool {
        self.sent.len() == attempted
            && self
                .sent
                .values()
                .all(|sent| sent.history_answered && sent.pushes.len() == devices - 1)
    }

    fn check_order(&mut self, devices: &[DeviceId]) {
        let mut positions = vec![BTreeMap::new(); self.order.len()];
        for (recipient, order) in self.order.iter().enumerate() {
            let mut counters = BTreeMap::new();
            for (position, id) in order.iter().enumerate() {
                let sent = &self.sent[id];
                if counters
                    .insert(sent.device, sent.upload)
                    .is_some_and(|last| last >= sent.upload)
                {
                    self.errors
                        .push(format!("out-of-order push {id} on {}", devices[recipient]));
                }
                positions[recipient].insert(id, position);
            }
        }
        // Recipients see the same server order on the intersection of their streams. A device
        // does not receive its own sends.
        for (left, order) in self.order.iter().enumerate() {
            for (right, other) in positions.iter().enumerate().skip(left + 1) {
                let mut last = None;
                for id in order {
                    if let Some(&position) = other.get(id) {
                        if last.is_some_and(|last| last >= position) {
                            self.errors.push(format!(
                                "channel order differs for {id} on {} and {}",
                                devices[left], devices[right]
                            ));
                        }
                        last = Some(position);
                    }
                }
            }
        }
    }

    fn finish(
        mut self,
        devices: &[DeviceId],
        kept: &BTreeMap<Vec<u8>, usize>,
        start: Instant,
        rate: u64,
        settings: &Settings,
    ) -> Step {
        self.check_order(devices);
        let mut latencies = Vec::new();
        let mut before = 0;
        let mut stored = 0;
        let mut history_stored = 0;
        let mut delivered = 0;
        for (id, sent) in &self.sent {
            if let Some(stored_at) = sent.stored {
                stored += 1;
                for &observed in sent.pushes.values() {
                    before += usize::from(observed < stored_at);
                    latencies.push(observed.saturating_duration_since(stored_at));
                }
            } else {
                self.errors.push(format!("no Stored for {id}"));
            }
            match (
                sent.history_answered,
                kept.get(&sent.entry).copied().unwrap_or_default(),
            ) {
                (true, 1) => history_stored += 1,
                (_, count) => self.errors.push(format!(
                    "history for {id}: confirmation {}, stored copies {count}",
                    sent.history_answered
                )),
            }
            for (recipient, device) in devices.iter().enumerate() {
                if recipient == sent.device {
                    continue;
                }
                if sent.pushes.contains_key(&recipient) {
                    delivered += 1;
                } else {
                    self.errors
                        .push(format!("lost message {id} for device {device}"));
                }
            }
        }
        let elapsed = self
            .last
            .duration_since(start)
            .max(Duration::from_secs(settings.seconds));
        let history_rate = history_stored as f64 / elapsed.as_secs_f64();
        let push_latency_ms = percentiles(&mut latencies);
        let failed_check = if !self.errors.is_empty() {
            Some("delivery_integrity")
        } else if push_latency_ms
            .as_ref()
            .is_some_and(|latency| latency.p99 > millis(settings.max_p99_push))
        {
            Some("max_p99_push_ms")
        } else if history_rate < settings.min_history_per_second {
            Some("min_history_entries_per_second")
        } else {
            None
        };
        Step {
            offered_messages_per_second: rate,
            offered_seconds: settings.seconds,
            measured_seconds: elapsed.as_secs_f64(),
            stored_messages_per_second: stored as f64 / elapsed.as_secs_f64(),
            history_entries_per_second: history_rate,
            push_latency_ms,
            pushes_observed_before_stored: before,
            attempted: self.sent.len(),
            stored,
            expected_pushes: self.sent.len() * (devices.len() - 1),
            delivered,
            acknowledged: delivered,
            history_stored,
            errors: self.errors,
            failed_check,
        }
    }
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn percentiles(samples: &mut [Duration]) -> Option<Percentiles> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    let value = |rank: usize| millis(samples[(samples.len() * rank).div_ceil(100) - 1]);
    Some(Percentiles {
        p50: value(50),
        p95: value(95),
        p99: value(99),
    })
}

fn read_history(http: &Http, at: ChannelAt, device: &Device) -> Result<BTreeMap<Vec<u8>, usize>> {
    let value = read_json(http.api("GET", &at.path("/history/entries"), &device.token, None)?)?;
    let entries = value["entries"]
        .as_array()
        .ok_or("missing history entries")?;
    let mut kept = BTreeMap::new();
    for entry in entries {
        let bytes = unb64(entry.as_str().ok_or("invalid history entry")?)?;
        *kept.entry(bytes).or_default() += 1;
    }
    Ok(kept)
}

struct Work<'a> {
    at: ChannelAt,
    http: &'a Http,
    settings: &'a Settings,
    devices: Vec<Device>,
    ids: Vec<DeviceId>,
    workers: Vec<UnboundedSender<Frame>>,
    events: UnboundedReceiver<Event>,
    text: String,
    upload: u64,
}

impl Work<'_> {
    fn send(&mut self, checks: &mut Checks) -> Result<()> {
        let device = self.upload as usize % self.devices.len().min(2);
        self.upload += 1;
        let (envelope, entry) = self.devices[device].message(self.at, &self.text)?;
        let msg = envelope.envelope().header.msg_id;
        let sent = Sent {
            device,
            envelope: envelope.clone(),
            entry,
            upload: self.upload,
            stored: None,
            history_answered: false,
            pushes: BTreeMap::new(),
        };
        checks.sent.insert(msg, sent);
        if self.workers[device]
            .send(Frame::Send {
                envelope,
                report: true,
            })
            .is_err()
        {
            checks
                .errors
                .push(format!("device {device} could not send {msg}"));
        }
        Ok(())
    }

    async fn observe_until(&mut self, checks: &mut Checks, deadline: Instant) {
        let deadline = TokioInstant::from_std(deadline);
        loop {
            if TokioInstant::now() >= deadline {
                return;
            }
            match timeout_at(deadline, self.events.recv()).await {
                Ok(Some(event)) => checks.observe(event, self.at, &self.workers),
                Err(_) => return,
                Ok(None) => {
                    checks
                        .errors
                        .push("all fixture connections ended".to_owned());
                    sleep_until(deadline).await;
                    return;
                }
            }
        }
    }

    async fn step(&mut self, rate: u64) -> Result<Step> {
        let start = Instant::now();
        let planned = self.settings.seconds * rate;
        let stops = start + Duration::from_secs(self.settings.seconds);
        let interval = Duration::from_nanos(1_000_000_000 / rate);
        let mut checks = Checks::new(self.devices.len(), start);
        let mut scheduled = start;
        for _ in 0..planned {
            self.observe_until(&mut checks, scheduled).await;
            let attempted_at = Instant::now();
            if attempted_at >= stops {
                checks
                    .errors
                    .push(format!("could not pace {planned} sends within the step"));
                break;
            }
            self.send(&mut checks)?;
            scheduled = attempted_at + interval;
        }
        self.observe_until(&mut checks, stops).await;
        let drain = if checks.complete(planned as usize, self.devices.len()) {
            QUIET
        } else {
            DRAIN
        };
        self.observe_until(&mut checks, stops + drain).await;
        let kept = match read_history(self.http, self.at, &self.devices[0]) {
            Ok(kept) => kept,
            Err(err) => {
                checks
                    .errors
                    .push(format!("could not verify stored history: {err}"));
                BTreeMap::new()
            }
        };
        Ok(checks.finish(&self.ids, &kept, start, rate, self.settings))
    }
}

pub async fn run(
    http: &Http,
    at: ChannelAt,
    devices: Vec<Device>,
    sockets: Vec<Socket>,
    settings: &Settings,
) -> Result<Report> {
    let (events_out, events) = mpsc::unbounded_channel();
    let mut workers = Vec::with_capacity(sockets.len());
    let mut tasks = Vec::with_capacity(sockets.len());
    for (index, socket) in sockets.into_iter().enumerate() {
        let (send, receive) = mpsc::unbounded_channel();
        tasks.push(tokio::spawn(serve(
            index,
            socket,
            receive,
            events_out.clone(),
        )));
        workers.push(send);
    }
    drop(events_out);
    let ids = devices.iter().map(|device| device.id).collect();
    let text = "x".repeat(settings.text_bytes);
    let mut work = Work {
        at,
        http,
        settings,
        devices,
        ids,
        workers,
        events,
        text,
        upload: 0,
    };
    let mut report = Report {
        push_latency_basis: "Client monotonic time from observing Stored to observing the push, floored at zero.",
        max_p99_push_ms: millis(settings.max_p99_push),
        min_history_entries_per_second: settings.min_history_per_second,
        steps: Vec::new(),
        maximum_passed_rate: None,
        first_failed_check: None,
    };
    for &rate in &settings.rates {
        let step = work.step(rate).await?;
        report.first_failed_check = step.failed_check;
        report.steps.push(step);
        if report.first_failed_check.is_some() {
            break;
        }
        report.maximum_passed_rate = Some(rate);
    }
    drop(work);
    for task in tasks {
        task.await?;
    }
    Ok(report)
}
