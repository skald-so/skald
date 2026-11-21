-- Where the device's messages are (`skald message-status`): for each channel message it sent,
-- how long it keeps their statuses after the server stored it, in seconds, when the device took
-- the server's `stored`, none before, and when its `to` agent's `held`, `read`, `replied` and
-- `refused`, with that `refused`'s reason, came; and each device the message went to, by its
-- envelope or a copy, with its certificate's `name`, its account and that account's handle in
-- the space, if known, and its status, `stored`, `delivered` or `expired`, with that status's
-- time, none before the server stored it.
CREATE TABLE sent_status (
    msg_id TEXT PRIMARY KEY,
    wait INTEGER NOT NULL CHECK (wait >= 0),
    stored INTEGER,
    held INTEGER,
    read INTEGER,
    replied INTEGER,
    refused INTEGER,
    reason TEXT
) STRICT;
CREATE TABLE sent_recipient (
    msg_id TEXT NOT NULL,
    device TEXT NOT NULL,
    copy INTEGER NOT NULL CHECK (copy IN (0, 1)),
    name TEXT,
    account TEXT,
    handle TEXT,
    status TEXT,
    timestamp INTEGER,
    PRIMARY KEY (msg_id, device)
) STRICT;
