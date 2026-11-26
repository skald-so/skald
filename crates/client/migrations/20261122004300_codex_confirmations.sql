-- Read confirmations of Codex sessions ([`crate::codex_confirm`]): of each message that waits for
-- an agent, the last path its signal went by, when its signal or `inbox` last gave it, in Unix
-- milliseconds, and whether it waits for its session's confirmation since `inbox` gave it.
ALTER TABLE handover ADD COLUMN went_by INTEGER NOT NULL DEFAULT 0;
ALTER TABLE handover ADD COLUMN since INTEGER;
ALTER TABLE handover ADD COLUMN confirming INTEGER NOT NULL DEFAULT 0 CHECK (confirming IN (0, 1));

-- A failed queue may restore only its own signal and only the time it wrote.
ALTER TABLE handover ADD COLUMN queue_attempt BLOB;
ALTER TABLE handover ADD COLUMN queue_owns_since INTEGER NOT NULL DEFAULT 0
    CHECK (queue_owns_since IN (0, 1));
ALTER TABLE codex_session ADD COLUMN queue_attempt BLOB;
