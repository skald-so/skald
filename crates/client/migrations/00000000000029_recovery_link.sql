-- A device waiting for linking that links with the recovery key: the master public key it
-- pinned and the certificate it issued with the master key, in CBOR, until it is linked. At
-- most one row.
CREATE TABLE recovery_link (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    master BLOB NOT NULL CHECK (length(master) = 32),
    cert BLOB NOT NULL
) STRICT
