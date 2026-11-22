-- The first queue key object per channel and epoch, as a member device of the epoch uploaded it
-- (docs/protocol/queues.md, "Queue key object").
CREATE TABLE queue_keys (
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    epoch bigint NOT NULL CHECK (epoch >= 1),
    object bytea NOT NULL,
    PRIMARY KEY (channel_id, epoch)
);
