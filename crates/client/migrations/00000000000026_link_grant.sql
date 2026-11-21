-- The grant a device waiting for linking took from a run of its link code: the master public
-- key it pins, its complete certificate and the link object, in CBOR, until the server stores
-- the certificate. At most one row. And, as an issuing device, how many runs of `skald link` in
-- a row failed.
CREATE TABLE link_grant (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    master BLOB NOT NULL CHECK (length(master) = 32),
    cert BLOB NOT NULL,
    link BLOB NOT NULL
) STRICT;
ALTER TABLE device ADD COLUMN failed_link_runs INTEGER NOT NULL DEFAULT 0
    CHECK (failed_link_runs >= 0);
