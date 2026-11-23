-- Resending (`crate::resend`): the path that last took each kept message that waits for an agent,
-- `socket`, `channel`, `rewake` or `mod`, or `hooks` once the paths are done, none before the
-- first; until when, in Unix milliseconds, it waits for its confirmation there; and when the first
-- path took it.
ALTER TABLE handover ADD COLUMN path TEXT;
ALTER TABLE handover ADD COLUMN due INTEGER;
ALTER TABLE handover ADD COLUMN given INTEGER;
