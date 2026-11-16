-- Channels' history: links, wraps and entries as member devices upload them, and the member
-- devices of each epoch the server followed (docs/protocol/membership.md, "Uploads" and "History
-- entry").

-- The member devices of each epoch of a channel's group, from the first GroupInfo on.
CREATE TABLE channel_epochs (
    channel_id uuid NOT NULL REFERENCES channel_groups (channel_id) ON DELETE CASCADE,
    epoch bigint NOT NULL CHECK (epoch >= 1),
    members uuid[] NOT NULL,
    PRIMARY KEY (channel_id, epoch)
);

INSERT INTO channel_epochs (channel_id, epoch, members)
SELECT channel_id, epoch, members FROM channel_groups WHERE view IS NOT NULL;

-- The first link per channel, epoch and prev.
CREATE TABLE history_links (
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    epoch bigint NOT NULL,
    prev bigint NOT NULL,
    link bytea NOT NULL,
    PRIMARY KEY (channel_id, epoch, prev)
);

-- The first wrap per channel, epoch and recipient: a browser (1) or an account's recovery key (2).
CREATE TABLE history_wraps (
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    epoch bigint NOT NULL,
    recipient_kind smallint NOT NULL CHECK (recipient_kind IN (1, 2)),
    recipient_id uuid NOT NULL,
    wrap bytea NOT NULL,
    PRIMARY KEY (channel_id, epoch, recipient_kind, recipient_id)
);

-- The first entry per sending device and msg_id.
CREATE TABLE history_entries (
    sender_id uuid NOT NULL,
    msg_id uuid NOT NULL,
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    entry bytea NOT NULL,
    PRIMARY KEY (sender_id, msg_id)
);

CREATE INDEX history_entries_channel ON history_entries (channel_id);
