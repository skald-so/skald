//! Grants built from their spec fields, and how the client under test judges them.

use skald_crypto::{GrantError, check_grant};
use skald_proto::{Cbor, ChannelId, DeviceId, Grant, Revocation};

use crate::cbor::{Value, bytes};
use crate::fixtures::{
    ALICE, ALICE_LAPTOP, ALICE_MASTER, BOB_CI, CHANNEL, DAY, Rev, TIMESTAMP, certs, current,
    device, public, signature,
};

/// When the client checks a grant: a minute after the revisions' timestamp.
pub const NOW: u64 = TIMESTAMP + 60;

/// A grant's body, field by field.
#[derive(Debug, Clone)]
pub struct Draft {
    pub service: DeviceId,
    pub channels: Vec<ChannelId>,
    /// The seed of the `signer` device.
    pub signer: u8,
    pub expires: u64,
    /// Fields of later versions.
    pub extra: Vec<(i64, Value)>,
}

/// The grant alice's laptop signs for `ci/deploy-bot`, bob's `service` device, to send in
/// [`CHANNEL`] for a day.
pub fn valid() -> Draft {
    Draft {
        service: device(BOB_CI),
        channels: vec![CHANNEL],
        signer: ALICE_LAPTOP,
        expires: NOW + DAY,
        extra: Vec::new(),
    }
}

/// [`valid`], signed by the device `seed`, which is its `signer`.
pub fn signed_by(seed: u8) -> Grant {
    Draft {
        signer: seed,
        ..valid()
    }
    .grant()
}

impl Draft {
    /// The encoded `grant-body`.
    pub fn body(&self) -> Vec<u8> {
        let channels = self.channels.iter().map(|id| bytes(id.as_bytes()));
        let mut fields = vec![
            (1, bytes(self.service.as_bytes())),
            (2, Value::Array(channels.collect())),
            (3, bytes(device(self.signer).as_bytes())),
            (4, Value::Uint(self.expires)),
        ];
        fields.extend(self.extra.iter().cloned());
        Value::Map(fields).encode()
    }

    /// The grant, its body signed by the key of the device `seed` with `label`, whatever its
    /// `signer` says.
    pub fn signed_as(&self, seed: u8, label: &str) -> Grant {
        let body = self.body();
        let signature = signature(seed, label, &body);
        let encoded = Value::Array(vec![bytes(&body), bytes(&signature)]).encode();
        Grant::from_cbor(&encoded).expect("a grant the spec's CDDL allows")
    }

    /// The grant, signed by the device `seed` over `["skald grant", body]`.
    pub fn signed_with(&self, seed: u8) -> Grant {
        self.signed_as(seed, "skald grant")
    }

    /// The grant, signed by its `signer`.
    pub fn grant(&self) -> Grant {
        self.signed_with(self.signer)
    }
}

/// How the client of alice's account, alice's master key pinned, judges `grant` in content from the
/// device `service` in `channel`, with `latest` the latest revision it accepted, at `now`.
pub fn judged_in(
    grant: &Grant,
    (service, channel): (DeviceId, ChannelId),
    latest: &Rev,
    now: u64,
) -> Result<(), GrantError> {
    judged_revoking(grant, (service, channel), latest, &[], now)
}

/// [`judged_in`], with `revocations` the revocations the server returns.
pub fn judged_revoking(
    grant: &Grant,
    (service, channel): (DeviceId, ChannelId),
    latest: &Rev,
    revocations: &[Revocation],
    now: u64,
) -> Result<(), GrantError> {
    let certs = certs();
    check_grant(
        grant,
        (service, channel),
        &latest.roster(),
        &certs,
        revocations,
        (ALICE, &public(ALICE_MASTER)),
        now,
    )
}

/// [`judged_in`] content from `ci/deploy-bot` in [`CHANNEL`], with [`current`] the latest revision,
/// at [`NOW`].
pub fn judged(grant: &Grant) -> Result<(), GrantError> {
    judged_in(grant, (device(BOB_CI), CHANNEL), &current(), NOW)
}
