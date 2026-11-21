-- The last anchors object the device signed, as its CBOR bytes, and whether it waits for the
-- server to take it. At most one row. A state older than this table has none, so the device
-- signs one at its next run.
CREATE TABLE anchors (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    anchors BLOB NOT NULL,
    pending INTEGER NOT NULL CHECK (pending IN (0, 1))
) STRICT
