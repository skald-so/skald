-- Revocations: each one the device holds as counting, as its CBOR bytes, whether the server
-- gave it, a Commit carried it or the device signed it; one the device signed is `pending` until
-- the server takes it. And whether the device last held each device it checked revoked, so it
-- warns when that changes.
CREATE TABLE revocation (
    revocation BLOB PRIMARY KEY,
    pending INTEGER NOT NULL DEFAULT 0 CHECK (pending IN (0, 1))
) STRICT;
CREATE TABLE device_status (
    device TEXT PRIMARY KEY,
    revoked INTEGER NOT NULL CHECK (revoked IN (0, 1))
) STRICT;
