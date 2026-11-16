-- The latest roster revision of each space that a device's long-poll answers have covered: a
-- long-poll answers a `space-notice` of each space with a later revision that lists the device's
-- account (docs/protocol/delivery.md, "Mailbox").
CREATE TABLE long_poll_revisions (
    device_id uuid NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
    space_id uuid NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    revision bigint NOT NULL,
    PRIMARY KEY (device_id, space_id)
);
CREATE INDEX long_poll_revisions_space_id ON long_poll_revisions (space_id);
