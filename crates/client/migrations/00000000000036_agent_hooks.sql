-- Agents' hooks (`crate::hook`): the line of each message the device can reply to, sealed as
-- its kept line, so an agent reads it whole again by its reference; whether the hook gave the
-- signal of each message below `own` that waits for an agent; and the `Stop` continuations
-- in a row of each agent's session.
ALTER TABLE reply_target ADD COLUMN line BLOB;
ALTER TABLE handover ADD COLUMN signalled INTEGER NOT NULL DEFAULT 0
    CHECK (signalled IN (0, 1));
ALTER TABLE agent ADD COLUMN stops INTEGER NOT NULL DEFAULT 0 CHECK (stops >= 0);
