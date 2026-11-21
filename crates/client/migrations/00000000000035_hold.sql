-- `hold` (`docs/protocol/delivery.md`, "Device to agent"): each kept message that waits for the
-- person's decision for an agent, by its entry's `seq`, with when the device took it in, in
-- Unix seconds; of each receipt the device owes, whether its `held` is due and whether it is a
-- `refused`, with or without a reason; whether the device took a `held` of each message it
-- sent; and, of each own message that waits for the server, the agents of this device that
-- hold it, as a JSON array, and its `to` agent's `held`, as JSON, if that agent holds it.
CREATE TABLE held_message (
    seq INTEGER NOT NULL,
    agent TEXT NOT NULL,
    taken INTEGER NOT NULL,
    PRIMARY KEY (seq, agent)
) STRICT;
ALTER TABLE read_receipt ADD COLUMN held_due INTEGER NOT NULL DEFAULT 0
    CHECK (held_due IN (0, 1));
ALTER TABLE read_receipt ADD COLUMN refused INTEGER NOT NULL DEFAULT 0
    CHECK (refused IN (0, 1));
UPDATE read_receipt SET refused = 1 WHERE reason IS NOT NULL;
ALTER TABLE sent ADD COLUMN held INTEGER NOT NULL DEFAULT 0 CHECK (held IN (0, 1));
ALTER TABLE own_message ADD COLUMN holding TEXT;
ALTER TABLE own_message ADD COLUMN held TEXT;
