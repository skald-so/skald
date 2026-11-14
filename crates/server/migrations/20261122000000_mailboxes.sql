-- Mailboxes: the envelopes devices sent, each stored once per idempotency key, and the entries of
-- each device's mailbox (docs/protocol/delivery.md, "Send" and "Mailbox").

-- An envelope as its sending device sent it.
CREATE TABLE envelopes (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    envelope bytea NOT NULL
);

-- A stored send by its idempotency key: its sending device, its msg_id and, under hpke, its
-- recipient device; and when it was committed, which the answer to each retry repeats.
CREATE TABLE sends (
    sender_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    msg_id uuid NOT NULL,
    recipient_id uuid,
    stored_at timestamptz NOT NULL,
    UNIQUE NULLS NOT DISTINCT (sender_id, msg_id, recipient_id)
);

-- A device's mailbox: the seq of its latest entry, which only grows, so no seq is reused. Its row
-- lock orders the entries stored at once.
CREATE TABLE mailboxes (
    device_id uuid PRIMARY KEY REFERENCES devices (id) ON DELETE CASCADE,
    last_seq bigint NOT NULL CHECK (last_seq >= 1)
);

-- A mailbox entry: a message, an envelope for the mailbox's device.
CREATE TABLE entries (
    device_id uuid NOT NULL REFERENCES mailboxes (device_id) ON DELETE CASCADE,
    seq bigint NOT NULL CHECK (seq >= 1),
    envelope_id bigint NOT NULL REFERENCES envelopes (id),
    PRIMARY KEY (device_id, seq)
);
CREATE INDEX entries_envelope_id ON entries (envelope_id);

-- Each mailbox entry with its envelope's bytes.
CREATE VIEW mailbox_entries AS
SELECT e.device_id, e.seq, v.envelope FROM entries e JOIN envelopes v ON v.id = e.envelope_id;
