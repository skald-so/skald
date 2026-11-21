-- Copies of the device's channel messages under `hpke` to `service` devices outside the
-- channel's group: the last `counter` the device gave content to each such device in each
-- channel. The outbox holds a copy with its recipient device.
CREATE TABLE copy_counter (
    channel TEXT NOT NULL,
    recipient TEXT NOT NULL,
    last INTEGER NOT NULL CHECK (last >= 1),
    PRIMARY KEY (channel, recipient)
) STRICT
