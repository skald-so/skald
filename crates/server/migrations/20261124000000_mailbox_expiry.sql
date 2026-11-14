-- Expiring mailbox entries (docs/protocol/delivery.md, "Mailbox"): when a message's ttl ends, and
-- when a status happened, which an expired message's status dates to its expiry.
ALTER TABLE entries
    ADD COLUMN ttl_ends_at timestamptz,
    ADD COLUMN happened_at timestamptz;
UPDATE entries SET happened_at = added_at WHERE status IS NOT NULL;
ALTER TABLE entries
    ADD CHECK ((status IS NULL) = (happened_at IS NULL)),
    ADD CHECK (ttl_ends_at IS NULL OR status IS NULL);

-- Finding expired entries and idempotency keys past the retention.
CREATE INDEX entries_added_at ON entries (added_at);
CREATE INDEX entries_ttl_ends_at ON entries (ttl_ends_at) WHERE ttl_ends_at IS NOT NULL;
CREATE INDEX sends_answered_at ON sends (answered_at);
