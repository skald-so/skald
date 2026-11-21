-- The device's mailbox: its cursor, the highest `seq` it wrote; each entry it wrote that no run
-- has shown yet, by `seq`, as its `message` or `status` frame; the sending device and `msg_id`
-- of each message it showed; and the space and channel of each message it sent, which its
-- statuses report on.
ALTER TABLE device ADD COLUMN mailbox_cursor INTEGER NOT NULL DEFAULT 0
    CHECK (mailbox_cursor >= 0);
CREATE TABLE inbox (
    seq INTEGER PRIMARY KEY CHECK (seq >= 1),
    entry BLOB NOT NULL
) STRICT;
CREATE TABLE received (
    sender TEXT NOT NULL,
    msg_id TEXT NOT NULL,
    PRIMARY KEY (sender, msg_id)
) STRICT;
CREATE TABLE sent (
    msg_id TEXT PRIMARY KEY,
    space TEXT NOT NULL,
    channel TEXT NOT NULL
) STRICT;
