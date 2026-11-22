-- Read confirmations of Codex sessions ([`crate::codex_confirm`]): of each message that waits for
-- an agent, the last path its signal went by, when its signal or `inbox` last gave it, in Unix
-- milliseconds, and whether it waits for its session's confirmation since `inbox` gave it.
ALTER TABLE handover ADD COLUMN went_by INTEGER NOT NULL DEFAULT 0;
ALTER TABLE handover ADD COLUMN since INTEGER;
ALTER TABLE handover ADD COLUMN confirming INTEGER NOT NULL DEFAULT 0 CHECK (confirming IN (0, 1));
