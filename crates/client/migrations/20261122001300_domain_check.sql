-- The DNS root of trust of spaces ([`crate::domains`]), by the resolver's URL: the TXT records of a
-- domain's `_skald` name it gave, each on a line, until they expire, in seconds since the Unix
-- epoch; and each key change of an account they showed that the device warned of, by the master
-- key the roster gave and the fingerprint they listed, or '' for an admin they did not list.
CREATE TABLE domain_records (
    resolver TEXT NOT NULL,
    domain TEXT NOT NULL,
    records TEXT NOT NULL,
    expires INTEGER NOT NULL,
    PRIMARY KEY (resolver, domain)
) STRICT;
CREATE TABLE domain_warning (
    resolver TEXT NOT NULL,
    domain TEXT NOT NULL,
    account TEXT NOT NULL,
    master BLOB NOT NULL CHECK (length(master) = 32),
    listed TEXT NOT NULL,
    PRIMARY KEY (resolver, domain, account, master, listed)
) STRICT
