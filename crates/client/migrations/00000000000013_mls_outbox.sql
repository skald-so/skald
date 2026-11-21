-- The device's own Commits and Welcomes the server has not answered yet, in the order they go
-- out, by message ID, with their space and channel and the envelope's bytes.
CREATE TABLE mls_outbox (
    position INTEGER PRIMARY KEY,
    msg_id TEXT NOT NULL UNIQUE,
    space TEXT NOT NULL,
    channel TEXT NOT NULL,
    envelope BLOB NOT NULL
) STRICT
