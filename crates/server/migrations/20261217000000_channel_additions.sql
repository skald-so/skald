-- The channel's epoch when a revision last added an account to the channel's members
-- (docs/protocol/membership.md, "Access"). The account's join is its first epoch after it; an
-- account without a row has been listed since before the group.
CREATE TABLE channel_additions (
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    account_id uuid NOT NULL,
    epoch bigint NOT NULL CHECK (epoch >= 0),
    PRIMARY KEY (channel_id, account_id)
);
