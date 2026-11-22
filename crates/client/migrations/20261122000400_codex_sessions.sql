-- Codex sessions ([`crate::codex`]): each session a hook registered, by its thread, with its
-- agent, the agent's space and folder, its session file and config home, if known, the Codex
-- process that runs it, if known, whether a turn of it runs, and the place of its last hook's
-- run in the order of hooks; and, of each kept message that waits for an agent, the session
-- whose hooks give its signal.
CREATE TABLE codex_session (
    thread TEXT PRIMARY KEY,
    space TEXT NOT NULL,
    agent TEXT NOT NULL,
    folder TEXT NOT NULL,
    file TEXT,
    home TEXT,
    process INTEGER CHECK (process >= 0),
    busy INTEGER NOT NULL DEFAULT 0 CHECK (busy IN (0, 1)),
    hooked INTEGER NOT NULL CHECK (hooked >= 1)
) STRICT;
ALTER TABLE handover ADD COLUMN thread TEXT;
