-- The accounts the person verified by fingerprint ([`crate::fingerprints`]), each with the
-- master key it was verified for.
CREATE TABLE verified_account (
    account TEXT NOT NULL,
    master BLOB NOT NULL CHECK (length(master) = 32),
    PRIMARY KEY (account, master)
) STRICT
