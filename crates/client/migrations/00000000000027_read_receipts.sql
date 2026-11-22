-- `read` receipts (`docs/protocol/delivery.md`, "Receipts"): whether each envelope of the
-- outbox goes with `report` true, false for a receipt; the `to` of each message the device
-- sent, if any, and whether it took a `read` of it; and, for each kept message whose `to` is an
-- agent of this device, by its entry's `seq`, what its receipt reports: its `msg_id`, space,
-- channel, sending device, `from` and `to`, with the agent's name, `due` once the message was
-- handed over to the agent, until the receipt goes into the outbox.
ALTER TABLE outbox ADD COLUMN report INTEGER NOT NULL DEFAULT 1 CHECK (report IN (0, 1));
ALTER TABLE sent ADD COLUMN addressee TEXT;
ALTER TABLE sent ADD COLUMN read INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1));
CREATE TABLE read_receipt (
    seq INTEGER PRIMARY KEY CHECK (seq >= 1),
    msg_id TEXT NOT NULL,
    space TEXT NOT NULL,
    channel TEXT NOT NULL,
    sender TEXT NOT NULL,
    author TEXT NOT NULL,
    addressee TEXT NOT NULL,
    agent TEXT NOT NULL,
    due INTEGER NOT NULL DEFAULT 0 CHECK (due IN (0, 1))
) STRICT;
