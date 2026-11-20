//! Queues and tasks: `docs/protocol/queues.md`. The acceptance tests of its items are in
//! `crates/crypto/tests/queues/`.

mod common;

use common::{
    a2a, alice, alice_device, assert_field_problem, bob_device, channel, content, hex, msg,
    server_device, to_hex,
};
use skald_proto::{
    Body, Cbor, Claim, ClaimId, Content, FieldProblem, Outcome, QUEUE_KEY_LABEL, QueueId,
    QueueKeyBody, QueueMode, QueuedTask, Sched, SignedBody, TASK_LABEL, Task, TaskBody, TaskId,
    TaskResult, Unknown,
};

const DEADLINE: u64 = 1_800_604_800;
const ENQUEUED: u64 = 1_800_000_000;

fn queue() -> QueueId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5eb".parse().unwrap()
}

fn task_id() -> TaskId {
    "019a6f21-0000-7000-8000-0000000000a1".parse().unwrap()
}

fn claim_id() -> ClaimId {
    "019a6f22-0000-7000-8000-0000000000c1".parse().unwrap()
}

/// `50` and the hex of a 16-byte ID.
fn id(bytes: &[u8; 16]) -> String {
    format!("50{}", to_hex(bytes))
}

fn sched() -> Sched {
    Sched {
        idempotency_key: Some(b"build-1842".to_vec()),
        template: Some("review".parse().unwrap()),
        deadline: Some(DEADLINE),
        device: Some(bob_device()),
        unknown: Unknown::default(),
    }
}

fn sched_hex() -> String {
    format!(
        "a4 014a{key} 0466{template} 051a{DEADLINE:08x} 06{device}",
        key = to_hex(b"build-1842"),
        template = to_hex(b"review"),
        device = id(bob_device().as_bytes()),
    )
}

fn queued() -> QueuedTask {
    QueuedTask {
        task: task_id(),
        queue: queue(),
        sched: sched(),
        epoch: 4,
        enc: [0xe5; 32],
        ct: vec![0xc7; 3],
        unknown: Unknown::default(),
    }
}

fn queued_hex() -> String {
    format!(
        "a6 01{task} 02{queue} 03{sched} 0404 055820{enc} 0643c7c7c7",
        task = id(task_id().as_bytes()),
        queue = id(queue().as_bytes()),
        sched = sched_hex(),
        enc = "e5".repeat(32),
    )
}

/// The code of each queue mode.
const MODES: [(u8, QueueMode); 2] = [(1, QueueMode::AtLeastOnce), (2, QueueMode::AtMostOnce)];

fn task_body(mode: QueueMode) -> TaskBody {
    TaskBody {
        sender: alice_device(),
        channel: channel(),
        queue: queue(),
        task: task_id(),
        sched: Sched::default(),
        mode,
        enqueued: ENQUEUED,
        content: content(alice(), Body::A2a(a2a(msg(1), "hi"))),
        unknown: Unknown::default(),
    }
}

fn task_body_hex(mode: u8) -> Vec<u8> {
    let content = task_body(QueueMode::AtLeastOnce).content.to_cbor();
    assert!(content.len() < 256, "a one-byte length");
    hex(&format!(
        "a8 01{sender} 02{channel} 03{queue} 04{task} 05a0 06{mode:02x} 071a{ENQUEUED:08x}
         0858{length:02x}{content}",
        sender = id(alice_device().as_bytes()),
        channel = id(channel().as_bytes()),
        queue = id(queue().as_bytes()),
        task = id(task_id().as_bytes()),
        length = content.len(),
        content = to_hex(&content),
    ))
}

fn result(outcome: Outcome, generation: u64) -> TaskResult {
    TaskResult {
        task: task_id(),
        claim_id: claim_id(),
        generation,
        outcome,
        response: "{}".to_owned(),
        unknown: Unknown::default(),
    }
}

fn result_hex(outcome: u8, generation: u8) -> Vec<u8> {
    hex(&format!(
        "a5 01{task} 02{claim} 03{generation:02x} 04{outcome:02x} 05627b7d",
        task = id(task_id().as_bytes()),
        claim = id(claim_id().as_bytes()),
    ))
}

fn claim(mode: QueueMode) -> Claim {
    Claim {
        claim_id: claim_id(),
        generation: 5,
        attempt: 2,
        lease: 300,
        mode,
        uploader: alice_device(),
        task: queued(),
        unknown: Unknown::default(),
    }
}

fn claim_hex(generation: u8, attempt: u8, mode: u8) -> Vec<u8> {
    hex(&format!(
        "a7 01{claim} 02{generation:02x} 03{attempt:02x} 0419012c 05{mode:02x} 06{uploader}
         07{queued}",
        claim = id(claim_id().as_bytes()),
        uploader = id(alice_device().as_bytes()),
        queued = queued_hex(),
    ))
}

fn queue_key_body() -> QueueKeyBody {
    QueueKeyBody {
        channel: channel(),
        epoch: 3,
        key: [0x9b; 32],
        signer: server_device(),
        unknown: Unknown::default(),
    }
}

fn queue_key_body_hex(epoch: u8) -> Vec<u8> {
    hex(&format!(
        "a4 01{channel} 02{epoch:02x} 035820{key} 04{signer}",
        channel = id(channel().as_bytes()),
        key = "9b".repeat(32),
        signer = id(server_device().as_bytes()),
    ))
}

#[test]
fn a_sched_is_its_map_of_optional_fields() {
    assert_eq!(sched().to_cbor(), hex(&sched_hex()));
    assert_eq!(Sched::from_cbor(&hex(&sched_hex())), Ok(sched()));
    assert_eq!(Sched::default().to_cbor(), hex("a0"));
    assert_eq!(Sched::from_cbor(&hex("a0")), Ok(Sched::default()));
}

/// The encoded `sched` of an idempotency key of `len` bytes.
fn key_hex(len: usize) -> Vec<u8> {
    let head = if len < 24 {
        format!("{:02x}", 0x40 + len)
    } else {
        format!("58{len:02x}")
    };
    hex(&format!("a1 01{head}{}", "6b".repeat(len)))
}

#[test]
fn an_idempotency_key_is_1_to_32_bytes() {
    for len in [1, 32] {
        let sched = Sched::from_cbor(&key_hex(len)).unwrap();
        assert_eq!(sched.idempotency_key, Some(vec![0x6b; len]));
    }
    for len in [0, 33] {
        assert_field_problem(
            Sched::from_cbor(&key_hex(len)),
            &FieldProblem::Size {
                min: 1,
                max: 32,
                actual: len,
            },
        );
    }
}

#[test]
fn a_queued_task_is_its_map_from_epoch_one() {
    assert_eq!(queued().to_cbor(), hex(&queued_hex()));
    assert_eq!(QueuedTask::from_cbor(&hex(&queued_hex())), Ok(queued()));
    let epoch_0 = queued_hex().replace(" 0404 ", " 0400 ");
    assert_field_problem(
        QueuedTask::from_cbor(&hex(&epoch_0)),
        &FieldProblem::Below { min: 1, actual: 0 },
    );
}

#[test]
fn a_task_body_is_its_map_with_each_mode_as_its_code_signed_over_skald_task() {
    assert_eq!(TASK_LABEL, "skald task");
    assert_eq!(<TaskBody as SignedBody>::LABEL, TASK_LABEL);
    for (code, mode) in MODES {
        assert_eq!(task_body(mode).to_cbor(), task_body_hex(code), "{code}");
        assert_eq!(
            TaskBody::from_cbor(&task_body_hex(code)),
            Ok(task_body(mode)),
            "{code}"
        );
    }
    assert_field_problem(
        TaskBody::from_cbor(&task_body_hex(3)),
        &FieldProblem::Value(3),
    );
}

#[test]
fn a_task_is_its_request() {
    let task = Task {
        request: "{}".to_owned(),
        unknown: Unknown::default(),
    };
    assert_eq!(task.to_cbor(), hex("a1 01627b7d"));
    assert_eq!(Task::from_cbor(&hex("a1 01627b7d")), Ok(task));
}

#[test]
fn task_content_holds_the_task_as_its_body_and_no_counter() {
    let task = Task {
        request: "{}".to_owned(),
        unknown: Unknown::default(),
    };

    let content = task.content(alice(), None, 4);

    assert_eq!(
        (content.counter, &content.body, content.hops),
        (None, &None, 4)
    );
    let bytes = content.to_cbor();
    assert_eq!(
        bytes,
        hex(&format!(
            "a3 01{} 0404 0ba1 01627b7d",
            to_hex(&cbor_text(&alice().to_string()))
        ))
    );
    let decoded = Content::from_cbor(&bytes).expect("task content decodes");
    assert_eq!(decoded, content);
    assert_eq!(decoded.unknown.get(11), Some(task.to_cbor().as_slice()));
}

/// `text` as a CBOR text string of under 256 bytes.
fn cbor_text(text: &str) -> Vec<u8> {
    let length = u8::try_from(text.len()).expect("under 256 bytes");
    let mut out = match length {
        0..24 => vec![0x60 + length],
        _ => vec![0x78, length],
    };
    out.extend_from_slice(text.as_bytes());
    out
}

#[test]
fn a_task_result_is_its_map_with_each_outcome_as_its_code_from_generation_one() {
    for (code, outcome) in [(1, Outcome::Done), (2, Outcome::Failed)] {
        assert_eq!(result(outcome, 1).to_cbor(), result_hex(code, 1), "{code}");
        assert_eq!(
            TaskResult::from_cbor(&result_hex(code, 1)),
            Ok(result(outcome, 1)),
            "{code}"
        );
    }
    assert_field_problem(
        TaskResult::from_cbor(&result_hex(3, 1)),
        &FieldProblem::Value(3),
    );
    assert_field_problem(
        TaskResult::from_cbor(&result_hex(1, 0)),
        &FieldProblem::Below { min: 1, actual: 0 },
    );
}

#[test]
fn a_claim_is_its_map_with_each_mode_from_generation_and_attempt_one() {
    for (code, mode) in MODES {
        assert_eq!(claim(mode).to_cbor(), claim_hex(5, 2, code), "{code}");
        assert_eq!(
            Claim::from_cbor(&claim_hex(5, 2, code)),
            Ok(claim(mode)),
            "{code}"
        );
    }
    for encoded in [claim_hex(0, 2, 1), claim_hex(5, 0, 1)] {
        assert_field_problem(
            Claim::from_cbor(&encoded),
            &FieldProblem::Below { min: 1, actual: 0 },
        );
    }
    assert_field_problem(
        Claim::from_cbor(&claim_hex(5, 2, 0)),
        &FieldProblem::Value(0),
    );
}

#[test]
fn a_queue_key_body_is_its_map_from_epoch_one_signed_over_skald_queue_key() {
    assert_eq!(QUEUE_KEY_LABEL, "skald queue key");
    assert_eq!(<QueueKeyBody as SignedBody>::LABEL, QUEUE_KEY_LABEL);
    assert_eq!(queue_key_body().to_cbor(), queue_key_body_hex(3));
    assert_eq!(
        QueueKeyBody::from_cbor(&queue_key_body_hex(3)),
        Ok(queue_key_body())
    );
    assert_field_problem(
        QueueKeyBody::from_cbor(&queue_key_body_hex(0)),
        &FieldProblem::Below { min: 1, actual: 0 },
    );
}

#[test]
fn the_associated_data_of_a_task_is_its_channel_queue_task_and_epoch() {
    assert_eq!(
        QueuedTask::associated_data(channel(), queue(), task_id(), 24),
        hex(&format!(
            "84 {} {} {} 1818",
            id(channel().as_bytes()),
            id(queue().as_bytes()),
            id(task_id().as_bytes()),
        ))
    );
}
