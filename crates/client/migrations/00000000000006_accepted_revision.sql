-- The hash of each revision the device accepted of a space, by its number, but that of the
-- space's last accepted revision, which the space holds: it is recorded once the device
-- accepts the next one. A state older than this table accepted only revision 1 of each space.
CREATE TABLE accepted_revision (
    space TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    hash BLOB NOT NULL CHECK (length(hash) = 32),
    PRIMARY KEY (space, revision)
) STRICT
