-- Delivering mailboxes (docs/protocol/delivery.md, "Mailbox" and "Receipts"): status entries, and
-- what a status needs of the envelope it reports on.

-- The send an envelope came in: its sending device, its msg_id and whether it gets statuses.
ALTER TABLE envelopes
    ADD COLUMN sender_id uuid REFERENCES devices (id) ON DELETE SET NULL,
    ADD COLUMN msg_id uuid,
    ADD COLUMN report boolean NOT NULL DEFAULT false;

-- A mailbox entry is a message, with its envelope, or a status: a server status (delivered 1,
-- expired 2) of the message msg_id to the device recipient_id. added_at is when it was added.
ALTER TABLE entries
    ALTER COLUMN envelope_id DROP NOT NULL,
    ADD COLUMN status smallint CHECK (status IN (1, 2)),
    ADD COLUMN msg_id uuid,
    ADD COLUMN recipient_id uuid,
    ADD COLUMN added_at timestamptz NOT NULL DEFAULT now(),
    ADD CHECK (
        num_nonnulls(envelope_id, status) = 1
        AND num_nonnulls(status, msg_id, recipient_id) IN (0, 3)
    );

-- Each mailbox entry with its envelope's bytes, NULL for a status.
CREATE OR REPLACE VIEW mailbox_entries AS
SELECT e.device_id, e.seq, v.envelope FROM entries e LEFT JOIN envelopes v ON v.id = e.envelope_id;
