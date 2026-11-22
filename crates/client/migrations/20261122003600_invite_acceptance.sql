-- The acceptance of each invite the device accepted that no revision has admitted its account by
-- yet, with the space it follows meanwhile, and whether the server took it ([`crate::acceptances`]).
CREATE TABLE invite_acceptance (
    invite TEXT PRIMARY KEY,
    space TEXT NOT NULL,
    acceptance BLOB NOT NULL,
    taken INTEGER NOT NULL DEFAULT 0 CHECK (taken IN (0, 1))
) STRICT;
