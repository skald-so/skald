-- Inbound modes (`docs/protocol/delivery.md`, "Device to agent"): the mode of each agent of
-- the device in each space, `accept` without a row; the senders on its allowlist, each a full
-- address, `owner/agent` or a member's handle, lowercased; the reason of each receipt the device
-- owes that is a `refused`, none for a `read`; and, of each own message that waits for the
-- server, the agents of this device that refuse it, as a JSON array, and its `to` agent's
-- `refused`, as JSON, if that agent refuses it.
CREATE TABLE inbound_mode (
    space TEXT NOT NULL,
    agent TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('accept', 'hold', 'refuse')),
    PRIMARY KEY (space, agent)
) STRICT;
CREATE TABLE allowed_sender (
    space TEXT NOT NULL,
    agent TEXT NOT NULL,
    sender TEXT NOT NULL,
    PRIMARY KEY (space, agent, sender)
) STRICT;
ALTER TABLE read_receipt ADD COLUMN reason TEXT;
ALTER TABLE own_message ADD COLUMN refusing TEXT;
ALTER TABLE own_message ADD COLUMN refused TEXT;
