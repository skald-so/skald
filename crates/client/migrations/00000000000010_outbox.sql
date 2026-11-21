-- The outbox: each envelope the device sealed that the server has not answered yet, in the
-- order it goes out, by its message ID and recipient device, with its space and its bytes; and
-- the last `counter` the device gave content to each recipient device in each channel.
CREATE TABLE outbox (
    position INTEGER PRIMARY KEY,
    msg_id TEXT NOT NULL,
    recipient TEXT NOT NULL,
    space TEXT NOT NULL,
    envelope BLOB NOT NULL,
    UNIQUE (msg_id, recipient)
) STRICT;
CREATE TABLE send_counter (
    channel TEXT NOT NULL,
    recipient TEXT NOT NULL,
    last INTEGER NOT NULL CHECK (last >= 1),
    PRIMARY KEY (channel, recipient)
) STRICT;
