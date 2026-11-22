-- The wraps of the channels' history keys the device does not upload again: those the server
-- took from it, listed to it or refused, by channel, epoch and recipient, a `browser` device
-- (1) or an account's recovery key (2).
CREATE TABLE settled_wrap (
    channel TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 1),
    kind INTEGER NOT NULL CHECK (kind IN (1, 2)),
    recipient TEXT NOT NULL,
    PRIMARY KEY (channel, epoch, kind, recipient)
) STRICT
