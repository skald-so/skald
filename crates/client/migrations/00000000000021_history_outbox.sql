-- The history entries of the device's channel messages that the server has not answered yet,
-- in the order they go out, by message ID, with their space and channel and the entry's bytes.
-- One whose message's envelope is in the outbox waits for the server to store it.
CREATE TABLE history_outbox (
    position INTEGER PRIMARY KEY,
    msg_id TEXT NOT NULL UNIQUE,
    space TEXT NOT NULL,
    channel TEXT NOT NULL,
    entry BLOB NOT NULL
) STRICT
