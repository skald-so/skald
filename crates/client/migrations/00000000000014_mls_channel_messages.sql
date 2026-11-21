-- Channel messages under `mls`: the outbox holds them without a recipient device; the last
-- `counter` the device gave its content in each channel, in place of one per recipient device;
-- and what decrypting each `mls` application message of the inbox in its channel's group gave,
-- sealed under the state key, until a run shows it.
CREATE TABLE outbox_next (
    position INTEGER PRIMARY KEY,
    msg_id TEXT NOT NULL,
    recipient TEXT,
    space TEXT NOT NULL,
    envelope BLOB NOT NULL,
    UNIQUE (msg_id, recipient)
) STRICT;
INSERT INTO outbox_next (position, msg_id, recipient, space, envelope)
    SELECT position, msg_id, recipient, space, envelope FROM outbox;
DROP TABLE outbox;
ALTER TABLE outbox_next RENAME TO outbox;
DROP TABLE send_counter;
CREATE TABLE channel_counter (
    channel TEXT PRIMARY KEY,
    last INTEGER NOT NULL CHECK (last >= 1)
) STRICT;
ALTER TABLE inbox ADD COLUMN opened BLOB;
