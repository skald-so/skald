-- Gap detection: per channel, sending device and scheme (1 `mls`, 2 `hpke`), the counters the
-- device accepted, as `gaps::Counters` encodes them, with the `seq` of the entry it last
-- accepted; and, for each join of a channel's group, the `seq` of the Welcome's entry, or,
-- joining by itself, the `seq` after its cursor.
-- From now on `received` holds only messages without `counter`.
CREATE TABLE received_counter (
    channel TEXT NOT NULL,
    sender TEXT NOT NULL,
    scheme INTEGER NOT NULL CHECK (scheme IN (1, 2)),
    counters BLOB NOT NULL,
    seq INTEGER NOT NULL CHECK (seq >= 1),
    PRIMARY KEY (channel, sender, scheme)
) STRICT;
CREATE TABLE channel_joined (
    channel TEXT NOT NULL,
    seq INTEGER NOT NULL CHECK (seq >= 1),
    PRIMARY KEY (channel, seq)
) STRICT;
