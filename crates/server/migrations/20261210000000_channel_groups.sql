-- Channels' MLS groups, which the server follows from outside as their Delivery Service
-- (docs/protocol/membership.md, "Creating a channel", "Commit ordering" and "MLS profile").

-- A channel's group: the device whose Commit of epoch 0 the server kept; the current epoch; the view
-- of the group the server follows, from the first GroupInfo on, NULL before it; the view's member
-- devices in leaf order; and the GroupInfo of the current epoch, NULL until a member uploads one.
-- The row lock stores the channel's mls envelopes one at a time, in one order across mailboxes.
CREATE TABLE channel_groups (
    channel_id uuid PRIMARY KEY REFERENCES channels (id) ON DELETE CASCADE,
    creator_id uuid NOT NULL,
    epoch bigint NOT NULL CHECK (epoch >= 1),
    view bytea,
    members uuid[] NOT NULL DEFAULT '{}',
    group_info bytea,
    CHECK (view IS NOT NULL OR group_info IS NULL)
);

-- A message entry of a handshake message, which does not expire by time (docs/protocol/delivery.md,
-- "Mailbox").
ALTER TABLE entries
    ADD COLUMN lasting boolean NOT NULL DEFAULT false,
    ADD CHECK (NOT lasting OR ttl_ends_at IS NULL);
