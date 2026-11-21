-- The epoch of each channel's group the device is in, with when it entered it, by the clock of
-- epochs' ages ([`crate::clock`]).
CREATE TABLE channel_epoch (
    channel TEXT PRIMARY KEY,
    epoch INTEGER NOT NULL CHECK (epoch >= 0),
    entered INTEGER NOT NULL CHECK (entered >= 0)
) STRICT
