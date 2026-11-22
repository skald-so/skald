-- The epochs of the channels whose queue key object the device does not read again: those it
-- found on the server, and those whose upload the server refused ([`crate::queue_keys`]).
CREATE TABLE settled_queue_key (
    channel TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK (epoch >= 1),
    PRIMARY KEY (channel, epoch)
) STRICT;
