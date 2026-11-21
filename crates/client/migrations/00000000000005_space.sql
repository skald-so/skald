-- The spaces the device accepted a revision of: each one's ID and name, its anchor (revision 1
-- of a space it created) and the last revision it accepted, as their CBOR bytes, and the
-- revision it signed that the server has not taken yet, if any.
CREATE TABLE space (
    space TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    anchor BLOB NOT NULL,
    accepted BLOB NOT NULL,
    pending BLOB
) STRICT
