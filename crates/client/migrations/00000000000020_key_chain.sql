-- The key chain: whether the device judged each epoch of a channel it entered clean, and the
-- links, by epoch and `prev`, it does not upload again: those the server took from it, listed
-- to it or refused.
CREATE TABLE epoch_judgment (
    channel TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    clean INTEGER NOT NULL CHECK (clean IN (0, 1)),
    PRIMARY KEY (channel, epoch)
) STRICT;
CREATE TABLE settled_link (
    channel TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    prev INTEGER NOT NULL CHECK (prev >= 0),
    PRIMARY KEY (channel, epoch, prev)
) STRICT;
